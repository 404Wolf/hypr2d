#pragma once
#include "hypr2d-core/src/lib.rs.h"
#include <hyprland/src/render/Framebuffer.hpp>
#include <string>
#include <vector>
namespace OverviewProbe {
SP<Render::IFramebuffer> capture(const hypr2d::ProbeCell &cell, SP<Render::IFramebuffer> fb, bool finish);
void init();
void close();
void shutdown();
std::string command(hypr2d::Core &core, const std::vector<hypr2d::Monitor> &monitors, const std::string &input);
} // namespace OverviewProbe
