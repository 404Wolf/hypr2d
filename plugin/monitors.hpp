#pragma once
#include "hypr2d-core/src/lib.rs.h"
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <vector>
namespace MonitorPolicy {
std::vector<hypr2d::Monitor> snapshot();
void init(hypr2d::Core *, HANDLE);
void shutdown();
std::string command(const std::string &);
} // namespace MonitorPolicy
