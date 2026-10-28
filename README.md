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
hl.bind("SUPER + ALT + TAB", hl.plugin.hypr2d.parked())
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
Plain drop moves only the visible member of a stack; the remaining members stay
in their workspace and the next member becomes current. Drop onto another window
to stack the moved window onto it as the active member.

| Modifier | Single-window drag | Region drag |
| --- | --- | --- |
| Plain | Move the visible member; stack onto a window target | Move every window in the block; merge occupied cells |
| Shift | Move the whole stack intact | Move the whole block, unchanged |
| Alt | Swap the visible source and target windows | Swap the two blocks |
| Shift + Alt | Swap whole stacks | Swap the two blocks |
| Esc | Cancel the pending drag | Cancel pickup/selection before closing |

Modifiers update while dragging. Shift takes the whole stack; combining it with
Alt exchanges whole stacks. Region drags always include every window in the
selected cells, so Shift does not change their contents.
Drop on empty space or a cell's top caption edge for a plain move. A dragged
group member leaves its former group; group locks refuse the drop with a red
flash. Empty source grid cells update immediately and a special tile disappears
when its last window leaves. Floating windows retain
the grabbed point at the pointer's destination, including near output edges.
A translucent ghost and gold cell or window outline show the pending drop;
window outlines are labelled stack or swap.
A Ctrl-drag anywhere, or a drag starting in an empty cell, selects a rectangle
of cells. Drag the selected block to move it, preserving cell offsets. Occupied
destinations merge; Alt swaps disjoint blocks. Overlapping swaps are refused.
Wrapping applies per axis; non-wrapping grids refuse blocks that do not fit.
Keyboard: `v` begins selection, hjkl extends it, `m` picks it up, hjkl moves it,
Tab changes the destination grid, and Enter drops it. Ctrl+Z undoes the last
region transfer as one operation, including floating positions. Regions include
whole groups; pre-existing destination windows are left alone on undo. History
holds up to 20 operations in memory and is cleared by single-window drops,
client closure, output removal or grid reassignment. Esc cancels selection first.

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
windows. Empty grid cells have no caption: a thin base02 outline and subtle translucent
fill sit over the dimmed wallpaper. Occupied cells retain their workspace thumbnails.

Configure appearance with one atomic JSON object (omitted fields use defaults):

```sh
hyprctl hypr2d overview-config '{"fit":true,"background":"#000000","cell":"#303030","border":"#353535","hover":"#4A4A4A","current":"#82AAFF","target":"#FFCB6B","label":"#EEFFFF","font":"sans-serif","font_size":16,"rounding":3}'
```

Colours use `#RRGGBB`; `tile` uses `#RRGGBBAA` (including alpha).
Font size is 10–40 output pixels and rounding is 0–4 px.
These defaults match base16 OneDark Extra Dark. Runtime changes apply while
open. Capture dimensions are the displayed cell size × monitor scale ×
`overview.supersample` (default 2), reduced only to meet the memory budget.
The current desktop has a 2 px `overview.accent` ring (default #9CBEFF) and faint
glow; selection/minimap colours retain `theme.current`. Hover lifts tiles 2%
with a 120 ms ease. Occupied tiles have soft shadows; headers use 85% opacity.
Open/close zooms around the current desktop over 180 ms, including interrupted
open/close transitions. Pointer selection starts when the zoom settles;
Esc and pinch cancellation remain available throughout.

Every cell has a translucent `programs.hypr2d.theme.tile` fill (default
`"#FFFFFF1A"`, white at about 10% alpha). For example, `"#D0D0D01F"` gives
light grey at about 12%. Occupied thumbnails draw above it; empty cells retain
the continuous wallpaper. Runtime JSON uses `"tile":"#FFFFFF1A"`.

Adjacent live monitor panels have row guides horizontally and column guides
vertically. These connect the **same visible logical row/column index**, including
unequal resolutions, using 1×scale px `theme.border` lines at 50% opacity (80%
while dragging). Small panel gutters leave room for the lines. Only the nearest
physical neighbour with overlapping span is connected; parked/expanded panels
and a single live monitor have no guides. In fit mode, indices hidden in either
panel have no connector. The guides illustrate **drag targets**: `move`/`carry`
continue to wrap or clamp inside their own monitor's grid, never cross monitors.

Overview cells and special tiles default to flush spacing with one-pixel inset
borders. Set `programs.hypr2d.overview.gap` (0–32, before layout scaling) to
increase spacing. Labels have an 80% background-colour scrim. Numeric special
workspace identifiers use the main app's short name; named specials and extension
labels take precedence.

The backdrop captures each monitor's BACKGROUND and BOTTOM layer surfaces
(including hyprpaper), without windows, top layers or cursor. Default darkening
is 35%; empty desktops retain this continuous image beneath their translucent fill. No additional
blur pass is added. Wallpapers refresh on layer surface commits/map/unmap.
Without mapped lower layers, the flat theme fallback needs no backdrop texture.

```nix
programs.hypr2d.overview = {
  supersample = 2;       # 1..4
  memoryBudgetMiB = 128; # 16..512, shared across monitors
  backgroundDarken = 0.35;
  accent = "#9CBEFF";
};
```

Captures use the simple full-fake path directly into supersampled physical
framebuffers, then generate mipmaps for trilinear minification. There is no
full-output export resample. Only visible occupied cells get individual
textures; empty cells allocate nothing and show the continuous backdrop. The budget reserves 25%
for backdrops and 75% for thumbnails, including mipmap payload. Resize retires
old mip levels. Refreshes remain damage-driven with a 2 ms queue budget; one
capture is indivisible. GPU/driver bookkeeping and text textures are outside
the RGBA capture payload cap. The first overview frame uses placeholders with no
captures; subsequent frames drain the queue. Pending client frame callbacks are
completed at native frame completion, separately from capture. Callback-only
commits do not dirty thumbnails; actual visual changes still refresh them.
The capture count is cumulative for the open session, not the number of allocated
textures. Diagnostics include `opening_dispatch_ms`, `first_frame_ms` and
`first_frame_capture_ms`; software-VM total frame time is not a Radeon latency
guarantee.
`hyprctl hypr2d overview state` reports capture payload, timing, framebuffer and drawn physical dimensions;
layout changes log both. `hyprctl hypr2d overview cursor` reports cursor-image
availability and native software/hardware state for hardware diagnostics.
`hyprctl hypr2d overview open` and `close` are idempotent, unlike the Lua toggle.
The overview temporarily owns an arrow cursor and restores the application's
cursor, including a hidden image, when it closes.

## Named grids

Grids can be created independently of outputs. They retain their own dimensions,
wrap policy, current cell and windows when adopted or swapped:

```sh
hypr2d grid create work '{"columns":9,"rows":9,"wrap":true}'
hypr2d grid list
hypr2d adopt work eDP-1
hypr2d grid rename work "writing space"
hypr2d grid delete "empty parked grid"
```

Creation without options inherits the focused grid's dimensions/wrap. The CLI
prints the new grid ID and configuration as JSON. Grid names and IDs are accepted
by adopt/rename/delete. Lua dispatcher constructors are
`hl.plugin.hypr2d.grid_create("work", {columns=9, rows=9, wrap=true})`,
`hl.plugin.hypr2d.grid_rename("work", "writing space")`,
`hl.plugin.hypr2d.grid_delete("empty parked grid")` and
`hl.plugin.hypr2d.adopt("work", "main")`. They can be passed to `hl.bind` or
`hyprctl dispatch` just like the move constructors.

Delete refuses live, active or nonempty grids; it never closes clients. IDs are
not recycled. Legacy grid numbers retain the original formula; named grids use
collision-checked reserved workspace blocks. Query `state`/`grid list` or use the
move/goto commands instead of constructing a named workspace number yourself.
Names and dimensions persist in `$XDG_STATE_HOME/hypr2d/catalog.json`; assignments
and per-grid current cells remain in `grids.json`. Legacy names initially follow
their output labels; renaming a grid gives it an independent name.

The focused development VM is `nix build .#catalog-integration --builders ''`.
It tests grouped/floating clients, virtual IDs, adoption, two/three-output swaps,
Lua CRUD, captures and restart persistence in an isolated compositor.

## Grid assignments and parked targets

A grid keeps its slot, workspace numbers, windows and current cell independently
of the output displaying it. `hypr2d adopt GRID [MONITOR]` assigns a grid
to an output (the focused output by default), parking its previous grid when the requested grid is parked. If the requested
grid is already live elsewhere, the two monitors swap grids.
`hypr2d swap-grids A B` exchanges two live assignments. Output names, numeric
monitor IDs and unique display labels are accepted, including quoted labels
with spaces. Lua binds use `hl.plugin.hypr2d.adopt(slot, monitor)` and
`hl.plugin.hypr2d.swap_grids(monitorA, monitorB)` dispatcher constructors.

Drag a parked card's header onto a live panel header to adopt it; drag between
two live headers to swap. Keyboard: Tab to the source grid, `g`, Enter assigns it
to the monitor that was focused when the overview opened. Esc cancels.
Parked grids remain available even when empty. Expand their cards to use the
full grid; during a single-window or region drag, hover the live/parked tab to
change the destination view. Both directions use the same safe drop path.

Assignments and each grid's current cell persist in
`$XDG_STATE_HOME/hypr2d/grids.json`. `programs.hypr2d.gridRestore = "auto"`
(default) prefers the output's remembered assignment on reconnect if free;
`"keep"` prefers its last fallback. Neither policy steals a live grid. Workspace
numbers never change on adoption or swap. Output identity remains in slots.json.

## Display labels

`programs.hypr2d.specials.names = { "3" = "chatting"; "1" = "agent"; };`
sets special-workspace captions; `special:3` is also accepted as a key. Explicit
labels take precedence over the app fallback, which remains a smaller dim
caption. `programs.hypr2d.monitors.labels = { "eDP-1" = "main";
"Vendor Model Serial" = "right"; };` accepts output names and monitor
descriptions (with optional `desc:`), make/model or make-model. Output-name
matches have precedence. Description keys survive port renames.
`monitors.autoLabel = "position"` names unlabelled connected outputs by x order:
left/middle/right (main for one output). Parked grids retain their last position
name. The default is `"none"`.

In the overview, point at a panel header or special tile and press `r` to edit
its label. Enter saves, Esc cancels, Ctrl+A clears the editor. Saving an empty
name removes a runtime override. Names persist atomically in
`$XDG_STATE_HOME/hypr2d/labels.json`; HM-configured names take precedence and
cannot be overwritten from the editor. Labels appear in panel/card headers,
adoption targets and the minimap tooltip. Duplicate monitor labels require an
output name or ID in CLI commands.

The shared wallpaper backdrop also accepts `overview.backgroundTint` (default
0.12): a neutral grey overlay that softens the image without extra captures.
Set it to zero to retain only `overview.backgroundDarken`.

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
`monitorSlots`, `theme`, `gestures`, and `waybar`. The module loads the plugin through Home
Manager's Lua plugin support, appends configuration Lua, installs the CLI and
writes `~/.config/hypr/hypr2d.json` as a readable record of generated settings.
The plugin is configured by generated Lua. It reads monitor slot pins from that
JSON at plugin load; it does not watch the file.
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

## Overview pinch cancellation

Keep the native four-finger opening bind:

```lua
hl.gesture({ fingers = 4, direction = 'pinchin', action = hl.plugin.hypr2d.overview() })
```

While the overview is open, a new four-finger pinch in either direction cancels
it, including any drag or uncommitted selection. Focus, group current members,
all monitors' current workspaces and visible specials remain unchanged. The
plugin consumes that entire cancellation gesture, so its end cannot reopen the
overview. The opening gesture itself finishes normally. Swipes remain separate.

The default cancellation scale change is 8% from 1.0. Configure it with
`programs.hypr2d.pinch.cancelScale = 0.08;` (strictly between 0 and 1), or at
runtime with `hyprctl hypr2d pinch-config 0.08`. A subthreshold
pinch does not close the overview. Closing uses the normal short animation.

Hand-test after deployment: pinch-in opens; lift, then pinch-in and pinch-out
each cancel without reopening; move the selection first and repeat with a
visible special and a non-head group member focused. Verify four-finger swipes
do not act as pinches.

## Tracked swipe

The plugin consumes three-finger swipes before Hyprland's built-in handlers.
After 5 logical pixels of accumulated travel, its smoothed angle selects one of
eight sectors. Diagonal sectors span ±15°; the wider cardinal sectors span ±30°,
so sloppy straight swipes stay straight. Re-targeting requires 8° inside another
sector and 12 px of sustained travel. Current, horizontal, vertical and diagonal
workspaces follow the fingers at 1:1; a cardinal swipe renders only two.

Swipe up means grid down with natural direction enabled. Release projects the
50 ms smoothed velocity with constant deceleration, then chooses the nearest
neighbour in viewport-normalized coordinates. Each committed axis must pass
half its viewport and the minimum distance (120 px). Projection starts above
0.8 px/ms; pausing removes stale velocity. A critically damped 2D spring preserves
release velocity. Non-wrapping edges resist per axis; blocked diagonal axes fall
back to a valid cardinal. Cancellation bounces.

Optional Lua configuration (inside the same namespace guard as the binds):

```lua
if hl.plugin.hypr2d then
    local ok, message = hl.plugin.hypr2d.swipe_config({
        fingers = 3, distance = 120, velocity = 0.8,
        axis_lock = 5, natural = true, sensitivity = 1.0,
        diagonal_width = 15, hysteresis = 8, hysteresis_distance = 12,
        projection_decel = 0.0006, spring_stiffness = 576,
    })
    assert(ok, message)
end
```

Home Manager exposes the same settings under `programs.hypr2d.gestures`, with
`diagonalWidth`, `hysteresis`, `hysteresisDistance`, `projectionDecel` and
`springStiffness` for the new tunables. Deceleration uses px/ms²; stiffness uses
s⁻². No diagonal keybinds are installed.
Set `programs.hypr2d.gestures.sensitivity = 1.3` (Lua `sensitivity = 1.3`)
for 30% faster tracking. The default is `1.0`; any finite positive gain is
accepted. Displacement and velocity are multiplied before classification and
commit decisions, so distance/velocity thresholds keep their screen-space
meaning. Edge resistance and springs then apply to the gained travel.


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

1. Three fingers: slow travel follows fingers at 1:1 by default in all eight directions;
   set sensitivity=1.3 and check 30% faster travel and screen-space commit thresholds.
   Natural swipe up selects grid down. Diagonal pans cover every corner without
   seams; slightly crooked horizontal swipes stay horizontal.
2. Hover around a sector boundary: no flicker. Clearly curve into another sector:
   it re-targets without a jump. Reverse through the origin smoothly.
3. Release slowly before the viewport midpoint: bounce; past it: commit. Check
   both axes on the laptop's 3:2 display. A diagonal with only one axis past its
   threshold becomes cardinal. A short fast flick commits; flick then pause
   bounces. Check spring continuity and tune the exposed settings if needed.
4. Cancel a gesture (including lifting fingers unexpectedly): bounce with no
   stuck offset. Repeated gestures during spring settling start from the cell
   already committed. Check both natural=true and natural=false.
5. With wrap enabled, cross every grid boundary. With wrap disabled, test all
   four edges and corners: diagonal fallback, bounded resistance, no invalid workspace, smooth return.
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
`hypr2d state`, `hypr2d move left`, `hypr2d carry down-right`, or `hypr2d goto 2 3`.
Moves/carries accept `up-left`, `up-right`, `down-left`, `down-right`;
`hypr2d move up-left all` steps every connected grid independently.
The CLI talks directly to Hyprland's Unix socket; it starts no subprocesses.
`waybar` stays alive when sockets or the plugin are missing, emits a small neutral
placeholder, and retries with bounded 100 ms–2 s backoff. Once ready it requests
snapshots only for socket2 events (16 ms burst debounce), never on a healthy idle
timer. It re-subscribes after socket2 disconnect/replacement and discovers a new
instance when the inherited signature becomes stale. `--instance` deliberately
pins a single instance, useful for nested testing. The HM entry defaults to
`restart-interval = 2` as an additional Waybar-side fallback.

Use `hypr2d waybar --columns full --rows centered` to anchor every grid column
at a fixed horizontal position. On a 9×9 grid this keeps nine columns in place;
left/right movement moves only the blue dot. Rows remain current row ±1 with
wrapping. `--rows full` anchors all rows too. Each axis defaults to `centered`;
its radius applies only in centered mode. Extra rows reduce the markup font size
so they share the existing compact content-height budget.

```nix
programs.hypr2d.waybar = {
  columns = "full";
  rows = "centered";
  radiusY = 1;
};
```


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

Minimap colours can be overridden independently of the overview theme:
`programs.hypr2d.waybar.colors = { current = "#9CBEFF"; occupied = "#A8A8A8"; };`
Each of `current`, `occupied`, `empty` and `urgent` defaults to `null`,
preserving the existing theme-derived colours (and fixed urgent red).
The CLI equivalents are `--current-color '#9CBEFF'`,
`--occupied-color '#A8A8A8'`, `--empty-color '#4A4A4A'` and
`--urgent-color '#F07178'`; quote hex values in shell commands.

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

## Doctor

`hyprctl hypr2d doctor` is a read-only JSON check of mapped windows, layout
membership, group members/current member, and both effective and member ghost
spaces. `details` distinguishes a null space from a space whose workspace has
expired. Singleton groups are warnings: Hyprland permits them.

To repair supported leftover corruption explicitly:

```sh
hyprctl hypr2d doctor repair
hyprctl hypr2d doctor
```

Repair returns `{"queued":true}` and runs after the input/IPC handler returns.
Query doctor again for `repair.state`: `complete`, `refused`, or `failed`, with an
error and lists of rebuilt windows/dissolved singleton groups. `ok` describes
the current invariants; inspect it as well as the repair result. Unloading cancels
a queued repair. Repeating a completed repair is safe and does no further work.

Supported repairs reattach an existing layout target to the window's existing,
enabled workspace, including missing membership, a wrong space, a literally
null target/ghost space, or a space with an expired workspace. All group member
ghosts are restored **before** target reinsertion or any group dispatcher.
Orphan layout algorithms are discarded before their unsafe callbacks can run.
Only after layout consistency is verified are unlocked singleton groups dissolved
through the public dispatcher. Multi-member groups keep their membership/current
member, and locked singleton groups remain warnings. Focus, workspace/special
visibility, fullscreen state and floating positions are retained; rebuilding a
tiled target can reflow its workspace.

Repair refuses unknown/stale native targets, duplicate targets within one layout
algorithm, invalid group membership/current index, groups split across workspaces,
shared targets outside a group, and windows without an authoritative enabled
workspace/space. Refused plans mutate nothing. A failed repair reports its error
and skips subsequent group operations if ownership is still inconsistent. No
client is closed, restarted or moved to another workspace as a repair strategy.
Drops, carry and extension placement refuse inconsistent state until repaired.

Avoid manual group removal on a corrupt target. In particular, the null-space
warning alone is not proof of corruption: normal group removal emits it too.
For explicit Lua window operations always resolve and assert the full selector
first: `local w = assert(hl.get_window("address:0x…"))`. A bare `0x…` selector can
return nil, and a nil dispatcher argument falls back to the focused window.
Generated hypr2d scripts assert every address lookup before dispatching.

## Monitor slots and parking

Monitor numbering uses permanent slots from `$XDG_STATE_HOME/hypr2d/slots.json`
(default `~/.local/state/hypr2d/slots.json`). `eDP-1` reserves slot 0; new outputs
get unused slots which are never recycled. Output name is primary; a unique
make/model/serial match reconnects a renamed port to its previous slot. With the
default grid the formula remains `10*(y*8+x)+slot+1`. IPC monitor IDs remain
Hyprland IDs; numeric `grids`/`configure` keys refer to stable slots. Configure
output names when possible. `programs.hypr2d.monitorSlots = { "eDP-1" = 0;
"DP-1" = 2; };` pins initial assignments; conflicting persisted assignments are
refused rather than silently renumbered. The shared stride must exceed all slots.

Unplugged normal workspaces keep their numbers on a remaining output. They are
excluded from that output's grid and minimap; its tooltip lists parked grids.
The overview shows compact dimmed cards beside specials. Click a card or use
Tab then Enter to expand it, Esc to return, and the live/parked tabs to switch.
Drag from a card into a live cell, or from an expanded parked grid over the live
tab and into a cell. `SUPER+ALT+Tab` (above) and `hyprctl hypr2d overview parked`
open the parked view directly. Replug restores workspaces and the saved active
cell to the same monitor slot.
