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
          fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./core ];
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
        src = pkgs.lib.fileset.toSource { root = ./.; fileset = ./plugin; };
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ compositor ] ++ compositor.buildInputs;
        dontConfigure = true;
        buildPhase = ''
          $CXX -std=c++26 -shared -fPIC plugin/main.cpp \
            -I${rust}/include $(pkg-config --cflags hyprland) \
            ${rust}/lib/libhypr2d_core.a -o libhypr2d.so
        '';
        installPhase = "mkdir -p $out/lib; cp libhypr2d.so $out/lib/";
      };
      nested = pkgs.writeShellApplication {
        name = "hypr2d-nested";
        runtimeInputs = [ compositor pkgs.python3 ];
        text = builtins.replaceStrings [ "@PLUGIN@" "@SMOKE@" ] [ "${plugin}/lib/libhypr2d.so" "${./nix/smoke.py}" ] (builtins.readFile ./nix/nested.sh);
      };
    in { default = plugin; inherit plugin nested rust; });
    apps = each (system: { nested = { type = "app"; meta.description = "Run the isolated nested Hyprland spike"; program = "${self.packages.${system}.nested}/bin/hypr2d-nested"; }; });
    checks = each (system: { inherit (self.packages.${system}) plugin rust; });
  };
}
