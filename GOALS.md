# hypr2d: a high-quality 2D workspace grid plugin for Hyprland

Read DESIGN.md first; it is the spec. Tick items (`- [x]`) as they land, with
tests. Never edit the headline (line 1). Add notes under an item if a decision
changed; flag anything that blocks you at the bottom under "Questions".

## M0 — Spike: prove the toolchain on Hyprland 0.56
- [x] Flake builds a minimal plugin `.so` against the `hyprland` flake input (pinned to the user's rev, see DESIGN.md) and loads in a nested Hyprland via `nix run .#nested`
- [x] C++ shim ↔ Rust core bridge via `cxx`, with a round-trip call proven from the plugin (log line in the nested instance's log)
- [x] Confirm what 0.56's plugin API offers for: registering dispatchers, posting socket2 events, hooking trackpad swipe gestures, rendering workspaces to textures (hyprexpo's path), render-pass elements. Write findings in DESIGN.md §Findings

## M1 — Grid core (one grid per monitor)
- [x] Rust core: grid model, (x,y,monitor) ↔ workspace id with the user's current numbering, wrap, moves, carry, "all monitors" moves, unit tests
- [x] Dispatchers: `hypr2d:move <dir> [all]`, `hypr2d:carry <dir>`, `hypr2d:goto <x> <y>`, plus correct slide direction (horizontal vs vertical) without config mutation
  - 0.56 deviation: legacy dispatcher registration is a stub; use the equivalent `hl.plugin.hypr2d.*` Lua dispatchers documented in README §Binds.
- [x] socket2 events (`hypr2d>>…`) and `hyprctl hypr2d state -j`
- [x] Integration test in a headless/nested Hyprland driven by hyprctl (CI-runnable)

## M2 — Tracked 2D trackpad swipe
- [ ] 3-finger swipe follows the fingers 1:1 along a locked axis to the neighbouring cell, commits past distance/velocity thresholds, otherwise springs back; rubber-band at non-wrapping edges
- [ ] Physics/decision logic in Rust with unit tests; C++ only feeds events and applies offsets

## M3 — Waybar module
- [ ] `hypr2d waybar` streams JSON (no polling): minimap around the current cell, occupied/empty/urgent states as CSS classes, tooltip listing windows per occupied cell, click/scroll to move
- [ ] Example waybar config + CSS in README

## M4 — Overview
- [ ] `hypr2d:overview` shows ALL monitors' grids at once (one panel per monitor, laid out like the physical monitors) plus the special-workspace strip, with live thumbnails
- [ ] Keyboard (hjkl/arrows, tab between monitors, Enter, Esc) and mouse (click a cell to go there)

## M5 — Drag & drop in the overview
- [ ] Drag a window between cells, between monitors' grids, and into/out of special workspaces; drop on a window to swap; floating windows placed at the drop point
- [ ] Ghost thumbnail + target highlight; Esc cancels

## M6 — Extension socket (no agent-specific code in this repo)
- [ ] Versioned JSON-lines socket: annotations (label/badge/dot/colour/tooltip on cells, specials, windows), reserved regions (rows with titles), placement, activate/drop callbacks, session-scoped cleanup on disconnect
- [ ] `hypr2d-client` Rust crate + protocol docs; overview and waybar render annotations

## M7 — Home Manager integration
- [ ] `homeManagerModules.default`: loads the plugin, writes grid config, provides the waybar module
- [ ] The user's binds and gestures switched over (see DESIGN.md §Binds) in their home-manager repo, `workspace2d.sh` removed

## Questions
