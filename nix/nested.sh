# An isolated config; never inherit the live instance signature for IPC.
test_dir=$(mktemp -d -t hypr2d-nested.XXXXXX)
trap 'rm -rf "$test_dir"' EXIT
cat > "$test_dir/hyprland.lua" <<'LUA'
hl.monitor({ output = "", mode = "1280x720@60", position = "auto", scale = 1 })
hl.config({ debug = { disable_logs = false }, misc = { disable_hyprland_logo = true, disable_splash_rendering = true }, xwayland = { enabled = false } })
hl.plugin.load("@PLUGIN@")
LUA
unset HYPRLAND_INSTANCE_SIGNATURE
export AQ_BACKEND=wayland
if [[ "${1:-}" == "--smoke-test" ]]; then
  python3 @SMOKE@ "$test_dir/hyprland.lua" "@PLUGIN@"
else
  Hyprland --config "$test_dir/hyprland.lua" "$@"
fi
