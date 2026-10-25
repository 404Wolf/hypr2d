#include "doctor.hpp"
#include "hypr2d-core/src/lib.rs.h"
#include "ui.hpp"
#include <hyprland/src/config/lua/ConfigManager.hpp>
#include <hyprland/src/config/shared/actions/ConfigActions.hpp>
#include <hyprland/src/desktop/state/FocusState.hpp>
#include <hyprland/src/desktop/state/WindowState.hpp>
#include <hyprland/src/desktop/view/Group.hpp>
#include <hyprland/src/desktop/view/window/WindowGroupMembership.hpp>
#include <hyprland/src/layout/space/Space.hpp>
#include <hyprland/src/layout/target/WindowGroupTarget.hpp>
#include <hyprland/src/layout/target/WindowTarget.hpp>
#include <hyprland/src/managers/eventLoop/EventLoopManager.hpp>
#include <hyprland/src/managers/fullscreen/FullscreenController.hpp>
#include <hyprland/src/pointer/PointerManager.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/state/workspace/State.hpp>
#include <hyprutils/utils/ScopeGuard.hpp>
#include <sstream>

namespace Doctor {
static uint64_t pending = 0;
static std::string last = "null";
static std::string key(PHLWORKSPACE ws) { return !ws ? "" : ws->numberedID() ? std::to_string(*ws->numberedID()) : ws->addressableName(); }
static std::string address(auto p) { return p ? std::format("0x{:x}", reinterpret_cast<uintptr_t>(p.get())) : std::string{}; }
static PHLWINDOW find(const std::string &id) {
    for (auto &w : Desktop::windowState()->windows())
        if (w->mapped() && address(w) == id)
            return w;
    return nullptr;
}
struct Snapshot {
    std::vector<hypr2d::DoctorWorkspace> spaces;
    std::vector<hypr2d::DoctorWindow> windows;
};
static Snapshot snapshot() {
    Snapshot result;
    const bool unsafeCount = std::ranges::any_of(Desktop::windowState()->windows(), [](auto &w) {
        const auto g = w->grouping().group();
        return g && g->getCurrentIdx() >= g->windows().size();
    });
    for (auto &weak : State::Workspace::state()->workspaces()) {
        auto ws = weak.lock();
        if (!ws)
            continue;
        hypr2d::DoctorWorkspace row{
            key(ws),
            static_cast<uint32_t>(unsafeCount
                                      ? (ws->space() ? std::ranges::count_if(ws->space()->targets(), [](auto &t) { return !!t; }) : 0)
                                      : ws->getWindowCount()),
            ws->space() && ws->space()->algorithm() && ws->m_monitor && ws->m_monitor->m_enabled,
            {}};
        if (ws->space())
            for (auto &t : ws->space()->targets())
                row.targets.push_back(address(t));
        result.spaces.push_back(std::move(row));
    }
    for (auto &w : Desktop::windowState()->windows()) {
        if (!w->mapped())
            continue;
        const auto target = w->layoutTarget();
        const auto member = w->windowTarget();
        hypr2d::DoctorWindow row{address(w),
                                 key(w->m_workspace),
                                 target ? key(target->workspace()) : "",
                                 member ? key(member->workspace()) : "",
                                 address(target),
                                 target && !!target->space(),
                                 member && !!member->space(),
                                 Desktop::windowState()->groupsLocked(),
                                 "",
                                 {},
                                 ""};
        if (auto g = w->grouping().group()) {
            row.group = address(g);
            row.locked = row.locked || g->locked();
            for (auto &m : g->windows())
                row.members.push_back(address(m));
            // Do not call current() on an already-invalid index.
            if (g->getCurrentIdx() < g->windows().size())
                row.current = address(g->current());
        }
        result.windows.push_back(std::move(row));
    }
    return result;
}
bool healthy() {
    const auto s = snapshot();
    return hypr2d::doctor_ok({s.spaces.data(), s.spaces.size()}, {s.windows.data(), s.windows.size()});
}
std::string report() {
    const auto s = snapshot();
    std::string text(hypr2d::doctor({s.spaces.data(), s.spaces.size()}, {s.windows.data(), s.windows.size()}));
    if (!text.empty() && text.back() == '}') {
        text.pop_back();
        text += ",\"repair\":" + last + "}";
    }
    return text;
}
static void run() {
    if (g_pHyprRenderer->m_renderData.currentFB)
        throw std::runtime_error("repair refused during render");
    const auto s = snapshot();
    auto plan = hypr2d::doctor_plan({s.spaces.data(), s.spaces.size()}, {s.windows.data(), s.windows.size()});
    if (!plan.error.empty()) {
        last = std::string(hypr2d::doctor_result("refused", plan.error, {}, {}));
        return;
    }
    struct Presentation {
        PHLWINDOW window;
        Fullscreen::SFullscreenMode fullscreen;
        bool layoutAware;
        bool floating;
        CBox box;
    };
    std::vector<Presentation> presentation;
    for (auto &id : plan.rebuild) {
        auto w = find(std::string(id));
        if (w)
            presentation.push_back({w, Fullscreen::controller()->getFullscreenModes(w), Fullscreen::controller()->layoutManagedFS(w),
                                    w->isFloating(), w->layoutTarget()->position()});
    }
    for (auto &id : plan.dissolve) {
        auto w = find(std::string(id));
        if (w && std::ranges::none_of(presentation, [&](auto &p) { return p.window == w; }))
            presentation.push_back({w, Fullscreen::controller()->getFullscreenModes(w), Fullscreen::controller()->layoutManagedFS(w),
                                    w->isFloating(), w->layoutTarget()->position()});
    }
    std::vector<PHLWINDOW> windows;
    std::vector<PHLWORKSPACE> held;
    for (auto &weak : State::Workspace::state()->workspaces())
        if (weak)
            held.push_back(weak.lock());
    for (auto &id : plan.rebuild) {
        auto w = find(std::string(id));
        if (!w || !w->m_workspace || !w->m_workspace->space())
            throw std::runtime_error("repair snapshot changed");
        windows.push_back(w);
    }
    const auto pointer = Pointer::mgr()->position();
    Hyprutils::Utils::CScopeGuard restorePointer([&] { (void)Config::Actions::moveCursor(pointer); });
    OverviewUI::command("overview close");
    // Only orphan spaces have no layout owner. Quarantine their algorithms
    // before detaching anything: their callbacks dereference the dead workspace.
    for (auto &w : windows) {
        for (const auto &t : {w->layoutTarget(), SP<Layout::ITarget>(w->windowTarget())})
            if (t && t->space() && !t->space()->workspace())
                t->space()->setAlgorithmProvider(nullptr);
    }
    for (auto &w : windows) {
        auto target = w->layoutTarget();
        const auto destination = w->m_workspace;
        // Seed ALL group ghosts first. assignToSpace invokes onUpdateSpace
        // before CWindowGroupTarget::assignToSpace calls updateWorkspace.
        if (auto g = w->grouping().group())
            for (const auto &m : g->windows())
                m->windowTarget()->setSpaceGhost(destination->space());
        // Force a genuine reinsert, then clear any remaining misindexed
        // references. The orphan's algorithm was quarantined above.
        target->assignToSpace(nullptr);
        for (auto &ws : held)
            if (ws->space() && std::ranges::any_of(ws->space()->targets(), [&](auto &t) { return t == target; }))
                ws->space()->remove(target);
        target->assignToSpace(destination->space());
    }
    if (!healthy())
        throw std::runtime_error("layout recovery verification failed; group operations skipped");
    // No strong group references may span this dispatcher (focus_removed_window=false).
    auto lua = dynamicPointerCast<Config::Lua::CConfigManager>(WP<Config::IConfigManager>(Config::mgr()));
    if (!lua)
        throw std::runtime_error("Lua dispatcher unavailable");
    const auto script = hypr2d::doctor_repair_script({plan.dissolve.data(), plan.dissolve.size()});
    if (auto error = lua->eval(std::string(script)))
        throw std::runtime_error(*error);
    for (const auto &p : presentation) {
        Fullscreen::controller()->setFullscreenMode(p.window, p.fullscreen.internal, p.fullscreen.client, p.layoutAware,
                                                    Fullscreen::FULLSCREEN_MUTATION_TRANSFER);
        if (p.floating && p.fullscreen.internal == Fullscreen::FSMODE_NONE)
            p.window->layoutTarget()->setPositionGlobal(p.box);
        g_pHyprRenderer->damageWindow(p.window);
    }
    if (!healthy())
        throw std::runtime_error("post-repair verification failed");
    last = std::string(
        hypr2d::doctor_result("complete", "", {plan.rebuild.data(), plan.rebuild.size()}, {plan.dissolve.data(), plan.dissolve.size()}));
}
std::string repair() {
    if (pending)
        return "{\"queued\":true}";
    last = std::string(hypr2d::doctor_result("queued", "", {}, {}));
    pending = g_pEventLoopManager->doLater([] {
        pending = 0;
        try {
            run();
        } catch (const std::exception &e) {
            last = std::string(hypr2d::doctor_result("failed", e.what(), {}, {}));
        } catch (...) {
            last = std::string(hypr2d::doctor_result("failed", "native repair failed", {}, {}));
        }
    });
    return "{\"queued\":true}";
}
void shutdown() {
    if (pending)
        g_pEventLoopManager->removeDoLater(pending);
    pending = 0;
}
std::string test(const std::string &input) {
    if (!std::getenv("HYPR2D_TESTING"))
        return "error: test-only command";
    std::istringstream words(input);
    std::string prefix, op, first, second;
    words >> prefix >> op >> first >> second;
    if (op == "missing-script") {
        hypr2d::UIDrop drop{};
        drop.window = first;
        drop.stack = second;
        drop.workspace = "special:3";
        auto lua = dynamicPointerCast<Config::Lua::CConfigManager>(WP<Config::IConfigManager>(Config::mgr()));
        const auto script = hypr2d::drop_script(drop, "1", 0, 0);
        if (auto error = lua->eval(std::string(script)))
            return "error: " + *error;
        return "error: expected missing lookup to assert";
    }
    auto w = find(first);
    if (!w)
        return "error: mapped test window required";
    if (op == "swap") {
        auto other = find(second);
        if (!other)
            return "error: mapped swap target required";
        return Config::Actions::swapWith(other, w) ? "ok" : "error: legacy swap failed";
    }
    if (op == "null-target")
        w->layoutTarget()->assignToSpace(nullptr);
    else if (op == "null-ghost" && w->grouping().group())
        w->windowTarget()->setSpaceGhost(nullptr);
    else if (op == "missing")
        w->m_workspace->space()->remove(w->layoutTarget());
    else
        return "error: unknown doctor fixture";
    return "ok";
}
} // namespace Doctor
