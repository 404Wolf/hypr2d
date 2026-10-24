# hypr2d

A Rust-backed Hyprland 0.56 plugin. M0 is a toolchain spike only: it logs a
C++ → Rust → C++ round trip on load. Grid features are not implemented yet.
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
