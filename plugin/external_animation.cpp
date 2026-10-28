#include "external_animation.hpp"
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/plugins/HookSystem.hpp>
#include <hyprland/src/workspace/HLWorkspace.hpp>
#include <hyprutils/utils/ScopeGuard.hpp>
namespace ExternalAnimation {
static hypr2d::Core *core;
static CFunctionHook *hook;
using Change = void (*)(Monitor::CMonitor *, const PHLWORKSPACE &, bool, bool, bool);
static std::string last = "{}";
static void change(Monitor::CMonitor *m, const PHLWORKSPACE &target, bool internal, bool noMouse, bool noFocus) {
    const auto old = m->m_activeWorkspace;
    std::optional<std::string> saved;
    std::string style;
    bool changed = false;
    try {
        if (core && !internal && old && target && old != target && old->m_monitor == target->m_monitor &&
            target->m_monitor.lock().get() == m && old->numberedID() && target->numberedID()) {
            const auto slot = core->monitor_slot(m->m_name);
            style = std::string(core->grid_animation(
                {static_cast<uint32_t>(m->m_id), slot, m->m_name, *old->numberedID(), true, 0, 0, m->m_size.x, m->m_size.y},
                *target->numberedID()));
            if (!style.empty()) {
                saved = target->m_animationStyle;
                target->m_animationStyle = style;
                changed = true;
            }
        }
    } catch (...) {
        LOG(Log::ERR, "hypr2d external animation policy failed; retaining native animation");
    }
    Hyprutils::Utils::CScopeGuard restore([&] {
        if (changed)
            target->m_animationStyle = saved;
    });
    reinterpret_cast<Change>(hook->m_original)(m, target, internal, noMouse, noFocus);
    if (changed && std::getenv("HYPR2D_TESTING")) {
        const auto in = target->m_renderOffset->value(), out = old->m_renderOffset->value();
        last = std::format("{{\"style\":\"{}\",\"from\":{},\"to\":{},\"in\":[{},{}],\"out\":[{},{}]}}", style, *old->numberedID(),
                           *target->numberedID(), in.x, in.y, out.x, out.y);
    }
}
void init(hypr2d::Core *c, HANDLE handle) {
    core = c;
    const auto matches = HyprlandAPI::findFunctionsByName(handle, "changeWorkspace");
    for (const auto &match : matches)
        if (match.demangled.starts_with("Monitor::CMonitor::changeWorkspace(")) {
            hook = HyprlandAPI::createFunctionHook(handle, match.address, reinterpret_cast<void *>(&change));
            if (!hook || !hook->hook())
                throw std::runtime_error("hypr2d: cannot hook native workspace animation path");
            return;
        }
    throw std::runtime_error("hypr2d: pinned CMonitor::changeWorkspace symbol missing");
}
void shutdown() {
    if (hook)
        hook->unhook();
    hook = nullptr;
    core = nullptr;
    last = "{}";
}
std::string state() { return last; }
} // namespace ExternalAnimation
