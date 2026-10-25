#include "ui.hpp"
#include "overview.hpp"
#include <chrono>
#include <deque>
#include <hyprland/src/config/shared/actions/ConfigActions.hpp>
#include <hyprland/src/desktop/state/FocusState.hpp>
#include <hyprland/src/desktop/state/WindowState.hpp>
#include <hyprland/src/desktop/view/window/Window.hpp>
#include <hyprland/src/desktop/view/window/WindowMetadata.hpp>
#include <hyprland/src/desktop/view/window/WindowPresentation.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/ipc/s2/S2.hpp>
#include <hyprland/src/managers/SeatManager.hpp>
#include <hyprland/src/managers/fullscreen/FullscreenController.hpp>
#include <hyprland/src/pointer/PointerManager.hpp>
#include <hyprland/src/protocols/core/Compositor.hpp>
#include <hyprland/src/protocols/core/Subcompositor.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/render/gl/GLFramebuffer.hpp>
#include <hyprland/src/state/MonitorState.hpp>
#include <hyprland/src/state/workspace/State.hpp>
#include <hyprland/src/workspace/HLWorkspace.hpp>
#include <hyprutils/utils/ScopeGuard.hpp>
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
static bool deletedObjects = true;
static Vector2D pointer;
static uint32_t captures = 0;
static double captureMs = 0;
static std::set<uint32_t> swallowedKeys;
static bool swallowedButton = false;
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
        if (!w->mapped() || w->isHidden() || w->presentation().alphaTotal() == 0.f || !w->m_workspace || !w->m_monitor)
            continue;
        auto pos = w->position(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
        auto size = w->size(Desktop::View::IGeometric::GEOMETRIC_CURRENT);
        auto m = w->m_monitor.lock();
        result.push_back({std::format("0x{:x}", reinterpret_cast<uintptr_t>(w.get())), key(w->m_workspace), static_cast<uint32_t>(m->m_id),
                          pos.x - m->m_position.x, pos.y - m->m_position.y, size.x, size.y, w->isFloating(), w->metadata().appID()});
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
        w->wlSurface()->resource()->breadthfirst(
            [weak](SP<CWLSurfaceResource> surface, const Vector2D &, void *) {
                surfaceListeners.push_back(surface->m_events.commit.listen([weak] {
                    if (auto w = weak.lock(); w && w->m_workspace)
                        mark(key(w->m_workspace));
                }));
                surfaceListeners.push_back(surface->m_events.newSubsurface.listen([weak](SP<CWLSubsurfaceResource>) {
                    if (auto w = weak.lock(); ui && w && w->m_workspace) {
                        rebuild = true;
                        mark(key(w->m_workspace));
                    }
                }));
            },
            nullptr);
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
    std::vector<GLuint> textures, buffers;
    for (auto &[key, fb] : cache) {
        textures.push_back(fb->getTexture()->m_texID);
        buffers.push_back(dynamicPointerCast<Render::GL::CGLFramebuffer>(fb)->getFBID());
    }
    for (auto &[key, tex] : textCache)
        textures.push_back(tex->m_texID);
    cache.clear();
    textCache.clear();
    deletedObjects = true;
    for (auto id : textures)
        deletedObjects &= !glIsTexture(id);
    for (auto id : buffers)
        deletedObjects &= !glIsFramebuffer(id);
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
static CHyprColor color(uint32_t rgb, double alpha) {
    return CHyprColor(((rgb >> 16) & 255) / 255.0, ((rgb >> 8) & 255) / 255.0, (rgb & 255) / 255.0, alpha);
}
static SP<Render::ITexture> text(const std::string &str, int fontSize, int width) {
    auto key = std::format("{}:{}:{}", fontSize, width, str);
    if (!textCache.contains(key))
        textCache[key] =
            g_pHyprRenderer->renderText(str, color(core->ui_style().label, 1), fontSize, false, std::string(core->ui_style().font), width);
    return textCache[key];
}
// The ghost samples a cropped region of an existing thumbnail; no extra capture.
class GhostPass final : public IPassElement {
    SP<Render::ITexture> texture;
    CBox box;
    Vector2D uv0, uv1;
    float alpha;

  public:
    GhostPass(SP<Render::ITexture> tex, const hypr2d::UIGhost &ghost, float a)
        : texture(tex), box{ghost.rect.x, ghost.rect.y, ghost.rect.width, ghost.rect.height}, uv0{ghost.uv.x, ghost.uv.y},
          uv1{ghost.uv.x + ghost.uv.width, ghost.uv.y + ghost.uv.height}, alpha(a) {}
    std::vector<UP<IPassElement>> draw() override {
        auto &r = *g_pHyprRenderer;
        auto tl = r.m_renderData.primarySurfaceUVTopLeft, br = r.m_renderData.primarySurfaceUVBottomRight;
        Hyprutils::Utils::CScopeGuard restore([&] {
            r.m_renderData.primarySurfaceUVTopLeft = tl;
            r.m_renderData.primarySurfaceUVBottomRight = br;
        });
        r.m_renderData.primarySurfaceUVTopLeft = uv0;
        r.m_renderData.primarySurfaceUVBottomRight = uv1;
        r.draw(CTexPassElement::SRenderData{.tex = texture, .box = box, .a = alpha, .allowCustomUV = true}, r.m_renderData.damage);
        return {};
    }
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    const char *passName() override { return "hypr2d-ghost"; }
    ePassElementType type() override { return EK_CUSTOM; }
};
class Pass final : public IPassElement {
    hypr2d::UIFrame frame;
    Vector2D size;

  public:
    Pass(hypr2d::UIFrame f, Vector2D s) : frame(std::move(f)), size(s) {}
    std::vector<UP<IPassElement>> draw() override {
        std::vector<UP<IPassElement>> items;
        auto style = core->ui_style();
        auto rect = [&](const CBox &box, CHyprColor color) {
            items.push_back(makeUnique<CRectPassElement>(
                CRectPassElement::SRectData{.box = box, .color = color, .round = static_cast<int>(style.rounding)}));
        };
        auto label = [&](const std::string &str, const CBox &box, float opacity = 1.f) {
            auto tex = text(str, static_cast<int>(style.font_size), std::clamp(static_cast<int>(box.w), 1, 500));
            if (tex)
                items.push_back(makeUnique<CTexPassElement>(
                    CTexPassElement::SRenderData{.tex = tex,
                                                 .box = {box.x, box.y, std::min(box.w, tex->m_size.x), std::min(box.h, tex->m_size.y)},
                                                 .a = float(frame.alpha * opacity)}));
        };
        const double uiScale = frame.labels.empty() ? 1.0 : frame.labels[0].rect.height / 24.0;
        rect({0, 0, size.x, size.y}, color(style.background, frame.alpha));
        for (auto &c : frame.cells) {
            CBox box{c.rect.x, c.rect.y, c.rect.width, c.rect.height};
            const auto borderColor = std::string(c.key) == std::string(frame.target) ? color(style.target, frame.alpha)
                                     : c.selected                                    ? color(style.current, frame.alpha)
                                     : c.hovered                                     ? color(style.hover, frame.alpha)
                                     : c.shown                                       ? color(style.current, frame.alpha)
                                                                                     : color(style.border, frame.alpha);
            rect(box.copy().expand(2), borderColor);
            rect(box, color(style.cell, frame.alpha));
            if (auto it = cache.find(std::string(c.key)); it != cache.end())
                items.push_back(
                    makeUnique<CTexPassElement>(CTexPassElement::SRenderData{.tex = it->second->getTexture(),
                                                                             .box = box,
                                                                             .a = float(frame.alpha * (c.occupied ? 1.0 : 0.35)),
                                                                             .round = static_cast<int>(style.rounding)}));
            if (!c.label.empty())
                label(std::string(c.label),
                      {box.x + 3 * uiScale, box.y + 2 * uiScale, std::max(1.0, box.w - 6 * uiScale),
                       std::min(static_cast<double>(style.font_size + 6), box.h * 0.45)},
                      c.occupied ? 1.f : 0.45f);
        }
        for (auto &l : frame.labels)
            label(std::string(l.text), {l.rect.x, l.rect.y, l.rect.width, l.rect.height});
        for (auto &w : frame.windows)
            if (w.target) {
                CBox b{w.rect.x, w.rect.y, w.rect.width, w.rect.height};
                auto col = color(style.target, frame.alpha);
                rect({b.x, b.y, b.w, 2}, col);
                rect({b.x, b.y + b.h - 2, b.w, 2}, col);
                rect({b.x, b.y, 2, b.h}, col);
                rect({b.x + b.w - 2, b.y, 2, b.h}, col);
            }
        if (frame.ghost.visible) {
            if (auto it = cache.find(std::string(frame.ghost.key)); it != cache.end())
                items.push_back(makeUnique<GhostPass>(it->second->getTexture(), frame.ghost, float(0.72 * frame.alpha)));
        }
        return items;
    }
    bool needsLiveBlur() override { return false; }
    bool needsPrecomputeBlur() override { return false; }
    const char *passName() override { return "hypr2d-overview"; }
    ePassElementType type() override { return EK_CUSTOM; }
    bool disableSimplification() override { return true; }
    bool requiresFullDamage() override { return true; }
};
static PHLWINDOW findWindow(const std::string &id) {
    for (auto &w : Desktop::windowState()->windows())
        if (w->mapped() && std::format("0x{:x}", reinterpret_cast<uintptr_t>(w.get())) == id)
            return w;
    return nullptr;
}
static void applyDrop(const hypr2d::UIDrop &drop) {
    if (!drop.activate.empty()) {
        activate(std::string(drop.activate));
        return;
    }
    if (drop.window.empty())
        return;
    auto w = findWindow(std::string(drop.window));
    if (!w || !w->m_workspace)
        return;
    auto source = key(w->m_workspace);
    auto focus = Desktop::focusState()->monitor();
    const auto savedPointer = Pointer::mgr()->position();
    Hyprutils::Utils::CScopeGuard restore([&] {
        if (focus)
            (void)Config::Actions::focusMonitor(focus);
        (void)Config::Actions::moveCursor(savedPointer);
        mark(source);
        mark(std::string(drop.workspace));
        rebuild = true;
        damage();
    });
    if (!drop.swap.empty()) {
        auto other = findWindow(std::string(drop.swap));
        if (!other || other == w || !other->m_workspace || key(other->m_workspace) != std::string(drop.workspace))
            return;
        auto first = Fullscreen::controller()->getFullscreenModes(w), second = Fullscreen::controller()->getFullscreenModes(other);
        (void)Config::Actions::fullscreenWindow(Fullscreen::FSMODE_NONE, Fullscreen::FSMODE_NONE, false, w);
        (void)Config::Actions::fullscreenWindow(Fullscreen::FSMODE_NONE, Fullscreen::FSMODE_NONE, false, other);
        auto result = Config::Actions::swapWith(other, w);
        (void)Config::Actions::fullscreenWindow(first.internal, first.client, false, w);
        (void)Config::Actions::fullscreenWindow(second.internal, second.client, false, other);
        if (!result)
            return;
    } else {
        PHLMONITOR mon;
        for (auto &m : State::monitorState()->monitors())
            if (m->m_id == drop.monitor)
                mon = m;
        if (!mon)
            return;
        PHLWORKSPACE target;
        for (auto &ws : State::Workspace::state()->workspaces())
            if (key(ws.lock()) == std::string(drop.workspace))
                target = ws.lock();
        if (!target && !std::string(drop.workspace).starts_with("special:"))
            target = State::Workspace::state()->createNumbered(
                Workspace::SWorkspaceNumberedID{static_cast<uint32_t>(std::stoul(std::string(drop.workspace)))}, mon);
        if (!target || target->m_monitor != mon)
            return;
        if (!Config::Actions::moveToWorkspace(target, true, w))
            return;
        if (drop.floating && w->isFloating())
            (void)Config::Actions::move(mon->m_position + Vector2D{drop.x, drop.y}, false, w);
    }
    IPC::Socket2::sock()->postEvent({"hypr2d", std::format("drop>>{},{}", std::string(drop.window), std::string(drop.workspace))});
}
// phase 0=motion, 1=press, 2=release; Rust owns the drag state machine.
static void movePointer(Vector2D global, int phase) {
    if (!ui)
        return;
    pointer = global;
    for (auto &m : State::monitorState()->monitors())
        if (CBox{m->m_position, m->m_size}.containsPoint(global)) {
            auto p = global - m->m_position;
            if (phase == 1)
                (*ui)->overview_press(m->m_size.x, m->m_size.y, p.x, p.y);
            else if (phase == 2)
                applyDrop((*ui)->overview_release(m->m_size.x, m->m_size.y, p.x, p.y));
            else
                (*ui)->overview_pointer(m->m_size.x, m->m_size.y, p.x, p.y, false);
            damage();
            return;
        }
    // A release outside all outputs cancels instead of keeping a stale drag.
    if (phase == 2)
        (*ui)->overview_release(1000, 700, -1, -1);
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
                            if (key(w.lock()) == k) {
                                g_pHyprRenderer->sendFrameEventsToWorkspace(mon, w.lock(), Time::steadyNow());
                                for (auto &win : Desktop::windowState()->windows())
                                    if (win->mapped() && win->m_workspace == w && win->wlSurface() && win->wlSurface()->resource())
                                        win->wlSurface()->resource()->breadthfirst([](SP<CWLSurfaceResource> surface, const Vector2D &,
                                                                                      void *) { surface->frame(Time::steadyNow()); },
                                                                                   nullptr);
                            }
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
    listeners.push_back(e.input.mouse.move.listen([](Vector2D, Event::SCallbackInfo &info) {
        if (ui) {
            info.cancelled = true;
            movePointer(Pointer::mgr()->position(), 0);
        }
    }));
    listeners.push_back(e.input.mouse.button.listen([](IPointer::SButtonEvent event, Event::SCallbackInfo &info) {
        if (event.button != 272) {
            if (ui)
                info.cancelled = true;
            return;
        }
        if (event.state == WL_POINTER_BUTTON_STATE_RELEASED) {
            if (swallowedButton) {
                info.cancelled = true;
                swallowedButton = false;
                if (ui)
                    movePointer(pointer, 2);
            }
        } else if (ui) {
            info.cancelled = true;
            swallowedButton = true;
            movePointer(pointer, 1);
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
    listeners.push_back(e.workspace.active.listen([changed](PHLWORKSPACE) { changed(); }));
    listeners.push_back(e.workspace.specialActive.listen([changed](PHLWORKSPACE, PHLMONITOR) { changed(); }));
}
void shutdown() {
    listeners.clear();
    if (ui)
        release();
    core = nullptr;
}
std::string command(const std::string &input) {
    if (input == "overview refresh") {
        textCache.clear();
        if (ui) {
            rebuild = true;
            damage();
        }
        return "ok";
    }
    if (input == "overview state") {
        if (!ui)
            return deletedObjects ? "{\"open\":false,\"deleted_gl_objects\":true}" : "{\"open\":false,\"deleted_gl_objects\":false}";
        auto m = Desktop::focusState()->monitor();
        return std::format("{{\"ui\":{},\"captures\":{},\"capture_ms\":{},\"pending\":{}}}",
                           std::string((*ui)->overview_state(m ? m->m_size.x : 1000.0, m ? m->m_size.y : 700.0, now())), captures,
                           captureMs, dirty.size());
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
    if (State::monitorState()->monitors().empty() || !Desktop::focusState()->monitor())
        return "error: overview requires a focused output";
    pointer = Pointer::mgr()->position();
    captures = 0;
    captureMs = 0;
    build();
    damage();
    IPC::Socket2::sock()->postEvent({"hypr2d", "overview>>open"});
    return "ok";
}
} // namespace OverviewUI
