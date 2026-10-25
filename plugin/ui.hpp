#pragma once
#include "hypr2d-core/src/lib.rs.h"
#include <string>
namespace OverviewUI {
void init(hypr2d::Core *core);
void shutdown();
bool active();
std::string command(const std::string &input);
} // namespace OverviewUI
