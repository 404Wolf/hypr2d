#pragma once
#include "hypr2d-core/src/lib.rs.h"
#include <hyprland/src/plugins/PluginAPI.hpp>
namespace ExternalAnimation {
void init(hypr2d::Core *, HANDLE);
void shutdown();
std::string state();
} // namespace ExternalAnimation
