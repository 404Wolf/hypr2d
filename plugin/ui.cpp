#include "ui.hpp"
#include "overview.hpp"
#include <chrono>
#include <deque>
#include <hyprland/src/config/shared/actions/ConfigActions.hpp>
#include <hyprland/src/desktop/state/FocusState.hpp>
#include <hyprland/src/desktop/state/WindowState.hpp>
#include <hyprland/src/desktop/view/window/Window.hpp>
#include <hyprland/src/desktop/view/window/WindowMetadata.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/ipc/s2/S2.hpp>
#include <hyprland/src/managers/SeatManager.hpp>
#include <hyprland/src/pointer/PointerManager.hpp>
#include <hyprland/src/protocols/core/Compositor.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/state/MonitorState.hpp>
#include <hyprland/src/state/workspace/State.hpp>
#include <hyprland/src/workspace/HLWorkspace.hpp>
#include <map>
#include <optional>
#include <set>

namespace OverviewUI {
static hypr2d::Core *core;
static std::optional<rust::Box<hypr2d::Overview>> ui;
static std::map<std::string, SP<Render::IFramebuffer>> cache;
static std::map<std::string, SP<Render::ITexture>> textCache;
static std::vector<PHLWORKSPACE> heldSpecials;
static std::vector<CHyprSignalListener> listeners, surfaceListeners;
static std::deque<std::string> dirty;
static bool capturing = false, rebuild = false;
static Vector2D pointer;
static uint32_t captures = 0;
static double captureMs = 0;
static std::set<uint32_t> swallowedKeys;
static double now() { return std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now().time_since_epoch()).count(); }
static void damage() {
    for (auto &m : State::monitorState()->monitors()) {
        g_pHyprRenderer->damageMonitor(m);
        m->scheduleFrame(Aquamarine::IOutput::AQ_SCHEDULE_DAMAGE);
    }
}
bool active() { return ui.has_value(); }
static std::string key(PHLWORKSPACE w) { return w->numberedID() ? std::to_string(*w->numberedID()) : w->addressableName(); }
static std::vector<hypr2d::UIWindow> windows() {
    std::vector<hypr2d::UIWindow> result;
    for (auto &w : Desktop::windowState()->windows()) {
        if (!w->mapped() || !w->m_workspace || !w->m_monitor)
            continue;
        auto pos = w->position(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
        auto size = w->size(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
        auto m = w->m_monitor.lock();
        result.push_back({std::format("0x{:x}", reinterpret_cast<uintptr_t>(w.get())), key(w->m_workspace), static_cast<uint32_t>(m->m_id),
                          pos.x - m->m_position.x, pos.y - m->m_position.y, size.x, size.y, w->isFloating(), w->metadata().title()});
    }
    return result;
}
static void mark(const std::string &key) {
    if (active() && std::ranges::find(dirty, key) == dirty.end()) {
        dirty.push_back(key);
        damage();
    }
}
static void watchSurfaces() {
    surfaceListeners.clear();
    for (auto &w : Desktop::windowState()->windows()) {
        if (!w->mapped() || !w->wlSurface() || !w->wlSurface()->resource())
            continue;
        auto weak = w->m_self;
        surfaceListeners.push_back(w->wlSurface()->resource()->m_events.commit.listen([weak] {
            if (auto w = weak.lock(); w && w->m_workspace)
                mark(key(w->m_workspace));
        }));
    }
}
static void build() {
    std::vector<hypr2d::UIOutput> outputs;
    std::vector<hypr2d::UISpecial> specials;
    auto previousSpecials = std::move(heldSpecials);
    heldSpecials.clear();
    for (auto &m : State::monitorState()->monitors())
        outputs.push_back({static_cast<uint32_t>(m->m_id), m->m_name, m->m_position.x, m->m_position.y, m->m_size.x, m->m_size.y,
                           m->m_activeWorkspace && m->m_activeWorkspace->numberedID() ? *m->m_activeWorkspace->numberedID() : 0,
                           m == Desktop::focusState()->monitor()});
    for (auto &w : State::Workspace::state()->workspaces())
        if (w->type() == Workspace::eWorkspaceType::SPECIAL && w->m_monitor) {
            heldSpecials.push_back(w.lock());
            specials.push_back(
                {static_cast<uint32_t>(w->m_monitor->m_id), w->addressableName(), w->m_monitor->m_activeSpecialWorkspace == w});
        }
    auto wins = windows();
    auto replacement =
        core->overview_new({outputs.data(), outputs.size()}, {specials.data(), specials.size()}, {wins.data(), wins.size()}, now());
    if (ui) {
        auto keys = (*ui)->overview_reconcile(std::move(replacement));
        for (auto &k : keys)
            mark(std::string(k));
    } else {
        ui.emplace(std::move(replacement));
        for (auto &w : wins)
            mark(std::string(w.workspace));
    }
    watchSurfaces();
    rebuild = false;
}
static void release() {
    g_pHyprRenderer->m_renderPass.removeAllOfType("hypr2d-overview");
    surfaceListeners.clear();
    ui.reset();
    cache.clear();
    textCache.clear();
    dirty.clear();
    heldSpecials.clear();
    rebuild = false;
    IPC::Socket2::sock()->postEvent({"hypr2d", "overview>>close"});
    damage();
}
static void activate(const std::string &target) {
    if (target.empty())
        return;
    auto frame = (*ui)->overview_frame(1000, 700, now());
    for (auto &cell : frame.cells)
        if (std::string(cell.key) == target) {
            PHLMONITOR mon;
            for (auto &m : State::monitorState()->monitors())
                if (m->m_id == cell.monitor)
                    mon = m;
            if (!mon)
                return;
            PHLWORKSPACE ws;
            for (auto &w : State::Workspace::state()->workspaces())
                if (key(w.lock()) == target)
                    ws = w.lock();
            if (!ws && !target.starts_with("special:"))
                ws = State::Workspace::state()->createNumbered(Workspace::SWorkspaceNumberedID{static_cast<uint32_t>(std::stoul(target))},
                                                               mon);
            if (!ws)
                return;
            release();
            (void)Config::Actions::focusMonitor(mon);
            if (target.starts_with("special:")) {
                if (mon->m_activeSpecialWorkspace != ws)
                    (void)Config::Actions::toggleSpecial(ws);
            } else {
                if (mon->m_activeSpecialWorkspace)
                    mon->setSpecialWorkspace(nullptr);
                mon->changeWorkspace(ws, false, true);
            }
            return;
        }
}
static SP<Render::ITexture> text(const std::string &str) {
    if (!textCache.contains(str))
        textCache[str] = g_pHyprRenderer->renderText(str, CHyprColor(0.67, 0.7, 0.76, 1.0), 12, false, "sans-serif", 500);
    return textCache[str];
}
class Pass final : public IPassElement {
    hypr2d::UIFrame frame;
    Vector2D size;

  public:
    Pass(hypr2d::UIFrame f, Vector2D s) : frame(std::move(f)), size(s) {}
    std::vector<UP<IPassElement>> draw() override {
        std::vector<UP<IPassElement>> items;
        auto rect = [&](const CBox &box, CHyprColor color) {
            items.push_back(makeUnique<CRectPassElement>(CRectPassElement::SRectData{.box = box, .color = color}));
        };
        auto label = [&](const std::string &str, const CBox &box, float opacity = 1.f) {
            auto tex = text(str);
            if (tex)
                items.push_back(makeUnique<CTexPassElement>(
                    CTexPassElement::SRenderData{.tex = tex,
                                                 .box = {box.x, box.y, std::min(box.w, tex->m_size.x), std::min(box.h, tex->m_size.y)},
                                                 .a = float(frame.alpha * opacity)}));
        };
        rect({0, 0, size.x, size.y}, CHyprColor(0.025, 0.03, 0.04, frame.alpha));
        for (auto &c : frame.cells) {
            CBox box{c.rect.x, c.rect.y, c.rect.width, c.rect.height};
            const auto color = c.selected  ? CHyprColor(0.38, 0.68, 0.94, frame.alpha)
                               : c.hovered ? CHyprColor(0.77, 0.48, 0.86, frame.alpha)
                               : c.shown   ? CHyprColor(0.6, 0.76, 0.47, frame.alpha)
                                           : CHyprColor(0.15, 0.17, 0.20, frame.alpha);
            rect(box.copy().expand(2), color);
            rect(box, CHyprColor(0.055, 0.06, 0.075, frame.alpha));
            if (auto it = cache.find(std::string(c.key)); it != cache.end())
                items.push_back(makeUnique<CTexPassElement>(CTexPassElement::SRenderData{
                    .tex = it->second->getTexture(), .box = box, .a = float(frame.alpha * (c.occupied ? 1.0 : 0.35))}));
            if (std::string(c.key).starts_with("special:") || c.selected || c.hovered || c.shown)
                label(std::string(c.label), {box.x + 3, box.y + 2, std::max(1.0, box.w - 6), std::min(14.0, box.h)},
                      c.occupied ? 1.f : 0.45f);
        }
        for (auto &l : frame.labels)
            label(std::string(l.text), {l.rect.x, l.rect.y, l.rect.width, l.rect.height});
        return items;
    }
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    const char *passName() override { return "hypr2d-overview"; }
    ePassElementType type() override { return EK_CUSTOM; }
    bool disableSimplification() override { return true; }
    bool requiresFullDamage() override { return true; }
};
static void movePointer(Vector2D global, bool click) {
    if (!ui)
        return;
    pointer = global;
    for (auto &m : State::monitorState()->monitors())
        if (CBox{m->m_position, m->m_size}.containsPoint(global)) {
            auto p = global - m->m_position;
            auto target = (*ui)->overview_pointer(m->m_size.x, m->m_size.y, p.x, p.y, click);
            damage();
            activate(std::string(target));
            return;
        }
}
void init(hypr2d::Core *c) {
    core = c;
    auto &e = Event::bus()->m_events;
    listeners.push_back(e.render.pre.listen([](PHLMONITOR) {
        if (!ui || capturing)
            return;
        auto wins = windows();
        for (auto &k : (*ui)->overview_windows({wins.data(), wins.size()}))
            mark(std::string(k));
        auto frame = (*ui)->overview_frame(1000, 700, now());
        if (!frame.alive) {
            release();
            return;
        }
        if (rebuild) {
            build();
            frame = (*ui)->overview_frame(1000, 700, now());
        }
        const double start = now();
        capturing = true;
        while (!dirty.empty() && now() - start < 2.0) {
            auto k = dirty.front();
            dirty.pop_front();
            for (auto &cell : frame.cells)
                if (std::string(cell.key) == k) {
                    if (!cell.occupied) {
                        cache.erase(k);
                        break;
                    }
                    PHLMONITOR mon;
                    for (auto &m : State::monitorState()->monitors())
                        if (m->m_id == cell.monitor)
                            mon = m;
                    if (!mon)
                        break;
                    hypr2d::ProbeCell cap{cell.monitor,
                                          k.starts_with("special:") ? 0u : static_cast<uint32_t>(std::stoul(k)),
                                          k.starts_with("special:") ? k : "",
                                          0,
                                          0,
                                          0,
                                          0,
                                          160,
                                          static_cast<uint32_t>(std::max(1.0, std::round(160 * mon->m_size.y / mon->m_size.x)))};
                    try {
                        double t = now();
                        cache[k] = OverviewProbe::capture(cap, cache[k], false);
                        captureMs += now() - t;
                        ++captures;
                        for (auto &w : State::Workspace::state()->workspaces())
                            if (key(w.lock()) == k)
                                g_pHyprRenderer->sendFrameEventsToWorkspace(mon, w.lock(), Time::steadyNow());
                    } catch (const std::exception &) {
                        cache.erase(k);
                    }
                    break;
                }
        }
        capturing = false;
        if (!dirty.empty() || frame.animating)
            damage();
    }));
    listeners.push_back(e.render.stage.listen([](eRenderStage stage) {
        if (ui && !capturing && stage == RENDER_LAST_MOMENT) {
            auto m = g_pHyprRenderer->m_renderData.pMonitor.lock();
            g_pHyprRenderer->m_renderPass.add(
                makeUnique<Pass>((*ui)->overview_frame(m->m_transformedSize.x, m->m_transformedSize.y, now()), m->m_transformedSize));
        }
    }));
    listeners.push_back(e.input.mouse.move.listen([](Vector2D p, Event::SCallbackInfo &info) {
        if (ui) {
            info.cancelled = true;
            movePointer(p, false);
        }
    }));
    listeners.push_back(e.input.mouse.button.listen([](IPointer::SButtonEvent event, Event::SCallbackInfo &info) {
        if (ui) {
            info.cancelled = true;
            if (event.button == 272 && event.state == WL_POINTER_BUTTON_STATE_RELEASED)
                movePointer(pointer, true);
        }
    }));
    listeners.push_back(e.input.mouse.axis.listen([](IPointer::SAxisEvent, Event::SCallbackInfo &info) {
        if (ui)
            info.cancelled = true;
    }));
    listeners.push_back(e.input.keyboard.key.listen([](IKeyboard::SKeyEvent event, Event::SCallbackInfo &info) {
        if (event.state == WL_KEYBOARD_KEY_STATE_RELEASED) {
            if (swallowedKeys.erase(event.keycode))
                info.cancelled = true;
            return;
        }
        if (!ui)
            return;
        info.cancelled = true;
        swallowedKeys.insert(event.keycode);
        auto kb = g_pSeatManager->m_keyboard.lock();
        if (!kb)
            return;
        auto sym = xkb_state_key_get_one_sym(kb->m_xkbState, event.keycode + 8);
        const bool shift = xkb_state_mod_name_is_active(kb->m_xkbState, XKB_MOD_NAME_SHIFT, XKB_STATE_MODS_EFFECTIVE) > 0;
        auto target = (*ui)->overview_key(sym, shift, now());
        damage();
        activate(std::string(target));
    }));
    auto changed = [] {
        if (ui && !capturing) {
            rebuild = true;
            damage();
        }
    };
    listeners.push_back(e.workspace.created.listen([changed](PHLWORKSPACEREF w) {
        if (ui && w && w->type() == Workspace::eWorkspaceType::SPECIAL) {
            heldSpecials.push_back(w.lock());
            changed();
        }
    }));
    listeners.push_back(e.window.open.listen([changed](PHLWINDOW) { changed(); }));
    listeners.push_back(e.window.close.listen([changed](PHLWINDOW) { changed(); }));
    listeners.push_back(e.window.moveToWorkspace.listen([changed](PHLWINDOW, PHLWORKSPACE) { changed(); }));
    listeners.push_back(e.window.title.listen([changed](PHLWINDOW) { changed(); }));
    listeners.push_back(e.monitor.preRemoved.listen([](PHLMONITOR) {
        if (ui)
            release();
    }));
    listeners.push_back(e.monitor.layoutChanged.listen([changed] { changed(); }));
    listeners.push_back(e.workspace.specialActive.listen([changed](PHLWORKSPACE, PHLMONITOR) { changed(); }));
}
void shutdown() {
    listeners.clear();
    if (ui)
        release();
    core = nullptr;
}
std::string command(const std::string &input) {
    if (input == "overview state") {
        if (!ui)
            return "{\"open\":false}";
        auto m = Desktop::focusState()->monitor();
        return std::format("{{\"ui\":{},\"captures\":{},\"capture_ms\":{},\"pending\":{}}}",
                           std::string((*ui)->overview_state(m->m_size.x, m->m_size.y, now())), captures, captureMs, dirty.size());
    }
    if (input == "overview close") {
        if (ui)
            (*ui)->overview_close(now());
        damage();
        return "ok";
    }
    if (input != "overview" && input != "overview open")
        return "error: expected overview [open|close|state]";
    if (ui) {
        if (input == "overview open")
            return "ok";
        (*ui)->overview_close(now());
        damage();
        return "ok";
    }
    if (g_pHyprRenderer->type() != Render::IHyprRenderer::RT_GL)
        return "error: overview requires the GL renderer";
    pointer = Pointer::mgr()->position();
    captures = 0;
    captureMs = 0;
    build();
    damage();
    IPC::Socket2::sock()->postEvent({"hypr2d", "overview>>open"});
    return "ok";
}
} // namespace OverviewUI
