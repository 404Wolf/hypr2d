#include "doctor.hpp"
#include "external_animation.hpp"
#include "hypr2d-core/src/lib.rs.h"
#include "monitors.hpp"
#include "overview.hpp"
#include "ui.hpp"
#include <algorithm>
#include <chrono>
#include <hyprland/src/Compositor.hpp>
#include <hyprland/src/animation/WorkspaceAnimationController.hpp>
#include <hyprland/src/config/shared/actions/ConfigActions.hpp>
#include <hyprland/src/debug/log/Logger.hpp>
#include <hyprland/src/desktop/state/FocusState.hpp>
#include <hyprland/src/desktop/state/WindowState.hpp>
#include <hyprland/src/desktop/view/window/Window.hpp>
#include <hyprland/src/desktop/view/window/WindowMetadata.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/ipc/s1/S1.hpp>
#include <hyprland/src/ipc/s2/S2.hpp>
#include <hyprland/src/managers/eventLoop/EventLoopManager.hpp>
#include <hyprland/src/managers/eventLoop/EventLoopTimer.hpp>
#include <hyprland/src/managers/input/InputManager.hpp>
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/state/MonitorState.hpp>
#include <hyprland/src/state/workspace/State.hpp>
#include <hyprland/src/workspace/HLWorkspace.hpp>
extern "C" {
#include <lauxlib.h>
#include <lua.h>
}

static std::optional<rust::Box<hypr2d::Core>> core;
static CHyprSignalListener workspaceListener;
static CHyprSignalListener swipeBeginListener, swipeUpdateListener, swipeEndListener, renderListener, monitorListener;
static CHyprSignalListener pinchBeginListener, pinchUpdateListener, pinchEndListener;
static bool testingEnabled = false;
static bool swipeClaimed = false, applyingSwipe = false;
static bool swipeAnimating = false;
static std::vector<PHLWORKSPACE> swipeVisuals;
static PHLMONITOR swipeMonitor;
static SP<CEventLoopTimer> settleTimer;
static bool feedSwipe(hypr2d::SwipePhase, uint32_t, double, double, bool);
static void cancelSwipe();

static std::vector<hypr2d::Monitor> monitors() {
    std::vector<hypr2d::Monitor> result;
    for (const auto &m : State::monitorState()->monitors()) {
        const auto ws = m->m_activeWorkspace;
        const auto off = ws ? ws->m_renderOffset->value() : Vector2D{};
        result.push_back({static_cast<uint32_t>(m->m_id), (*core)->monitor_slot(m->m_name), m->m_name,
                          ws && ws->numberedID() ? *ws->numberedID() : 0, m == Desktop::focusState()->monitor(), off.x, off.y, m->m_size.x,
                          m->m_size.y});
    }
    return result;
}
static std::vector<hypr2d::Window> windows() {
    std::vector<hypr2d::Window> result;
    std::vector<hypr2d::SpecialInfo> specials;
    for (const auto &w : Desktop::windowState()->windows()) {
        if (!w->mapped() || !w->m_workspace || !w->m_monitor)
            continue;
        if (!w->m_workspace->numberedID())
            specials.push_back({w->m_workspace->addressableName(), w->metadata().appID()});
        result.push_back({w->m_workspace->numberedID() ? *w->m_workspace->numberedID() : 0, static_cast<uint32_t>(w->m_monitor->m_id),
                          std::format("0x{:x}", reinterpret_cast<uintptr_t>(w.get())), w->metadata().title(), w->metadata().appID(),
                          bool(w->m_hints & Desktop::View::WINDOW_HINT_URGENT)});
    }
    (*core)->specials_observe({specials.data(), specials.size()});
    return result;
}
static PHLMONITOR monitor(uint32_t id) {
    for (const auto &m : State::monitorState()->monitors())
        if (m->m_id == id)
            return m;
    return nullptr;
}
static PHLWORKSPACE workspace(uint32_t id, PHLMONITOR mon) {
    for (const auto &ws : State::Workspace::state()->workspaces())
        if (ws->numberedID() && *ws->numberedID() == id)
            return ws.lock();
    return State::Workspace::state()->createNumbered(::Workspace::SWorkspaceNumberedID{id}, mon);
}
static void event(const hypr2d::Move &move) {
    IPC::Socket2::sock()->postEvent({"hypr2d", std::format("cell>>{},{},{}", move.monitor, move.x, move.y)});
}
// The shim only applies Rust's offsets; axis, thresholds and spring are in Rust.
static double nowMs() { return std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now().time_since_epoch()).count(); }
static void releaseVisuals() {
    if (settleTimer) {
        settleTimer->cancel();
        g_pEventLoopManager->removeTimer(settleTimer);
        settleTimer.reset();
    }
    for (const auto &w : swipeVisuals) {
        const auto m = w->m_monitor.lock();
        w->m_forceRendering = false;
        w->m_renderOffset->setValueAndWarp(Vector2D{});
        w->m_alpha->setValueAndWarp(m && m->m_activeWorkspace == w ? 1.f : 0.f);
        w->updateWindowDecos();
    }
    swipeVisuals.clear();
    if (swipeMonitor)
        g_pHyprRenderer->damageMonitor(swipeMonitor);
    swipeMonitor.reset();
    swipeAnimating = false;
}
static void cancelSwipe() {
    if (core) {
        auto ms = monitors();
        (*core)->swipe({ms.data(), ms.size()}, {hypr2d::SwipePhase::Cancel, 0, 0, 0, nowMs(), true});
    }
    releaseVisuals();
}
static void applySwipe(const hypr2d::SwipeFrame &frame) {
    if (frame.done) {
        releaseVisuals();
        return;
    }
    if (!frame.active)
        return;
    auto m = monitor(frame.monitor);
    if (!m) {
        cancelSwipe();
        return;
    }
    // Rust supplies the visible quadrant, at most four workspaces.
    std::vector<std::pair<PHLWORKSPACE, hypr2d::SwipeView>> views;
    for (const auto &view : frame.views) {
        auto w = workspace(view.workspace, m);
        if (!w || w->m_monitor != m) {
            cancelSwipe();
            return;
        }
        views.emplace_back(w, view);
    }
    auto target = frame.commit ? workspace(frame.commit, m) : nullptr;
    if (frame.commit && (!target || target->m_monitor != m)) {
        cancelSwipe();
        return;
    }
    swipeMonitor = m;
    if (target) {
        if (std::ranges::find(swipeVisuals, target) == swipeVisuals.end())
            swipeVisuals.push_back(target);
        applyingSwipe = true;
        m->changeWorkspace(target, false, true);
        applyingSwipe = false;
        IPC::Socket2::sock()->postEvent({"hypr2d", std::format("cell>>{},{},{}", frame.monitor, frame.cell_x, frame.cell_y)});
    }
    // A projected snap can reverse direction. Hide its active target until it
    // enters the visible quadrant; render only the four supplied views.
    for (const auto &w : swipeVisuals) {
        if (std::ranges::none_of(views, [&](const auto &v) { return v.first == w; })) {
            w->m_forceRendering = false;
            w->m_renderOffset->setValueAndWarp(Vector2D{});
            w->m_alpha->setValueAndWarp(0.f);
        }
    }
    for (const auto &[w, view] : views) {
        if (std::ranges::find(swipeVisuals, w) == swipeVisuals.end())
            swipeVisuals.push_back(w);
        w->m_forceRendering = true;
        w->m_alpha->setValueAndWarp(1.f);
        w->m_renderOffset->setValueAndWarp(Vector2D{view.x, view.y});
        w->updateWindowDecos();
    }
    swipeAnimating = frame.settling;
    g_pHyprRenderer->damageMonitor(m);
    if (frame.settling)
        m->scheduleFrame(Aquamarine::IOutput::AQ_SCHEDULE_DAMAGE);
    if (frame.finish_after_ms > 0) {
        if (settleTimer) {
            settleTimer->cancel();
            g_pEventLoopManager->removeTimer(settleTimer);
            settleTimer.reset();
        }
        settleTimer = makeShared<CEventLoopTimer>(
            std::chrono::duration_cast<Time::steady_dur>(std::chrono::duration<double, std::milli>{frame.finish_after_ms}),
            [](SP<CEventLoopTimer>, void *) { feedSwipe(hypr2d::SwipePhase::Tick, 0, 0, 0, false); }, nullptr);
        g_pEventLoopManager->addTimer(settleTimer);
    }
}
static bool feedSwipe(hypr2d::SwipePhase phase, uint32_t fingers = 0, double dx = 0, double dy = 0, bool cancelled = false) {
    auto ms = monitors();
    auto frame = (*core)->swipe({ms.data(), ms.size()}, {phase, fingers, dx, dy, nowMs(), cancelled});
    if (phase == hypr2d::SwipePhase::Begin && frame.claimed)
        releaseVisuals();
    applySwipe(frame);
    return frame.claimed;
}
static wl_event_source *extensionSource = nullptr;
static int extensionReady(int, uint32_t, void *) {
    auto ms = monitors();
    auto batch = (*core)->extension_drain({ms.data(), ms.size()});
    for (auto &a : batch.actions) {
        std::string error;
        try {
            error = OverviewUI::place(a);
        } catch (const std::exception &e) {
            error = e.what();
        }
        (*core)->extension_result(a.token, error);
    }
    if (batch.changed) {
        OverviewUI::command("overview refresh");
        IPC::Socket2::sock()->postEvent({"hypr2d", "extensions"});
    }
    return 0;
}
static std::string execute(const std::string &input) {
    if (input.starts_with("adopt ") || input.starts_with("swap-grids ") || input.starts_with("move-grid ")) {
        cancelSwipe();
        return MonitorPolicy::command(input);
    }
    if (input.starts_with("monitor-test "))
        return MonitorPolicy::command(input);
    try {
        if (input == "animation-state" && testingEnabled)
            return ExternalAnimation::state();
        if (input == "doctor")
            return Doctor::report();
        if (input == "doctor repair") {
            cancelSwipe();
            return Doctor::repair();
        }
        if (input.starts_with("doctor-test "))
            return Doctor::test(input);
        if (input == "overview" || input.starts_with("overview ")) {
            cancelSwipe();
            return OverviewUI::command(input);
        }
        if (input.starts_with("translate-grid ")) {
            cancelSwipe();
            return OverviewUI::translate(input.substr(15));
        }
        auto ms = monitors();
        if (input.starts_with("probe "))
            return testingEnabled ? OverviewProbe::command(**core, ms, input) : "error: test-only command";
        if (input.starts_with("grid "))
            MonitorPolicy::observeWorkspaces();
        auto ws = windows();
        auto reply = (*core)->command({ms.data(), ms.size()}, {ws.data(), ws.size()}, input);
        if (!reply.error.empty())
            return "error: " + std::string(reply.error);
        if (!reply.moves.empty() || input.starts_with("configure ") || input.starts_with("swipe-config ") ||
            input.starts_with("swipe-enabled "))
            cancelSwipe();
        const auto focus = Desktop::focusState()->monitor();
        const auto window = Desktop::focusState()->window();
        // Validate all targets before mutating any monitor.
        for (const auto &move : reply.moves) {
            if (move.carry && !Doctor::healthy())
                return "error: layout inconsistent; run hyprctl hypr2d doctor";
            if (move.carry && !window)
                return "error: no active window";
            const auto target = workspace(move.workspace, monitor(move.monitor));
            if (!target || target->m_monitor != monitor(move.monitor))
                return "error: target workspace belongs to another monitor";
        }
        for (const auto &move : reply.moves) {
            auto m = monitor(move.monitor);
            const auto old = m->m_activeWorkspace;
            const auto target = workspace(move.workspace, m);
            if (!move.all)
                (void)Config::Actions::focusMonitor(m);
            if (old == target)
                continue;
            const auto savedStyle = target->m_animationStyle;
            const std::string style = move.vertical ? "slidevert" : "slide";
            target->m_animationStyle = style;
            if (move.carry) {
                auto result = Config::Actions::moveToWorkspace(target, false, window);
                if (!result) {
                    target->m_animationStyle = savedStyle;
                    return "error: carry failed";
                }
            } else
                m->changeWorkspace(target, false, true, move.all && m != focus);
            target->m_animationStyle = savedStyle;
            // Direction follows the grid move even when workspace ids wrap.
            Animation::Workspace::startAnimation(old, Animation::Workspace::ANIMATION_TYPE_OUT, move.positive, false, style);
            Animation::Workspace::startAnimation(target, Animation::Workspace::ANIMATION_TYPE_IN, move.positive, false, style);
            event(move);
        }
        if (!reply.moves.empty() && reply.moves[0].all)
            (void)Config::Actions::focusMonitor(focus);
        if (input.starts_with("overview-config ")) {
            OverviewUI::command("overview refresh");
            IPC::Socket2::sock()->postEvent({"hypr2d", "config"});
        }
        if (input.starts_with("configure "))
            IPC::Socket2::sock()->postEvent({"hypr2d", "config"});
        if (input.starts_with("grid ") && input != "grid list") {
            OverviewUI::command("overview refresh");
            IPC::Socket2::sock()->postEvent({"hypr2d", "grids"});
        }
        return std::string(reply.output);
    } catch (const std::exception &e) {
        return std::string("error: ") + e.what();
    }
}
// Lua dispatcher constructors return a callable, matching hl.dsp.* conventions.
static int invoke(lua_State *L) {
    const auto result = execute(luaL_checkstring(L, 1));
    lua_pushboolean(L, !result.starts_with("error:"));
    lua_pushlstring(L, result.data(), result.size());
    return 2;
}
static int dispatcher(lua_State *L, const std::string &command) {
    // Keep only a Lua closure in binds: no plugin code pointer survives unload.
    luaL_loadstring(L, "local command = ...; return function() return hl.plugin.hypr2d._execute(command) end");
    lua_pushlstring(L, command.data(), command.size());
    lua_call(L, 1, 1);
    return 1;
}
static int moveLua(lua_State *L) {
    std::string cmd = "move " + std::string(luaL_checkstring(L, 1));
    if (lua_istable(L, 2)) {
        lua_getfield(L, 2, "all");
        if (lua_toboolean(L, -1))
            cmd += " all";
        lua_pop(L, 1);
    }
    return dispatcher(L, cmd);
}
static int gridCreateLua(lua_State *L) {
    const std::string name = luaL_checkstring(L, 1);
    int64_t columns = -1, rows = -1;
    int8_t wrap = -1;
    if (lua_istable(L, 2)) {
        lua_getfield(L, 2, "columns");
        if (!lua_isnil(L, -1))
            columns = luaL_checkinteger(L, -1);
        lua_pop(L, 1);
        lua_getfield(L, 2, "rows");
        if (!lua_isnil(L, -1))
            rows = luaL_checkinteger(L, -1);
        lua_pop(L, 1);
        lua_getfield(L, 2, "wrap");
        if (!lua_isnil(L, -1)) {
            luaL_checktype(L, -1, LUA_TBOOLEAN);
            wrap = lua_toboolean(L, -1);
        }
        lua_pop(L, 1);
    } else if (!lua_isnoneornil(L, 2))
        return luaL_error(L, "expected grid options table");
    return dispatcher(L, std::string(hypr2d::grid_create_args(name, columns, rows, wrap)));
}
static int gridRenameLua(lua_State *L) {
    return dispatcher(L, std::string(hypr2d::grid_args("grid rename", luaL_checkstring(L, 1), luaL_checkstring(L, 2))));
}
static int gridDeleteLua(lua_State *L) { return dispatcher(L, std::string(hypr2d::grid_args("grid delete", luaL_checkstring(L, 1), ""))); }
static int adoptLua(lua_State *L) {
    const std::string slot = luaL_checkstring(L, 1);
    const std::string mon = lua_isnoneornil(L, 2) ? "" : luaL_checkstring(L, 2);
    return dispatcher(L, std::string(hypr2d::grid_args("adopt", slot, mon)));
}
static int swapGridsLua(lua_State *L) {
    return dispatcher(L, std::string(hypr2d::grid_args("swap-grids", luaL_checkstring(L, 1), luaL_checkstring(L, 2))));
}
static int translateGridLua(lua_State *L) { return dispatcher(L, "translate-grid " + std::string(luaL_checkstring(L, 1))); }
static int moveGridLua(lua_State *L) { return dispatcher(L, "move-grid " + std::string(luaL_checkstring(L, 1))); }
static int parkedLua(lua_State *L) { return dispatcher(L, "overview parked"); }
static int overviewLua(lua_State *L) { return dispatcher(L, "overview"); }
static int carryLua(lua_State *L) { return dispatcher(L, "carry " + std::string(luaL_checkstring(L, 1))); }
static int gotoLua(lua_State *L) {
    const auto x = luaL_checkinteger(L, 1), y = luaL_checkinteger(L, 2);
    std::string cmd = std::format("goto {} {}", x, y);
    if (!lua_isnoneornil(L, 3))
        cmd += " " + std::string(luaL_checkstring(L, 3));
    return dispatcher(L, cmd);
}
static int configureLua(lua_State *L) {
    const std::string mon = luaL_checkstring(L, 1);
    luaL_checktype(L, 2, LUA_TTABLE);
    auto integer = [&](const char *key, int fallback) {
        lua_getfield(L, 2, key);
        int v = lua_isnil(L, -1) ? fallback : lua_tointeger(L, -1);
        lua_pop(L, 1);
        return v;
    };
    const auto columns = integer("columns", 8), rows = integer("rows", 8), stride = integer("stride", 10);
    lua_getfield(L, 2, "wrap");
    bool wrap = lua_isnil(L, -1) || lua_toboolean(L, -1);
    lua_pop(L, 1);
    const auto result = execute(std::format("configure {} {} {} {} {}", mon, columns, rows, wrap ? "true" : "false", stride));
    lua_pushboolean(L, !result.starts_with("error:"));
    lua_pushlstring(L, result.data(), result.size());
    return 2;
}
static int swipeConfigLua(lua_State *L) {
    luaL_checktype(L, 1, LUA_TTABLE);
    auto number = [&](const char *key, double fallback) {
        lua_getfield(L, 1, key);
        auto n = lua_isnil(L, -1) ? fallback : lua_tonumber(L, -1);
        lua_pop(L, 1);
        return n;
    };
    const auto fingers = number("fingers", 3), distance = number("distance", 120), velocity = number("velocity", 0.8),
               lock = number("axis_lock", 5);
    lua_getfield(L, 1, "natural");
    const bool natural = lua_isnil(L, -1) || lua_toboolean(L, -1);
    lua_pop(L, 1);
    const auto result =
        execute(std::format("swipe-config {} {} {} {} {} {} {} {} {} {} {}", fingers, distance, velocity, natural ? "true" : "false", lock,
                            number("diagonal_width", 15), number("hysteresis", 8), number("hysteresis_distance", 12),
                            number("projection_decel", 0.0006), number("spring_stiffness", 576), number("sensitivity", 1.0)));
    lua_pushboolean(L, !result.starts_with("error:"));
    lua_pushlstring(L, result.data(), result.size());
    return 2;
}
// Enabled only by the isolated test launcher. Emit the same cancellable events
// consumed by the production listeners; never inject input into the parent.
static int testSwipeLua(lua_State *L) {
    const std::string phase = luaL_checkstring(L, 1);
    Event::SCallbackInfo info;
    if (phase == "begin")
        Event::bus()->m_events.gesture.swipe.begin.emit(
            IPointer::SSwipeBeginEvent{.fingers = static_cast<uint32_t>(luaL_checkinteger(L, 2))}, info);
    else if (phase == "update")
        Event::bus()->m_events.gesture.swipe.update.emit(
            IPointer::SSwipeUpdateEvent{.delta = {luaL_checknumber(L, 2), luaL_checknumber(L, 3)}}, info);
    else if (phase == "end")
        Event::bus()->m_events.gesture.swipe.end.emit(IPointer::SSwipeEndEvent{.cancelled = bool(lua_toboolean(L, 2))}, info);
    else
        return luaL_error(L, "expected begin/update/end");
    if (lua_isboolean(L, 4) && info.cancelled != bool(lua_toboolean(L, 4)))
        return luaL_error(L, "unexpected swipe cancellation result");
    lua_pushboolean(L, info.cancelled);
    return 1;
}
// Exercise the real InputManager pipeline, including native Lua gestures.
static int testPinchLua(lua_State *L) {
    const std::string phase = luaL_checkstring(L, 1);
    if (phase == "begin")
        g_pInputManager->onPinchBegin({.fingers = static_cast<uint32_t>(luaL_checkinteger(L, 2))});
    else if (phase == "update")
        g_pInputManager->onPinchUpdate({.fingers = static_cast<uint32_t>(luaL_checkinteger(L, 2)), .scale = luaL_checknumber(L, 3)});
    else if (phase == "end")
        g_pInputManager->onPinchEnd({.cancelled = bool(lua_toboolean(L, 2))});
    else
        return luaL_error(L, "expected begin/update/end");
    return 0;
}
APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }
APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    if (HyprlandAPI::getHyprlandVersion(handle).hash != GIT_COMMIT_HASH)
        throw std::runtime_error("hypr2d: compositor build does not match plugin");
    core.emplace(hypr2d::new_core());
    MonitorPolicy::init(&**core, handle);
    ExternalAnimation::init(&**core, handle);
    LOG(Log::INFO, "hypr2d M0: C++ -> Rust -> C++: 41 -> {}", hypr2d::spike_round_trip(41));
    for (const auto &[name, fn] : std::vector<std::pair<std::string, PLUGIN_LUA_FN>>{{"grid_create", gridCreateLua},
                                                                                     {"grid_rename", gridRenameLua},
                                                                                     {"grid_delete", gridDeleteLua},
                                                                                     {"adopt", adoptLua},
                                                                                     {"swap_grids", swapGridsLua},
                                                                                     {"translate_grid", translateGridLua},
                                                                                     {"move_grid", moveGridLua},
                                                                                     {"move", moveLua},
                                                                                     {"carry", carryLua},
                                                                                     {"overview", overviewLua},
                                                                                     {"parked", parkedLua},
                                                                                     {"goto_cell", gotoLua},
                                                                                     {"configure", configureLua},
                                                                                     {"_execute", invoke},
                                                                                     {"swipe_config", swipeConfigLua}})
        if (!HyprlandAPI::addLuaFunction(handle, "hypr2d", name, fn))
            throw std::runtime_error("hypr2d: failed Lua registration");
    if (!HyprlandAPI::registerHyprCtlCommand(handle, {.name = "hypr2d",
                                                      .match = IPC::Socket1::COMMAND_MATCH_PREFIX,
                                                      .handler = [](const IPC::Socket1::SRequest &req) -> IPC::Socket1::SResponse {
                                                          const auto pos = req.command.find(' ');
                                                          return execute(pos == std::string::npos ? "state" : req.command.substr(pos + 1));
                                                      }}))
        throw std::runtime_error("hypr2d: failed IPC registration");
    workspaceListener = Event::bus()->m_events.workspace.active.listen([](PHLWORKSPACE w) {
        if (w && w->numberedID())
            (*core)->extension_event("activate", std::to_string(*w->numberedID()), "");
        if (!applyingSwipe)
            cancelSwipe();
        IPC::Socket2::sock()->postEvent({"hypr2d", "changed"});
    });
    pinchBeginListener = Event::bus()->m_events.gesture.pinch.begin.listen([](IPointer::SPinchBeginEvent e, Event::SCallbackInfo &info) {
        try {
            cancelSwipe();
            swipeClaimed = false;
            info.cancelled |= ((*core)->pinch_event(0, e.fingers, 1., OverviewUI::active()) & 1) != 0;
        } catch (...) {
            LOG(Log::ERR, "hypr2d: pinch begin failed");
        }
    });
    pinchUpdateListener = Event::bus()->m_events.gesture.pinch.update.listen([](IPointer::SPinchUpdateEvent e, Event::SCallbackInfo &info) {
        try {
            const auto result = (*core)->pinch_event(1, e.fingers, e.scale, OverviewUI::active());
            info.cancelled |= (result & 1) != 0;
            if (result & 2)
                OverviewUI::command("overview close");
        } catch (...) {
            LOG(Log::ERR, "hypr2d: pinch update failed");
        }
    });
    pinchEndListener = Event::bus()->m_events.gesture.pinch.end.listen([](IPointer::SPinchEndEvent, Event::SCallbackInfo &info) {
        try {
            info.cancelled |= ((*core)->pinch_event(2, 0, 1., OverviewUI::active()) & 1) != 0;
        } catch (...) {
            LOG(Log::ERR, "hypr2d: pinch end failed");
        }
    });
    swipeBeginListener = Event::bus()->m_events.gesture.swipe.begin.listen([](IPointer::SSwipeBeginEvent e, Event::SCallbackInfo &info) {
        if ((*core)->pinch_active())
            return; // Pinch translation is never workspace swipe travel.
        if (OverviewUI::active()) {
            swipeClaimed = true; // Consume the entire sequence, including after close.
            info.cancelled = true;
            return;
        }
        if (swipeClaimed)
            cancelSwipe();
        // Leave special workspaces and unrelated finger counts to native handlers.
        const auto m = Desktop::focusState()->monitor();
        if (m && !m->m_activeSpecialWorkspace)
            swipeClaimed = feedSwipe(hypr2d::SwipePhase::Begin, e.fingers);
        else
            swipeClaimed = false;
        if (swipeClaimed)
            info.cancelled = true;
    });
    swipeUpdateListener = Event::bus()->m_events.gesture.swipe.update.listen([](IPointer::SSwipeUpdateEvent e, Event::SCallbackInfo &info) {
        if (!swipeClaimed)
            return;
        info.cancelled = true;
        if (!OverviewUI::active())
            feedSwipe(hypr2d::SwipePhase::Update, 0, e.delta.x, e.delta.y);
    });
    swipeEndListener = Event::bus()->m_events.gesture.swipe.end.listen([](IPointer::SSwipeEndEvent e, Event::SCallbackInfo &info) {
        if (!swipeClaimed)
            return;
        info.cancelled = true;
        if (!OverviewUI::active())
            feedSwipe(hypr2d::SwipePhase::End, 0, 0, 0, e.cancelled);
        swipeClaimed = false;
    });
    renderListener = Event::bus()->m_events.render.pre.listen([](PHLMONITOR m) {
        if (swipeAnimating && swipeMonitor == m)
            feedSwipe(hypr2d::SwipePhase::Tick);
    });
    monitorListener = Event::bus()->m_events.monitor.preRemoved.listen([](PHLMONITOR m) {
        if (swipeMonitor == m)
            cancelSwipe();
    });
    if (const char *testing = getenv("HYPR2D_TESTING"); testing && std::string(testing) == "1") {
        testingEnabled = true;
        HyprlandAPI::addLuaFunction(handle, "hypr2d", "_test_swipe", testSwipeLua);
        HyprlandAPI::addLuaFunction(handle, "hypr2d", "_test_pinch", testPinchLua);
        OverviewProbe::init();
    }
    OverviewUI::init(&**core);
    const auto runtime = getenv("XDG_RUNTIME_DIR"), signature = getenv("HYPRLAND_INSTANCE_SIGNATURE");
    if (!runtime || !signature)
        throw std::runtime_error("hypr2d: missing instance environment");
    auto fd = (*core)->extension_start(std::format("{}/hypr/{}/hypr2d.sock", runtime, signature));
    if (fd < 0)
        throw std::runtime_error("hypr2d: extension socket failed");
    extensionSource = wl_event_loop_add_fd(g_pCompositor->m_wlEventLoop, fd, WL_EVENT_READABLE, extensionReady, nullptr);
    if (!extensionSource) {
        (*core)->extension_stop();
        throw std::runtime_error("hypr2d: extension event source failed");
    }
    IPC::Socket2::sock()->postEvent({"hypr2d", "ready"});
    return {"hypr2d", "2D workspace grid", "Wolf Mermelstein", "0.2.0"};
}
APICALL EXPORT void PLUGIN_EXIT() {
    if (extensionSource)
        wl_event_source_remove(extensionSource);
    extensionSource = nullptr;
    if (core)
        (*core)->extension_stop();
    ExternalAnimation::shutdown();
    Doctor::shutdown();
    OverviewUI::shutdown();
    MonitorPolicy::shutdown();
    if (testingEnabled)
        OverviewProbe::shutdown();
    testingEnabled = false;
    IPC::Socket2::sock()->postEvent({"hypr2d", "unloaded"});
    pinchBeginListener.reset();
    pinchUpdateListener.reset();
    pinchEndListener.reset();
    swipeBeginListener.reset();
    swipeUpdateListener.reset();
    swipeEndListener.reset();
    renderListener.reset();
    monitorListener.reset();
    cancelSwipe();
    swipeClaimed = false;
    workspaceListener.reset();
    core.reset();
}
