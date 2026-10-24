#include "hypr2d-core/src/lib.rs.h"
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
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <hyprland/src/state/MonitorState.hpp>
#include <hyprland/src/state/workspace/State.hpp>
#include <hyprland/src/workspace/HLWorkspace.hpp>
extern "C" {
#include <lauxlib.h>
#include <lua.h>
}

static std::optional<rust::Box<hypr2d::Core>> core;
static CHyprSignalListener workspaceListener;

static std::vector<hypr2d::Monitor> monitors() {
    std::vector<hypr2d::Monitor> result;
    for (const auto &m : State::monitorState()->monitors()) {
        const auto ws = m->m_activeWorkspace;
        const auto off = ws ? ws->m_renderOffset->value() : Vector2D{};
        result.push_back({static_cast<uint32_t>(m->m_id), m->m_name, ws && ws->numberedID() ? *ws->numberedID() : 0,
                          m == Desktop::focusState()->monitor(), off.x, off.y});
    }
    return result;
}
static std::vector<hypr2d::Window> windows() {
    std::vector<hypr2d::Window> result;
    for (const auto &w : Desktop::windowState()->windows()) {
        if (!w->mapped() || !w->m_workspace || !w->m_workspace->numberedID())
            continue;
        result.push_back({*w->m_workspace->numberedID(), std::format("0x{:x}", reinterpret_cast<uintptr_t>(w.get())), w->metadata().title(),
                          w->metadata().appID(), bool(w->m_hints & Desktop::View::WINDOW_HINT_URGENT)});
    }
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
static std::string execute(const std::string &input) {
    try {
        auto ms = monitors();
        auto ws = windows();
        auto reply = (*core)->command({ms.data(), ms.size()}, {ws.data(), ws.size()}, input);
        if (!reply.error.empty())
            return "error: " + std::string(reply.error);
        const auto focus = Desktop::focusState()->monitor();
        const auto window = Desktop::focusState()->window();
        // Validate all targets before mutating any monitor.
        for (const auto &move : reply.moves) {
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
            if (old == target)
                continue;
            const auto savedStyle = target->m_animationStyle;
            const std::string style = move.vertical ? "slidevert" : "slide";
            target->m_animationStyle = style;
            if (!move.all)
                Desktop::focusState()->rawMonitorFocus(m);
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
            Desktop::focusState()->rawMonitorFocus(focus);
        if (input.starts_with("configure "))
            IPC::Socket2::sock()->postEvent({"hypr2d", "config"});
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
APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }
APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    if (HyprlandAPI::getHyprlandVersion(handle).hash != GIT_COMMIT_HASH)
        throw std::runtime_error("hypr2d: compositor build does not match plugin");
    core.emplace(hypr2d::new_core());
    LOG(Log::INFO, "hypr2d M0: C++ -> Rust -> C++: 41 -> {}", hypr2d::spike_round_trip(41));
    for (const auto &[name, fn] : std::vector<std::pair<std::string, PLUGIN_LUA_FN>>{
             {"move", moveLua}, {"carry", carryLua}, {"goto_cell", gotoLua}, {"configure", configureLua}, {"_execute", invoke}})
        if (!HyprlandAPI::addLuaFunction(handle, "hypr2d", name, fn))
            throw std::runtime_error("hypr2d: failed Lua registration");
    if (!HyprlandAPI::registerHyprCtlCommand(handle, {.name = "hypr2d",
                                                      .match = IPC::Socket1::COMMAND_MATCH_PREFIX,
                                                      .handler = [](const IPC::Socket1::SRequest &req) -> IPC::Socket1::SResponse {
                                                          const auto pos = req.command.find(' ');
                                                          return execute(pos == std::string::npos ? "state" : req.command.substr(pos + 1));
                                                      }}))
        throw std::runtime_error("hypr2d: failed IPC registration");
    workspaceListener =
        Event::bus()->m_events.workspace.active.listen([](PHLWORKSPACE) { IPC::Socket2::sock()->postEvent({"hypr2d", "changed"}); });
    return {"hypr2d", "2D workspace grid", "Wolf Mermelstein", "0.2.0"};
}
APICALL EXPORT void PLUGIN_EXIT() {
    workspaceListener.reset();
    core.reset();
}
