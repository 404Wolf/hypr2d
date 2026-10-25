#include "monitors.hpp"
#include <hyprland/src/config/shared/actions/ConfigActions.hpp>
#include <hyprland/src/desktop/state/FocusState.hpp>
#include <hyprland/src/desktop/state/WindowState.hpp>
#include <hyprland/src/desktop/view/window/Window.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/ipc/s2/S2.hpp>
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/state/MonitorState.hpp>
#include <hyprland/src/state/workspace/State.hpp>
#include <hyprland/src/workspace/HLWorkspace.hpp>
#include <map>
#include <optional>
#include <sstream>
namespace MonitorPolicy {
static hypr2d::Core *core;
static HANDLE handle;
static CHyprSignalListener added, removed, before, active;
static std::optional<hypr2d::UnplugPlan> pending;
static std::map<std::string, std::array<std::string, 3>> testIdentities;
static std::vector<hypr2d::Monitor> snapshot() {
    std::vector<hypr2d::Monitor> result;
    for (auto &m : State::monitorState()->monitors())
        if (m->m_enabled && m->m_name != "FALLBACK")
            result.push_back({static_cast<uint32_t>(m->m_id), core->monitor_slot(m->m_name), m->m_name,
                              m->m_activeWorkspace && m->m_activeWorkspace->numberedID() ? *m->m_activeWorkspace->numberedID() : 0,
                              m == Desktop::focusState()->monitor(), 0, 0, m->m_size.x, m->m_size.y});
    return result;
}
static void update() {
    std::vector<hypr2d::MonitorIdentity> identities;
    for (auto &m : State::monitorState()->monitors())
        if (m->m_enabled && m->m_name != "FALLBACK") {
            auto identity = std::array<std::string, 3>{m->m_output->make, m->m_output->model, m->m_output->serial};
            if (auto test = testIdentities.find(m->m_name); test != testIdentities.end())
                identity = test->second;
            identities.push_back({static_cast<uint32_t>(m->m_id), m->m_name, identity[0], identity[1], identity[2]});
        }
    auto error = core->slots_update({identities.data(), identities.size()});
    if (!error.empty())
        LOG(Log::ERR, "hypr2d slots: {}", std::string(error));
}
static PHLMONITOR monitor(uint32_t id) {
    for (auto &m : State::monitorState()->monitors())
        if (m->m_id == id && m->m_enabled)
            return m;
    return nullptr;
}
static PHLWORKSPACE workspace(uint32_t id, PHLMONITOR m) {
    if (id == 0)
        return nullptr;
    for (auto &w : State::Workspace::state()->workspaces())
        if (w->numberedID() && *w->numberedID() == id)
            return w.lock();
    return State::Workspace::state()->createNumbered(Workspace::SWorkspaceNumberedID{id}, m);
}
static void restore(PHLMONITOR m) {
    update();
    auto ms = snapshot();
    auto native = std::ranges::find_if(ms, [&](const auto &s) { return s.id == m->m_id; });
    if (native == ms.end())
        return;
    const auto focus = Desktop::focusState()->monitor();
    std::vector<PHLWORKSPACE> owned;
    for (auto &w : State::Workspace::state()->workspaces())
        if (w->numberedID() && core->owns_workspace(*native, *w->numberedID()))
            owned.push_back(w.lock());
    for (auto &w : owned)
        if (w->m_monitor != m)
            (void)Config::Actions::moveToMonitor(w, m);
    auto target = workspace(core->restore_workspace(*native), m);
    if (target && target->m_monitor == m)
        m->changeWorkspace(target, false, true, focus != m);
    if (focus && focus->m_enabled)
        (void)Config::Actions::focusMonitor(focus);
    IPC::Socket2::sock()->postEvent({"hypr2d", "monitors"});
}
void init(hypr2d::Core *c, HANDLE h) {
    core = c;
    handle = h;
    auto error = core->slots_init();
    if (!error.empty())
        throw std::runtime_error("hypr2d slots: " + std::string(error));
    update();
    // Normalize only outputs whose active workspace is outside their own slot.
    for (auto &m : State::monitorState()->monitors())
        restore(m);
    active = Event::bus()->m_events.workspace.active.listen([](PHLWORKSPACE) {
        auto ms = snapshot();
        core->monitor_observe({ms.data(), ms.size()});
    });
    added = Event::bus()->m_events.monitor.added.listen([](PHLMONITOR m) { restore(m); });
    before = Event::bus()->m_events.monitor.preRemoved.listen([](PHLMONITOR m) {
        if (!m->m_enabled)
            return;
        auto ms = snapshot();
        std::vector<hypr2d::Window> windows;
        for (auto &w : Desktop::windowState()->windows())
            if (w->mapped() && w->m_monitor)
                windows.push_back({w->m_workspace && w->m_workspace->numberedID() ? *w->m_workspace->numberedID() : 0,
                                   static_cast<uint32_t>(w->m_monitor->m_id), "", "", "", false});
        pending = core->unplug(m->m_id, {ms.data(), ms.size()}, {windows.data(), windows.size()});
    });
    removed = Event::bus()->m_events.monitor.removed.listen([](PHLMONITOR) {
        update();
        if (!pending)
            return;
        for (auto &move : pending->workspaces)
            if (auto m = monitor(move.monitor)) {
                auto w = workspace(move.workspace, m);
                if (w && w->m_monitor == m)
                    m->changeWorkspace(w, false, true, move.monitor != pending->focus);
            }
        if (auto m = monitor(pending->focus))
            (void)Config::Actions::focusMonitor(m);
        HyprlandAPI::addNotification(handle, std::string(pending->notification), CHyprColor(0.51, 0.67, 1, 1), 5000);
        pending.reset();
        IPC::Socket2::sock()->postEvent({"hypr2d", "monitors"});
    });
}
std::string command(const std::string &input) {
    if (!std::getenv("HYPR2D_TESTING"))
        return "error: test identity injection disabled";
    std::istringstream stream(input);
    std::string prefix, op, name, make, model, serial;
    stream >> prefix >> op >> name >> make >> model >> serial;
    if (op != "identity" || serial.empty())
        return "error: expected monitor-test identity NAME MAKE MODEL SERIAL";
    testIdentities[name] = {make, model, serial};
    update();
    return "ok";
}
void shutdown() {
    active.reset();
    added.reset();
    before.reset();
    removed.reset();
    pending.reset();
    testIdentities.clear();
    core = nullptr;
}
} // namespace MonitorPolicy
