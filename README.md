# hypr2d

A Rust-backed Hyprland 0.56 plugin. The grid core supports independent per-monitor moves, carry, wrapping,
Lua binds and a JSON state endpoint.
See [DESIGN.md](DESIGN.md) for API findings and [GOALS.md](GOALS.md) for milestones.

```sh
nix build                    # result/lib/libhypr2d.so + result/bin/hypr2d
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
hl.unbind("SUPER + TAB") -- replace an existing group-next bind
hl.bind("SUPER + TAB", hl.plugin.hypr2d.overview())
end
```

Suggested overview bind (Lua, Hyprland 0.56):

```lua
hl.unbind("SUPER + TAB") -- replace an existing group-next bind
hl.bind("SUPER + TAB", hl.plugin.hypr2d.overview())
```

The callable toggles the overview. Inside it, use hjkl/arrows, Tab/Shift-Tab to
switch monitor panels and the special strip, Enter to activate, or Esc to close.
Click a cell to activate it. `hyprctl hypr2d overview` is the equivalent IPC
command; `overview open`, `overview close` and `overview state` are also available.
All monitor grids appear together in their physical arrangement on each output.
Specials are labelled; empty ones are dim and currently shown ones highlighted.

Drag window content to another cell, another monitor's grid, or a special.
Drop on another window to swap their native layout slots; drop on empty space
or a cell's top caption edge to move without swapping. Floating windows retain
the grabbed point at the pointer's destination, including near output edges.
A translucent ghost and gold destination/window outline show the pending drop.
Esc cancels a drag first; a second Esc closes the overview. Releasing a cancelled
drag does nothing. Fullscreen swaps preserve each window's fullscreen mode.

## Overview appearance

Fit is on by default: each monitor shows the used/current bounding box plus
one cell on every side (clipped to the grid), with at least 3×3 cells.
Keyboard selection expands the box when necessary. Press `a` to toggle all
cells without changing workspace numbering. Panels retain physical monitor
positions and each thumbnail retains its source monitor's aspect ratio.
The combined layout fits the output width/height and is centered vertically,
including the grids and special strip. Cell captions use the main app class's
short name (for example `firefox` or `kitty`), followed by `+N` for additional
windows. Empty grid cells have no caption: a thin base02 outline surrounds the
base00 background. Occupied cells retain their thumbnails and configured fill.

Configure appearance with one atomic JSON object (omitted fields use defaults):

```sh
hyprctl hypr2d overview-config '{"fit":true,"background":"#000000","cell":"#303030","border":"#353535","hover":"#4A4A4A","current":"#82AAFF","target":"#FFCB6B","label":"#EEFFFF","font":"sans-serif","font_size":16,"rounding":3}'
```

Colours use `#RRGGBB`; font size is 10–40 output pixels and rounding is 0–4 px.
These defaults match base16 OneDark Extra Dark. Runtime changes apply while
open. Grid capture size stays at 160 pixels wide, independent of layout size.

## Extension clients

The plugin serves a versioned, connection-scoped JSON-lines protocol at
`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/hypr2d.sock` (0600).
Clients can annotate cells/specials/windows, reserve titled rows, place windows,
and subscribe to activation/drop callbacks on their rows. Disconnect removes
all session state. Overview and Waybar update on events, without polling.
See [the protocol](docs/protocol.md) for schema, limits, ownership and examples;
[`hypr2d-client`](client/src/lib.rs) supplies the typed Rust client.

## Home Manager module

This repository exports `homeManagerModules.default`. Add it to your own
configuration; nothing here edits or activates a Home Manager checkout.
Keep the plugin ABI aligned with the compositor:

```nix
inputs.hypr2d.url = "path:/home/wolf/Documents/hypr2d"; # or your chosen remote
inputs.hypr2d.inputs.hyprland.follows = "hyprland";
```

Import `inputs.hypr2d.homeManagerModules.default` in your Home Manager modules,
then enable `programs.hypr2d`. See [the complete option example](examples/home-manager.nix).
Options cover `grid.stride`, per-output `grids`, `overview.fit`/`overview.bind`,
`theme`, `gestures`, and `waybar`. The module loads the plugin through Home
Manager's Lua plugin support, appends configuration Lua, installs the CLI and
writes `~/.config/hypr/hypr2d.json` as a readable record of generated settings.
The plugin is configured by the generated Lua; it does not watch that JSON file.
Lua `configType` is required. The module defaults the Hyprland package to the
same pinned flake input; an explicit package override must preserve that ABI. Unlisted outputs retain an 8×8 wrapping grid with
the shared stride. Output names can be configured before hotplug.

`waybar.enable` adds `custom/hypr2d` to the chosen named bar and position,
adds absolute CLI paths, event streaming and optional black OneDark CSS.
Choose `waybar.bar` to match your existing named settings entry; configurations
using Waybar's list form should convert to named entries before enabling this
integration. `waybar.monitor = null` follows focus; otherwise it targets the
named output. `waybar.placement = "start"` prepends it before existing modules;
the default `"end"` appends it. `radiusX` and `radiusY` default to 4 and 1.
No interval or refresh signal is installed.

`gestures.enable = false` stops claiming swipes. When wiring this into your
configuration, remove conflicting native/execGesture swipes and switch the
existing movement binds using §Binds. `overview.bind = true` replaces existing SUPER+TAB bindings with the overview.
Register manual replacement binds after your existing bindings; duplicate chords
can dispatch both the overview and `group.next`.
Other existing binds, your old workspace script and your Home Manager files
are left for you to migrate.

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

## Waybar

The built package includes `bin/hypr2d` (also `nix build .#cli`). Run
`hypr2d state`, `hypr2d move left`, `hypr2d carry down`, or `hypr2d goto 2 3`.
The CLI talks directly to Hyprland's Unix socket; it starts no subprocesses.
It uses `HYPRLAND_INSTANCE_SIGNATURE` and `XDG_RUNTIME_DIR`, or accepts an exact
`--instance SIGNATURE`. Tests always supply their own nested signature.

`hypr2d waybar [--monitor NAME] [--radius-x 0..4] [--radius-y 0..4]`
emits newline-delimited JSON. Defaults are radius X=4, Y=1: **9×3 dots**, centered
on the current cell and wrapping with the grid (including repeated edge cells
when the view is wider than the grid). `--radius N` remains a shorthand for both
axes. Without `--monitor`, it follows the focused monitor.

The stream subscribes to socket2 before its initial snapshot. Native window
open/close/move, workspace create/destroy/move/change, focus, fullscreen, urgency,
monitor, title and hypr2d events trigger a refresh, coalesced to one per 16 ms.
It blocks when idle and suppresses identical output. Plugin unload/reload is
handled without restarting the stream. No polling interval is used.

Use [examples/waybar.jsonc](examples/waybar.jsonc) and
[examples/waybar.css](examples/waybar.css), also installed in
`share/hypr2d/`. The exact module configuration is:

```json
"custom/hypr2d": {
  "exec": "hypr2d waybar --radius-x 4 --radius-y 1",
  "return-type": "json",
  "format": "{text}",
  "escape": false,
  "exec-on-event": false,
  "on-click": "hypr2d move right",
  "on-click-right": "hypr2d move left",
  "on-click-middle": "hypr2d goto 0 0",
  "on-scroll-up": "hypr2d move up",
  "on-scroll-down": "hypr2d move down"
}
```

For a bar fixed to one output, add `--monitor eDP-1` to the stream and **all**
actions, e.g. `hypr2d move right --monitor eDP-1`. Selected-monitor moves are
planned atomically in the plugin; the CLI does not calculate a cell and race
another read/goto pair. Add `custom/hypr2d` to the bar's module list. Do not set
`interval`, `signal` or `restart-interval`; keep `exec-on-event` false.

Every cell uses the same dot glyph. Pango spans set current blue (`theme.current`,
#82AAFF), occupied white (`theme.label`, #EEFFFF), empty grey (`theme.hover`,
#4A4A4A) and urgent red (#F07178). Extension annotations remain in the tooltip,
so arbitrary badges cannot widen cells or add height. Titles and metadata are
markup-escaped. Keep `escape: false`; no CSS state classes, underlines or borders
are needed for colours.

The dot block uses a 4-point monospace span with `line_height="0.75"`, keeping
three rows within a single 13px-text bar. Leave the bar's height automatic; do
not enlarge it for this module. Minimal module CSS is:

```css
#custom-hypr2d {
    font-family: monospace;
    min-height: 0;
    padding: 0 8px;
    margin: 0 4px;
    border: none;
}
```

To put it at the far left, before an existing `cpu` module:

```nix
programs.hypr2d.waybar = {
  enable = true;
  position = "left";
  placement = "start"; # mkBefore; "end" (the default) uses mkAfter
  radiusX = 4;
  radiusY = 1;
};
```

The VM renders a real Waybar with the user's monospace 13px, `min-height: 0`
settings, no fixed height, and `cpu` after the minimap. It compares layer heights
with and without the module and checks all three dot rows in a screenshot.

## Overview rendering prototype

The production overview uses the approved thumbnail rendering path. Its test-only
contact sheet remains available for capacity, state restoration and resource
regressions; the VM suite also drives the production UI with virtual input.

Run the deterministic screenshot and 25-cycle resource checks in the VM:

```sh
nix build .#integration --builders '' -L
```

`result/hypr2d-overview-results/` contains before/after/overlay/diff PNGs,
per-cell timings, process/DRM memory samples, and the child compositor log.
`nix flake check --builders '' -L` includes this suite and the M1–M3 regressions.
The VM shuts down after testing.

For a single hardware capture measurement in a normal development window:

```sh
HYPR2D_PROBE_RESULTS=/tmp/hypr2d-overview-hardware \
  nix run .#nested --builders '' -- --overview-benchmark
```

It creates only its own clients/outputs and closes them and the nested compositor
when finished. It performs no screenshot comparisons or repeated leak test;
those belong in the VM, independently of the host's frame callbacks.
See DESIGN.md §Findings for API constraints, measurements and the recommended
capture-on-open / per-cell damage-driven design.
