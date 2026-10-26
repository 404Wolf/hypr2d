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
  - Upgraded to eight sectors with deliberate re-targeting and projected 2D spring release; see the completed extension below.
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
- [x] Drag a window between cells, between monitors' grids, and into/out of special workspaces; drop on a window to stack (Shift-drop swaps); floating windows placed at the drop point
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

## Live Waybar minimap feedback
- [x] Native occupancy/position events refresh once per 16 ms burst, without polling; moving-window regression
- [x] Configurable 9×3 dot view (radiusX=4, radiusY=1), wrapping, Pango state colours and no borders
- [x] Three rows fit the existing automatic 13px-text bar: equal VM heights with/without the real Waybar module, screenshot verified
- [x] Home Manager start/end placement; start puts the minimap before cpu

- [x] Nullable dedicated minimap colours with validated CLI overrides and Home Manager wiring; defaults unchanged

## Live-machine overview regressions
- [x] Capture at displayed physical cell dimensions, resize on fit/full changes, filter minification and bound cache payload
- [x] Keep the software cursor above the overview; verify composited cursor pixels in the VM
- [x] Center a 2256×1504 3:2 eDP-1 at layout origin (8934, 6682), with consistent gaps; Rust geometry and VM screenshot checks
- [x] Preserve grouped-window focus/current members/order and normal/special workspace state; replace the conflicting SUPER+Tab bind and exercise it in the VM

## Additional live feedback
- [x] Zero/configurable overview gaps, direct thumbnail capture/filtering, label scrims and app-named numeric specials; VM screenshots
- [x] Reproduce an app-hidden cursor in the VM and give the overview a scoped native cursor override
- [x] VM minimap latency below 50 ms from commit/native events on swipe, carry, drop, map/unmap, specials, monitor and external workspace focus
- [x] Stable persisted monitor slots, parked workspace lifecycle, parked overview cards/expansion, minimap tooltip and hotplug VM regressions

- [x] Text-sized, physically aligned panel/special headers with consistent scaled 8px gaps; Rust and laptop VM screenshot regression
- [x] Drop-on-window stacks, Shift-drop swaps, locked-group refusal and immediate empty-special cleanup; VM regressions

- [x] Deferred public-dispatcher overview drops, cross-special/group accounting regressions and read-only doctor diagnostics
- [x] Eight-way tracked swipe: sector hysteresis, 2D tracking/projection/spring, diagonal commands, HM tunables and VM/unit tests

- [x] Crash postmortem: diagnose member/orphan spaces, assert generated Lua selectors, guarded deferred doctor repair and VM crash/recovery regressions

- [x] Waybar startup/plugin/socket/instance recovery with neutral placeholder and bounded backoff; default HM restart interval
- [x] Anchored full/centered minimap axes, HM options, geometry tests and unchanged 9-column bar height
- [x] Configurable swipe sensitivity with gained screen-space thresholds, HM/Lua wiring and tests

- [x] Four-finger overview pinch cancellation with sequence ownership, configurable threshold, native Lua opener and VM state/no-reopen regressions

- [x] Overview wallpaper/empty desktops, budgeted supersampling and subtle animated tile polish; VM screenshots and 2256×1504 9×9 timing
  - Software-rendered VM timing is recorded in DESIGN.md; Radeon frame pacing remains a user-owned live verification.

- [x] Live full-grid regression: fit five-special strip inside the 2256×1504 output, continuous empty-cell backdrop, no capture feedback, deferred opening frame; Rust and VM checks
  - First frame captures zero thumbnails; the software VM still exceeds 30 ms total frame time. Radeon opening feel remains a user-owned live check.

- [x] Translucent configurable tile fills and adjacent-panel row/column drag guides; unequal-resolution multi-output VM screenshots

## Questions
