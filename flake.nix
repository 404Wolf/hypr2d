{
  description = "hypr2d: Rust-backed 2D workspace grid plugin";
  inputs.hyprland.url = "github:hyprwm/Hyprland/e368c13c27a42a173b9e08fa0bf413f9f7073187";
  inputs.nixpkgs.follows = "hyprland/nixpkgs";
  outputs = { self, nixpkgs, hyprland }: let
    systems = [ "x86_64-linux" "aarch64-linux" ];
    each = nixpkgs.lib.genAttrs systems;
  in {
    packages = each (system: let
      pkgs = import nixpkgs { inherit system; };
      compositor = hyprland.packages.${system}.hyprland;
      rust = pkgs.rustPlatform.buildRustPackage {
        pname = "hypr2d-core";
        version = "0.1.0";
        src = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./core ./cli ];
        };
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = [ pkgs.rustfmt pkgs.clippy ];
        postCheck = ''
          cargo fmt --all -- --check
          cargo clippy --workspace --all-targets --offline -- -D warnings
        '';
        installPhase = ''
          mkdir -p $out/lib $out/include
          cp target/*/release/libhypr2d_core.a $out/lib/
          cp -rL target/*/cxxbridge/* $out/include/
        '';
      };
      plugin = compositor.stdenv.mkDerivation {
        pname = "hypr2d";
        version = "0.1.0";
        src = pkgs.lib.fileset.toSource { root = ./.; fileset = pkgs.lib.fileset.unions [ ./plugin ./.clang-format ]; };
        nativeBuildInputs = [ pkgs.pkg-config pkgs.clang-tools ];
        buildInputs = [ compositor ] ++ compositor.buildInputs;
        dontConfigure = true;
        buildPhase = ''
          ${pkgs.bash}/bin/bash ${pkgs.clang-tools}/bin/clang-format --dry-run -Werror plugin/main.cpp
          $CXX -std=c++26 -shared -fPIC plugin/main.cpp \
            -I${rust}/include $(pkg-config --cflags hyprland) \
            ${rust}/lib/libhypr2d_core.a -o libhypr2d.so
        '';
        installPhase = "mkdir -p $out/lib; cp libhypr2d.so $out/lib/";
      };
      nested = pkgs.writeShellApplication {
        name = "hypr2d-nested";
        runtimeInputs = [ compositor pkgs.python3 pkgs.foot ];
        text = builtins.replaceStrings [ "@PLUGIN@" "@SMOKE@" ] [ "${plugin}/lib/libhypr2d.so" "${./nix/smoke.py}" ] (builtins.readFile ./nix/nested.sh);
      };
      parentConfig = pkgs.writeText "hypr2d-ci-parent.lua" ''
        hl.monitor({ output = "", mode = "1280x720@60", position = "auto", scale = 1 })
        hl.config({ xwayland = { enabled = false } })
        hl.on("hyprland.start", function()
          hl.exec_cmd("${nested}/bin/hypr2d-nested --smoke-test >/tmp/hypr2d-test.log 2>&1; echo $? >/tmp/hypr2d-status")
        end)
      '';
      integration = pkgs.testers.runNixOSTest {
        name = "hypr2d-integration";
        nodes.machine = { pkgs, ... }: {
          environment.systemPackages = [ nested compositor ];
          hardware.graphics.enable = true;
          fonts.packages = [ pkgs.dejavu_fonts ];
          services.getty.autologinUser = "root";
          virtualisation = {
            cores = 2;
            memorySize = 4096;
            qemu.options = [ "-vga none -device virtio-gpu-pci" ];
          };
          system.stateVersion = "26.05";
        };
        testScript = ''
          machine.wait_for_unit("multi-user.target")
          machine.wait_until_succeeds("test -d /run/user/0")
          # The non-3D virtio GPU exposes an unusable render node. Force the
          # authenticated primary-node path inside this disposable VM only;
          # keep the expected path as an alias for EGL device discovery.
          machine.succeed("rm -f /dev/dri/renderD128; ln -s card0 /dev/dri/renderD128")
          machine.send_chars("Hyprland --i-am-really-stupid --config ${parentConfig} >/tmp/hypr2d-parent.log 2>&1\n")
          machine.wait_until_succeeds("test -f /tmp/hypr2d-status", timeout=180)
          print(machine.succeed("cat /tmp/hypr2d-test.log"))
          machine.succeed("grep 'PASS M1:' /tmp/hypr2d-test.log")
          machine.succeed("grep '^0$' /tmp/hypr2d-status")
          machine.copy_from_machine("/tmp/hypr2d-test.log")
        '';
      };
    in { default = plugin; inherit plugin nested rust integration; });
    apps = each (system: { nested = { type = "app"; meta.description = "Run the isolated nested Hyprland spike"; program = "${self.packages.${system}.nested}/bin/hypr2d-nested"; }; });
    devShells = each (system: let pkgs = import nixpkgs { inherit system; }; in { default = pkgs.mkShell { packages = [ pkgs.cargo pkgs.rustc pkgs.rustfmt pkgs.clippy pkgs.clang-tools ]; }; });
    checks = each (system: { inherit (self.packages.${system}) plugin rust integration; });
  };
}
