# hypr2d

A Rust-backed Hyprland 0.56 plugin. The grid core supports independent per-monitor moves, carry, wrapping,
Lua binds and a JSON state endpoint.
See [DESIGN.md](DESIGN.md) for API findings and [GOALS.md](GOALS.md) for milestones.

```sh
nix build                    # result/lib/libhypr2d.so
nix flake check              # plugin build and Rust tests
nix run .#nested             # isolated, normal Wayland window
nix run .#nested -- --smoke-test  # preload, log, unload/reload, exit assertions
```

The nested runner uses the exact pinned compositor and its own temporary Lua
config. It does not load any user config. The smoke test addresses only its
child's instance (verified using the PID in the compositor lock file) and
terminates that child even on failure. It needs a working parent Wayland
session. Close the nested window normally when testing interactively.

Never load this development plugin into the live compositor. Plugin builds must
match the compositor revision; the flake pins both to
`e368c13c27a42a173b9e08fa0bf413f9f7073187`. A consuming flake can make
`inputs.hypr2d.inputs.hyprland.follows = "hyprland"` to share that dependency.

## Commands and grid configuration

Hyprland 0.56 at the pinned revision uses Lua dispatch, and its old
`addDispatcherV2` implementation is a stub. Use these exact forms:

```sh
hyprctl dispatch 'hl.plugin.hypr2d.move("left")'
hyprctl dispatch 'hl.plugin.hypr2d.move("down", { all = true })'
hyprctl dispatch 'hl.plugin.hypr2d.carry("right")'
hyprctl dispatch 'hl.plugin.hypr2d.goto_cell(2, 3)'
hyprctl hypr2d move left
hyprctl hypr2d goto 2 3 HEADLESS-2
hyprctl hypr2d state -j
```

The plugin functions `move`, `carry`, and `goto_cell` return callable dispatchers,
just like `hl.dsp.*`. `goto_cell(x, y, monitor)` optionally selects a monitor by
name or numeric ID (string). `move(dir, {all=true})` keeps the original monitor
focused. Commands use the active workspace immediately, including during an
unfinished animation. Invalid/out-of-grid workspaces require an explicit goto.
Special workspaces are excluded from grid numbering.

`hl.plugin.hypr2d.configure("eDP-1", { columns=8, rows=8, wrap=true, stride=10 })`
is an immediate configuration call, returning `(success, message)`; it can be
placed after `hl.plugin.load(...)` inside `if hl.plugin.hypr2d then ... end`.
The loader queues plugins and reparses config after loading them, so this guard
avoids an undefined namespace on the first parse. Names may be configured before output
creation. Stride is global (the last configured stride applies to all grids),
so monitor residues never collide. Dimensions and wrapping are per monitor.
`hyprctl hypr2d configure eDP-1 8 8 false 10` changes the runtime configuration.
The default workspace ID is exactly `10 * (y * 8 + x) + monitor_id + 1`.

## Binds

Place these after loading the plugin. No Home Manager files are modified by
this repository. These mirror the existing binds in DESIGN.md:

```lua
if hl.plugin.hypr2d then
hl.bind("SUPER + CONTROL + H", hl.plugin.hypr2d.move("left"))
hl.bind("SUPER + CONTROL + J", hl.plugin.hypr2d.move("down"))
hl.bind("SUPER + CONTROL + K", hl.plugin.hypr2d.move("up"))
hl.bind("SUPER + CONTROL + L", hl.plugin.hypr2d.move("right"))
hl.bind("SUPER + SHIFT + H", hl.plugin.hypr2d.carry("left"))
hl.bind("SUPER + SHIFT + J", hl.plugin.hypr2d.carry("down"))
hl.bind("SUPER + SHIFT + K", hl.plugin.hypr2d.carry("up"))
hl.bind("SUPER + SHIFT + L", hl.plugin.hypr2d.carry("right"))
hl.bind("SUPER + ALT + J", function()
    hl.dispatch(hl.plugin.hypr2d.move("down", { all = true }))
    hl.dispatch(hl.dsp.workspace.toggle_special(""))
end)
hl.bind("SUPER + ALT + K", function()
    hl.dispatch(hl.plugin.hypr2d.move("up", { all = true }))
    hl.dispatch(hl.dsp.workspace.toggle_special(""))
end)
-- SUPER + TAB / overview is reserved for M4; do not bind it yet.
end
```

## Validation

`nix develop --command cargo test --workspace` runs the pure Rust tests.
`nix run .#nested -- --smoke-test` tests plugin load/reload, grid operations,
rapid moves, a second headless output, socket2 events and a real carry client.
`nix build .#integration` runs the same suite in a NixOS VM with a virtual GPU
and a plugin-free Hyprland parent, so CI does not need a live desktop. KVM is required for the VM.
`nix flake check` includes that integration test, Rust tests/lints/formatting,
C++ formatting and the plugin build. The upstream-main build is non-blocking.
