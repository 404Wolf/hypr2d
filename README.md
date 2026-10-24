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

## Tracked swipe

The plugin consumes three-finger swipes before Hyprland's built-in gesture
handlers. Axis locks after 5 logical pixels; the current and neighbouring
workspaces follow the fingers 1:1. Swipe up means grid down, matching the current
natural direction. Release commits at 120 px or 0.8 px/ms in the swipe direction;
a pause removes stale velocity. Cancellation or a short slow gesture springs
back. Non-wrapping edges resist further travel.

Optional Lua configuration (inside the same namespace guard as the binds):

```lua
if hl.plugin.hypr2d then
    local ok, message = hl.plugin.hypr2d.swipe_config({
        fingers = 3, distance = 120, velocity = 0.8,
        axis_lock = 5, natural = true,
    })
    assert(ok, message)
end
```

Other finger counts and swipes while a special workspace is open remain with
native handlers. A keyboard grid move interrupts a swipe. Spring motion advances on render callbacks, with a one-shot Rust-defined
settlement deadline when output frames pause. There is no polling loop. Unload, monitor removal and
external workspace changes release all plugin-owned rendering offsets.

The nested test path emits synthetic cancellable events through the production
listeners. `_test_swipe` exists only with `HYPR2D_TESTING=1`, set by
`nix run .#nested -- --smoke-test`; it never injects input into the parent.

### Real trackpad hand-test (pending, to be scheduled by the user)

No live plugin loading was performed. After arranging a controlled real-input
test, check each of these:

1. Three fingers: slow horizontal and vertical travel follows fingers at 1:1;
   swipe up selects grid down and all four directions reach the expected cell.
2. Diagonal onset locks the dominant axis after the configured distance;
   perpendicular motion does not change axis. Reverse through the starting
   point and confirm the opposite neighbour appears without a jump or ghost.
3. Release below the distance threshold slowly: bounce. Release past it: commit.
   A short fast flick commits in its travel direction; flick then pause bounces.
4. Cancel a gesture (including lifting fingers unexpectedly): bounce with no
   stuck offset. Repeated gestures during spring settling start from the cell
   already committed. Check both natural=true and natural=false.
5. With wrap enabled, cross every grid boundary. With wrap disabled, test all
   four edges: bounded resistance, no invalid workspace, smooth return.
6. Two-, four-, and five-finger native gestures keep working with default
   three-finger configuration. An open special workspace keeps native handling.
   Replace the old three-finger exec gesture when adopting the plugin; verify
   no duplicate workspace action is triggered.
7. Check tiled, floating and fullscreen windows, fractional scaling and mixed
   monitor sizes: no seams, stale decorations, wrong focus or stuck layer alpha.
8. During tracking and settling, use a grid keybind, switch workspace externally,
   unplug the gesture's monitor, and unload/reload the plugin. No forced-visible
   neighbour, orphan callback, crash, or stray animation may remain.
