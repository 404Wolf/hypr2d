#pragma once
#include "hypr2d-core/src/lib.rs.h"
#include <string>
#include <vector>
namespace OverviewProbe {
void init();
void close();
void shutdown();
std::string command(hypr2d::Core &core, const std::vector<hypr2d::Monitor> &monitors, const std::string &input);
} // namespace OverviewProbe
