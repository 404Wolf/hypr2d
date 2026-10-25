# An isolated config; never inherit the live instance signature for IPC.
test_dir=$(mktemp -d -t hypr2d-nested.XXXXXX)
trap 'rm -rf "$test_dir"' EXIT
cat > "$test_dir/hyprland.lua" <<'LUA'
hl.monitor({ output = "", mode = "1280x720@60", position = "auto", scale = 1 })
hl.config({ debug = { disable_logs = false, enable_stdout_logs = true }, misc = { disable_watchdog_warning = true, disable_hyprland_logo = true, disable_splash_rendering = true }, xwayland = { enabled = false } })
hl.plugin.load("@PLUGIN@")
if hl.plugin.hypr2d then
  hl.bind("SUPER + CONTROL + H", hl.plugin.hypr2d.move("left"))
  hl.plugin.hypr2d.configure("WAYLAND-1", {columns=8, rows=8, wrap=true, stride=10})
end
LUA
unset HYPRLAND_INSTANCE_SIGNATURE HYPR2D_TESTING
: "${WAYLAND_DISPLAY:?hypr2d nested requires a Wayland parent}"
# No seat backend means no DRM/input device access; Wayland fallback remains.
export LIBSEAT_BACKEND=none
if [[ "${1:-}" == "--overview-benchmark" ]]; then
  export HYPR2D_TESTING=1 HYPR2D_PROBE_BENCHMARK=1
  python3 @OVERVIEW@ "$test_dir/hyprland.lua" "@PLUGIN@" "@CLI@"
elif [[ "${1:-}" == "--overview-test" ]]; then
  export HYPR2D_TESTING=1
  python3 @OVERVIEW@ "$test_dir/hyprland.lua" "@PLUGIN@" "@CLI@"
elif [[ "${1:-}" == "--smoke-test" ]]; then
  export HYPR2D_TESTING=1
  python3 @SMOKE@ "$test_dir/hyprland.lua" "@PLUGIN@" "@CLI@" "@EXAMPLES@"
else
  Hyprland --config "$test_dir/hyprland.lua" "$@"
fi
