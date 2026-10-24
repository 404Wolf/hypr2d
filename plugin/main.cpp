#include <hyprland/src/plugins/PluginAPI.hpp>
#include <hyprland/src/debug/log/Logger.hpp>
#include "hypr2d-core/src/lib.rs.h"

APICALL EXPORT std::string PLUGIN_API_VERSION() {
    return HYPRLAND_API_VERSION;
}

APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    if (HyprlandAPI::getHyprlandVersion(handle).hash != GIT_COMMIT_HASH)
        throw std::runtime_error("hypr2d: compositor build does not match plugin");
    const auto result = hypr2d::spike_round_trip(41);
    LOG(Log::INFO, "hypr2d M0: C++ -> Rust -> C++: 41 -> {}", result);
    if (result != 42)
        throw std::runtime_error("hypr2d: Rust bridge self-test failed");
    return {"hypr2d", "2D workspace grid (M0 spike)", "Wolf Mermelstein", "0.1.0"};
}

APICALL EXPORT void PLUGIN_EXIT() {}
