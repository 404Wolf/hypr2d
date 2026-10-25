#include "ui.hpp"
#include "doctor.hpp"
#include "overview.hpp"
#include <chrono>
#include <deque>
#include <hyprland/src/config/lua/ConfigManager.hpp>
#include <hyprland/src/config/shared/actions/ConfigActions.hpp>
#include <hyprland/src/desktop/state/FocusState.hpp>
#include <hyprland/src/desktop/state/WindowState.hpp>
#include <hyprland/src/desktop/view/Group.hpp>
#include <hyprland/src/desktop/view/window/Window.hpp>
#include <hyprland/src/desktop/view/window/WindowGroupMembership.hpp>
#include <hyprland/src/desktop/view/window/WindowMetadata.hpp>
#include <hyprland/src/desktop/view/window/WindowPresentation.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/helpers/MiscFunctions.hpp>
#include <hyprland/src/ipc/s2/S2.hpp>
#include <hyprland/src/layout/space/Space.hpp>
#include <hyprland/src/layout/target/Target.hpp>
#include <hyprland/src/managers/SeatManager.hpp>
#include <hyprland/src/managers/eventLoop/EventLoopManager.hpp>
#include <hyprland/src/managers/fullscreen/FullscreenController.hpp>
#include <hyprland/src/pointer/PointerManager.hpp>
#include <hyprland/src/pointer/cursor/CursorShapeOverrideController.hpp>
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
static std::map<uint32_t, uint32_t> backingMonitors;
static uint32_t backing(uint32_t id) {
    auto found = backingMonitors.find(id);
    return found == backingMonitors.end() ? id : found->second;
}
static std::vector<CHyprSignalListener> listeners, surfaceListeners;
static std::deque<std::string> dirty;
static bool capturing = false, rebuild = false;
static bool deletedObjects = true;
static Vector2D pointer;
static uint32_t captures = 0;
static double captureMs = 0;
static std::set<uint32_t> swallowedKeys;
static std::set<std::string> emptiedSpecials;
static bool swallowedButton = false;
static std::vector<uint64_t> pendingDrops;
static double now() { return std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now().time_since_epoch()).count(); }
static void damage() {
    for (auto &m : State::monitorState()->monitors()) {
        g_pHyprRenderer->damageMonitor(m);
        m->scheduleFrame(Aquamarine::IOutput::AQ_SCHEDULE_DAMAGE);
    }
}
bool active() { return ui.has_value(); }
static std::string key(PHLWORKSPACE w) { return w->numberedID() ? std::to_string(*w->numberedID()) : w->addressableName(); }
static std::vector<hypr2d::UIWindow> windows(bool includeHidden = false) {
    std::vector<hypr2d::UIWindow> result;
    for (auto &w : Desktop::windowState()->windows()) {
        if (!w->mapped() || (!includeHidden && (w->isHidden() || w->presentation().alphaTotal() == 0.f)) || !w->m_workspace ||
            !w->m_monitor)
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
static SP<Render::ITexture> text(const std::string &, int, int, uint32_t);
static void build() {
    auto style = core->ui_style();
    if (auto metrics = text("Ag", static_cast<int>(style.font_size), 1000, style.label))
        core->ui_font_height(metrics->m_size.y);
    std::vector<hypr2d::UIOutput> outputs;
    std::vector<hypr2d::UISpecial> specials;
    auto previousSpecials = std::move(heldSpecials);
    heldSpecials.clear();
    for (auto &m : State::monitorState()->monitors())
        outputs.push_back({static_cast<uint32_t>(m->m_id), core->monitor_slot(m->m_name), false, static_cast<uint32_t>(m->m_id), m->m_name,
                           m->m_position.x, m->m_position.y, m->m_size.x, m->m_size.y,
                           m->m_activeWorkspace && m->m_activeWorkspace->numberedID() ? *m->m_activeWorkspace->numberedID() : 0,
                           m == Desktop::focusState()->monitor()});
    for (auto &w : State::Workspace::state()->workspaces())
        if (w->type() == Workspace::eWorkspaceType::SPECIAL && w->m_monitor) {
            const auto name = w->addressableName();
            const bool occupied = std::ranges::any_of(Desktop::windowState()->windows(),
                                                      [&](const auto &win) { return win->mapped() && win->m_workspace == w; });
            if (occupied)
                emptiedSpecials.erase(name);
            if (emptiedSpecials.contains(name))
                continue;
            heldSpecials.push_back(w.lock());
            specials.push_back(
                {static_cast<uint32_t>(w->m_monitor->m_id), w->addressableName(), w->m_monitor->m_activeSpecialWorkspace == w});
        }
    for (const auto &output : outputs)
        if (auto metrics = text(std::string(output.name), static_cast<int>(style.font_size), 1000, style.label))
            core->ui_header_width(output.name, metrics->m_size.x);
    if (auto metrics = text("Special workspaces", static_cast<int>(style.font_size), 1000, style.label))
        core->ui_header_width("Special workspaces", metrics->m_size.x);
    auto wins = windows();
    auto allWindows = windows(true);
    auto parked = core->parked_panels({outputs.data(), outputs.size()}, {allWindows.data(), allWindows.size()});
    for (auto &p : parked)
        outputs.push_back(std::move(p));
    backingMonitors.clear();
    for (auto &o : outputs)
        backingMonitors[o.id] = o.backing;
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
    for (auto id : pendingDrops)
        g_pEventLoopManager->removeDoLater(id);
    pendingDrops.clear();
    g_pHyprRenderer->m_renderPass.removeAllOfType("hypr2d-overview");
    surfaceListeners.clear();
    ui.reset();
    Pointer::Cursor::overrideController->unsetOverride(Pointer::Cursor::CURSOR_OVERRIDE_SPECIAL_ACTION);
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
    emptiedSpecials.clear();
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
                if (m->m_id == backing(cell.monitor))
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
static SP<Render::ITexture> text(const std::string &str, int fontSize, int width, uint32_t labelColor) {
    auto key = std::format("{}:{}:{}:{}", fontSize, width, labelColor, str);
    if (!textCache.contains(key))
        textCache[key] = g_pHyprRenderer->renderText(str, color(labelColor, 1), fontSize, false, std::string(core->ui_style().font), width);
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
    double outputScale;

  public:
    Pass(hypr2d::UIFrame f, Vector2D s, double scale) : frame(std::move(f)), size(s), outputScale(scale) {}
    std::vector<UP<IPassElement>> draw() override {
        std::vector<UP<IPassElement>> items;
        auto style = core->ui_style();
        auto rect = [&](const CBox &box, CHyprColor color) {
            items.push_back(makeUnique<CRectPassElement>(
                CRectPassElement::SRectData{.box = box, .color = color, .round = static_cast<int>(style.gap ? style.rounding : 0)}));
        };
        auto label = [&](const std::string &str, const CBox &box, float opacity = 1.f, uint32_t labelColor = 0) {
            auto tex = text(str, static_cast<int>(style.font_size * outputScale), std::clamp(static_cast<int>(box.w), 1, 1000),
                            labelColor ? labelColor : style.label);
            if (tex)
                items.push_back(makeUnique<CTexPassElement>(
                    CTexPassElement::SRenderData{.tex = tex,
                                                 .box = {box.x, box.y, std::min(box.w, tex->m_size.x), std::min(box.h, tex->m_size.y)},
                                                 .a = float(frame.alpha * opacity)}));
        };
        rect({0, 0, size.x, size.y}, color(style.background, frame.alpha));
        for (auto &c : frame.cells) {
            CBox box{std::round(c.rect.x), std::round(c.rect.y), std::ceil(c.rect.width), std::ceil(c.rect.height)};
            const auto borderColor = std::string(c.key) == std::string(frame.target) ? color(style.target, frame.alpha)
                                     : c.selected                                    ? color(style.current, frame.alpha)
                                     : c.hovered                                     ? color(style.hover, frame.alpha)
                                     : c.shown                                       ? color(style.current, frame.alpha)
                                               : color(c.colour ? c.colour : style.border, frame.alpha);
            rect(box, color(c.occupied ? style.cell : style.background, frame.alpha));
            if (auto it = cache.find(std::string(c.key)); c.occupied && it != cache.end())
                items.push_back(
                    makeUnique<CTexPassElement>(CTexPassElement::SRenderData{.tex = it->second->getTexture(),
                                                                             .box = box,
                                                                             .a = float(frame.alpha * (c.dimmed     ? 0.55
                                                                                                       : c.occupied ? 1.0
                                                                                                                    : 0.35)),
                                                                             .round = static_cast<int>(style.gap ? style.rounding : 0)}));
            // One physical pixel, drawn inside the cell: zero-gap tiles share
            // edges without the old expanded two-pixel halos.
            rect({box.x, box.y, box.w, 1}, borderColor);
            rect({box.x, box.y + box.h - 1, box.w, 1}, borderColor);
            rect({box.x, box.y, 1, box.h}, borderColor);
            rect({box.x + box.w - 1, box.y, 1, box.h}, borderColor);
            if (!c.label.empty()) {
                const auto caption = text(std::string(c.label), static_cast<int>(style.font_size * outputScale),
                                          std::clamp(static_cast<int>(box.w - 8), 1, 1000), style.label);
                if (caption) {
                    const auto h = std::min(box.h * 0.45, caption->m_size.y + 4);
                    rect({box.x + 1, box.y + 1, std::min(box.w - 2, caption->m_size.x + 8), h}, color(style.background, 0.8 * frame.alpha));
                    label(std::string(c.label), {box.x + 5, box.y + 3, std::max(1.0, box.w - 10), std::max(1.0, h - 4)},
                          c.occupied ? 1.f : 0.45f);
                }
            }
        }
        for (auto &l : frame.labels) {
            CBox b{l.rect.x, l.rect.y, l.rect.width, l.rect.height};
            if (l.band)
                rect(b, color(style.cell, frame.alpha));
            label(std::string(l.text), b, 1.f, l.colour);
        }
        if (frame.ghost.visible) {
            if (auto it = cache.find(std::string(frame.ghost.key)); it != cache.end())
                items.push_back(makeUnique<GhostPass>(it->second->getTexture(), frame.ghost, float(0.72 * frame.alpha)));
        }
        for (auto &w : frame.windows)
            if (w.target || std::string(w.id) == std::string(frame.refused)) {
                CBox b{w.rect.x, w.rect.y, w.rect.width, w.rect.height};
                const bool refused = std::string(w.id) == std::string(frame.refused);
                auto col = color(refused ? 0xF07178 : style.target, frame.alpha);
                rect({b.x, b.y, b.w, 2}, col);
                rect({b.x, b.y + b.h - 2, b.w, 2}, col);
                rect({b.x, b.y, 2, b.h}, col);
                rect({b.x + b.w - 2, b.y, 2, b.h}, col);
                const auto mode = refused ? std::string("locked") : std::string(frame.drop_mode);
                rect({b.x + 2, b.y + 2, std::min(b.w - 4, 72.0 * outputScale), 24.0 * outputScale},
                     color(style.background, 0.8 * frame.alpha));
                label(mode, {b.x + 4, b.y + 3, std::max(1.0, b.w - 8), 24.0 * outputScale}, 1.f, refused ? 0xF07178 : style.target);
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
    if (!Doctor::healthy()) {
        (*ui)->overview_refuse(drop.window, now());
        LOG(Log::ERR, "hypr2d: drop refused on inconsistent layout; run hyprctl hypr2d doctor");
        damage();
        return;
    }
    const auto sourceWorkspace = w->m_workspace; // Keep its monitor/space alive through both silent swap moves.
    auto source = key(sourceWorkspace);
    auto other = findWindow(std::string(drop.stack.empty() ? drop.swap : drop.stack));
    if ((!drop.stack.empty() || !drop.swap.empty()) &&
        (!other || other == w || !other->m_workspace || key(other->m_workspace) != std::string(drop.workspace)))
        return;
    { // Do not retain native groups across detach dispatchers: an empty group must expire.
        const auto oldGroup = w->grouping().group();
        const auto targetGroup = other ? other->grouping().group() : nullptr;
        const bool groupOperation = oldGroup || !drop.stack.empty() || targetGroup;
        if ((groupOperation && Desktop::windowState()->groupsLocked()) || (oldGroup && oldGroup->locked()) ||
            (targetGroup && (targetGroup->locked() || targetGroup->denied()))) {
            (*ui)->overview_refuse(other ? (drop.stack.empty() ? drop.swap : drop.stack) : drop.window, now());
            damage();
            return;
        }
    }
    // Public group detach/active dispatchers can focus another workspace. Keep
    // original active spaces alive while the Lua transaction restores them.
    std::vector<PHLWORKSPACE> activeSpaces;
    for (auto &m : State::monitorState()->monitors()) {
        if (m->m_activeWorkspace)
            activeSpaces.push_back(m->m_activeWorkspace);
        if (m->m_activeSpecialWorkspace)
            activeSpaces.push_back(m->m_activeSpecialWorkspace);
    }
    auto focus = Desktop::focusState()->monitor();
    const auto savedPointer = Pointer::mgr()->position();
    bool committed = false;
    Hyprutils::Utils::CScopeGuard restore([&] {
        if (focus)
            (void)Config::Actions::focusMonitor(focus);
        (void)Config::Actions::moveCursor(savedPointer);
        if (source.starts_with("special:") && !std::ranges::any_of(Desktop::windowState()->windows(), [&](const auto &win) {
                return win->mapped() && win->m_workspace && key(win->m_workspace) == source;
            }))
            emptiedSpecials.insert(source);
        mark(source);
        mark(std::string(drop.workspace));
        rebuild = true;
        damage();
        if (committed)
            IPC::Socket2::sock()->postEvent({"hypr2d", std::format("drop>>{},{}", std::string(drop.window), std::string(drop.workspace))});
    });
    PHLMONITOR mon;
    for (auto &m : State::monitorState()->monitors())
        if (m->m_id == backing(drop.monitor))
            mon = m;
    if (!mon)
        return;
    auto lua = dynamicPointerCast<Config::Lua::CConfigManager>(WP<Config::IConfigManager>(Config::mgr()));
    if (!lua)
        return;
    auto nativeDrop = drop;
    nativeDrop.monitor = static_cast<uint32_t>(mon->m_id);
    const auto script =
        core ? hypr2d::drop_script(nativeDrop, source, mon->m_position.x + drop.x, mon->m_position.y + drop.y) : rust::String{};
    if (auto error = lua->eval(std::string(script))) {
        (*ui)->overview_refuse(drop.window, now());
        LOG(Log::ERR, "hypr2d drop dispatcher: {}", *error);
        return;
    }
    core->extension_event("drop", drop.workspace, drop.window);
    committed = true;
}
std::string place(const hypr2d::ExtensionAction &action) {
    if (!Doctor::healthy())
        return "layout inconsistent; run hyprctl hypr2d doctor";
    auto w = findWindow(std::string(action.window));
    if (!w || !w->m_workspace)
        return "window is not mapped";
    PHLMONITOR mon;
    PHLWORKSPACE target;
    for (auto &ws : State::Workspace::state()->workspaces())
        if (key(ws.lock()) == std::string(action.workspace))
            target = ws.lock();
    for (auto &m : State::monitorState()->monitors())
        if (m->m_id == action.monitor)
            mon = m;
    if (action.monitor == UINT32_MAX && target)
        mon = target->m_monitor.lock();
    if (!mon)
        return "destination monitor/special is unavailable";
    if (!target && !std::string(action.workspace).starts_with("special:"))
        target = State::Workspace::state()->createNumbered(
            Workspace::SWorkspaceNumberedID{static_cast<uint32_t>(std::stoul(std::string(action.workspace)))}, mon);
    if (!target || target->m_monitor != mon)
        return "destination belongs to another monitor";
    auto source = key(w->m_workspace);
    if (!Config::Actions::moveToWorkspace(target, true, w))
        return "native placement failed";
    mark(source);
    mark(std::string(action.workspace));
    rebuild = active();
    core->extension_event("drop", action.workspace, action.window);
    IPC::Socket2::sock()->postEvent({"hypr2d", std::format("drop>>{},{}", std::string(action.window), std::string(action.workspace))});
    return "";
}
// phase 0=motion, 1=press, 2=release; Rust owns the drag state machine.
static void movePointer(Vector2D global, int phase) {
    if (!ui)
        return;
    pointer = global;
    if (auto kb = g_pSeatManager->m_keyboard.lock())
        (*ui)->overview_modifiers(xkb_state_mod_name_is_active(kb->m_xkbState, XKB_MOD_NAME_SHIFT, XKB_STATE_MODS_EFFECTIVE) > 0);
    else
        (*ui)->overview_modifiers(false);
    for (auto &m : State::monitorState()->monitors())
        if (CBox{m->m_position, m->m_size}.containsPoint(global)) {
            auto p = global - m->m_position;
            if (phase == 1)
                (*ui)->overview_press(m->m_size.x, m->m_size.y, p.x, p.y);
            else if (phase == 2) {
                auto drop = (*ui)->overview_release(m->m_size.x, m->m_size.y, p.x, p.y);
                pendingDrops.push_back(g_pEventLoopManager->doLater([drop] {
                    if (!ui || g_pHyprRenderer->m_renderData.currentFB)
                        return;
                    try {
                        applyDrop(drop);
                    } catch (const std::exception &error) {
                        LOG(Log::ERR, "hypr2d deferred drop: {}", error.what());
                    } catch (...) {
                        LOG(Log::ERR, "hypr2d deferred drop failed");
                    }
                }));
            } else
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
        std::vector<hypr2d::UIRect> viewports;
        for (auto &m : State::monitorState()->monitors())
            viewports.push_back({m->m_scale, 0, m->m_transformedSize.x, m->m_transformedSize.y});
        auto plans = (*ui)->overview_captures({viewports.data(), viewports.size()});
        std::set<std::string> visible;
        for (auto &cap : plans) {
            auto k = cap.special.empty() ? std::to_string(cap.workspace) : std::string(cap.special);
            visible.insert(k);
            auto old = cache.find(k);
            if (old == cache.end() || old->second->m_size != Vector2D{double(cap.capture_width), double(cap.capture_height)})
                mark(k);
        }
        std::erase_if(cache, [&](const auto &entry) { return !visible.contains(entry.first); });
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
                        if (m->m_id == backing(cell.monitor))
                            mon = m;
                    if (!mon)
                        break;
                    auto planned = std::ranges::find_if(plans, [&](const auto &p) {
                        return (p.special.empty() ? std::to_string(p.workspace) : std::string(p.special)) == k;
                    });
                    if (planned == plans.end())
                        break;
                    const auto &cap = *planned;
                    try {
                        if (!cache[k] || cache[k]->m_size != Vector2D{double(cap.capture_width), double(cap.capture_height)})
                            LOG(Log::INFO, "hypr2d capture {} framebuffer={}x{} drawn={}x{} physical pixels", k, cap.capture_width,
                                cap.capture_height, cap.width, cap.height);
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
                    } catch (const std::exception &error) {
                        LOG(Log::ERR, "hypr2d capture {}: {}", k, error.what());
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
            auto frame = (*ui)->overview_frame_scaled(m->m_transformedSize.x, m->m_transformedSize.y, m->m_scale, now());
            const bool more = frame.animating || !dirty.empty();
            g_pHyprRenderer->m_renderPass.add(makeUnique<Pass>(std::move(frame), m->m_transformedSize, m->m_scale));
            // pre-render requests can be consumed by this frame's commit.
            // During rendering scheduleFrame also sets pendingFrame, which the
            // native renderer schedules AFTER commit (essential on headless).
            if (more)
                m->scheduleFrame(Aquamarine::IOutput::AQ_SCHEDULE_DAMAGE);
            // Native cursors are queued before LAST_MOMENT. Queue the native
            // software cursor again above our overlay, respecting visibility/HW.
            if (g_pHyprRenderer->shouldRenderCursor())
                Pointer::mgr()->renderSoftwareCursorsFor(m, Time::steadyNow(), g_pHyprRenderer->m_renderData.damage);
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
        if (ui && (event.keycode == 42 || event.keycode == 54)) {
            (*ui)->overview_modifiers(event.state == WL_KEYBOARD_KEY_STATE_PRESSED);
            damage();
        }
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
    listeners.push_back(e.monitor.added.listen([changed](PHLMONITOR) { changed(); }));
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
    if (input == "overview invariants" && std::getenv("HYPR2D_TESTING")) {
        auto address = [](PHLWINDOW w) { return std::format("0x{:x}", reinterpret_cast<uintptr_t>(w.get())); };
        std::string result = std::format("{{\"focus\":\"{}\",\"order\":[", address(Desktop::focusState()->window()));
        bool first = true;
        for (auto &w : Desktop::windowState()->windows()) {
            if (!first)
                result += ",";
            first = false;
            result += std::format("\"{}\"", address(w));
        }
        result += "],\"outputs\":[";
        first = true;
        for (auto &m : State::monitorState()->monitors()) {
            if (!first)
                result += ",";
            first = false;
            result += std::format("{{\"name\":\"{}\",\"enabled\":{}}}", escapeJSONStrings(m->m_name), m->m_enabled);
        }
        result += "],\"groups\":[";
        first = true;
        for (auto &weak : Desktop::View::groups())
            if (auto g = weak.lock()) {
                if (!first)
                    result += ",";
                first = false;
                result += std::format("{{\"current\":\"{}\",\"locked\":{},\"denied\":{},\"members\":[", address(g->current()), g->locked(),
                                      g->denied());
                bool memberFirst = true;
                for (auto &w : g->windows()) {
                    if (!memberFirst)
                        result += ",";
                    memberFirst = false;
                    result += std::format("\"{}\"", address(w.lock()));
                }
                result += "]}";
            }
        return result + "]}";
    }
    if (input == "overview cursor") {
        auto m = Desktop::focusState()->monitor();
        const auto &image = Pointer::mgr()->currentCursorImage();
        return std::format("{{\"overview_open\":{},\"should_render\":{},\"texture\":{},\"image_width\":{},\"image_height\":{},\"software_"
                           "locked\":{},\"hardware_visible\":{}}}",
                           active(), g_pHyprRenderer->shouldRenderCursor(), bool(Pointer::mgr()->getCurrentCursorTexture()), image.size.x,
                           image.size.y, m && Pointer::mgr()->softwareLockedFor(m), m && Pointer::mgr()->hasVisibleHWCursor(m));
    }
    if (input == "overview state") {
        if (!ui)
            return deletedObjects ? "{\"open\":false,\"deleted_gl_objects\":true}" : "{\"open\":false,\"deleted_gl_objects\":false}";
        auto m = Desktop::focusState()->monitor();
        std::string images = "[";
        for (auto &[key, fb] : cache) {
            if (images.size() > 1)
                images += ",";
            auto frame = (*ui)->overview_frame_scaled(m ? m->m_transformedSize.x : 1000.0, m ? m->m_transformedSize.y : 700.0,
                                                      m ? m->m_scale : 1.0, now());
            auto cell = std::ranges::find_if(frame.cells, [&](const auto &c) { return std::string(c.key) == key; });
            const auto drawn = cell == frame.cells.end() ? Vector2D{} : Vector2D{std::ceil(cell->rect.width), std::ceil(cell->rect.height)};
            images += std::format("{{\"key\":\"{}\",\"width\":{},\"height\":{},\"drawn_width\":{},\"drawn_height\":{},\"direct\":true}}",
                                  escapeJSONStrings(key), fb->m_size.x, fb->m_size.y, drawn.x, drawn.y);
        }
        images += "]";
        return std::format("{{\"ui\":{},\"captures\":{},\"capture_ms\":{},\"pending\":{},\"images\":{}}}",
                           std::string((*ui)->overview_state_scaled(m ? m->m_transformedSize.x : 1000.0, m ? m->m_transformedSize.y : 700.0,
                                                                    m ? m->m_scale : 1.0, now())),
                           captures, captureMs, dirty.size(), images);
    }
    if (input == "overview close") {
        if (ui)
            (*ui)->overview_close(now());
        damage();
        return "ok";
    }
    if (input != "overview" && input != "overview open" && input != "overview parked")
        return "error: expected overview [open|close|state]";
    if (ui) {
        if (input == "overview parked") {
            (*ui)->overview_parked();
            damage();
            return "ok";
        }
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
    if (input == "overview parked")
        (*ui)->overview_parked();
    Pointer::Cursor::overrideController->setOverride("left_ptr", Pointer::Cursor::CURSOR_OVERRIDE_SPECIAL_ACTION);
    damage();
    IPC::Socket2::sock()->postEvent({"hypr2d", "overview>>open"});
    return "ok";
}
} // namespace OverviewUI
