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
- [x] 3-finger swipe follows the fingers 1:1 along a locked axis to the neighbouring cell, commits past distance/velocity thresholds, otherwise springs back; rubber-band at non-wrapping edges
- [x] Physics/decision logic in Rust with unit tests; C++ only feeds events and applies offsets
  - Tested with Rust physics and synthetic events through production listeners in nested Hyprland. Real-trackpad hand-tests remain scheduled by the user; see README §Tracked swipe.

## M3 — Waybar module
- [x] `hypr2d waybar` streams JSON (no polling): minimap around the current cell, occupied/empty/urgent states as CSS classes, tooltip listing windows per occupied cell, click/scroll to move
- [x] Example waybar config + CSS in README

## M4 — Overview
- [x] Rendering prototype: thumbnail-sized full-fake normal/special captures from two monitors, custom-pass composition, cost/memory measurements, and VM screenshot/state/resource tests
  - Rendering prototype approved; full M4 UI implemented and VM-tested below. See DESIGN.md §Findings.
- [x] `hypr2d:overview` shows ALL monitors' grids at once (one panel per monitor, laid out like the physical monitors) plus the special-workspace strip, with live thumbnails
- [x] Keyboard (hjkl/arrows, tab between monitors, Enter, Esc) and mouse (click a cell to go there)

## M5 — Drag & drop in the overview
- [x] Drag a window between cells, between monitors' grids, and into/out of special workspaces; drop on a window to swap; floating windows placed at the drop point
- [x] Ghost thumbnail + target highlight; Esc cancels

## Overview polish (after M5)
- [x] Maximize layout scale while preserving physical panel arrangement and monitor-aspect cells; default fit bounding box + one-cell margin, at least 3×3, `a` full-grid toggle
- [x] Configurable OneDark Extra Dark palette/fonts, subtle rounding, readable app/count cell labels; verified in VM screenshots

- [x] Final visual polish: blank outlined empty cells, vertically centered composition, short app-class labels with +N; Rust and VM screenshot checks

## M6 — Extension socket (no agent-specific code in this repo)
- [x] Versioned JSON-lines socket: annotations (label/badge/dot/colour/tooltip on cells, specials, windows), reserved regions (rows with titles), placement, activate/drop callbacks, session-scoped cleanup on disconnect
- [x] `hypr2d-client` Rust crate + protocol docs; overview and waybar render annotations

## M7 — Home Manager integration
- [x] `homeManagerModules.default`: loads the plugin, writes grid config, provides the waybar module
- [ ] The user's binds and gestures switched over (see DESIGN.md §Binds) in their home-manager repo, `workspace2d.sh` removed
  - User-owned follow-up: this task implements the module in this repo only; no Home Manager checkout edits or activation.

## Questions
