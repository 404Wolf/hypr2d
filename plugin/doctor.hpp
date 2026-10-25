#pragma once
#include <string>
namespace Doctor {
std::string report();
std::string repair();
std::string test(const std::string &input);
bool healthy();
void shutdown();
} // namespace Doctor
