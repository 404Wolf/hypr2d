{ self, home-manager, pkgs }:
let
  inherit (pkgs) lib;
  evaluate = settings: home-manager.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [ self.homeManagerModules.default {
      home.username = "hypr2d-test";
      home.homeDirectory = "/home/hypr2d-test";
      home.stateVersion = "26.05";
      programs.hypr2d = settings;
    } ];
  };
  enabled = evaluate {
    enable = true;
    grids = { "WAYLAND-1" = { rows = 9; wrap = false; }; "HEADLESS-1" = {}; };
    overview.bind = true;
    theme.fontSize = 18;
    theme.rounding = 2;
    gestures = { fingers = 4; distance = 140; axisLock = 6; natural = false; };
    waybar = { enable = true; monitor = "WAYLAND-1"; radius = 2; };
  };
  disabled = evaluate {};
  invalid = evaluate { enable = true; gestures.axisLock = 1000; };
  c = enabled.config;
  lua = pkgs.writeText "hypr2d-hm-generated.lua" c.wayland.windowManager.hyprland.extraConfig;
  valid = assert c.programs.hypr2d.enable;
    assert lib.all (a: a.assertion) c.assertions;
    assert c.wayland.windowManager.hyprland.configType == "lua";
    assert c.wayland.windowManager.hyprland.package.outPath == self.inputs.hyprland.packages.${pkgs.stdenv.hostPlatform.system}.hyprland.outPath;
    assert lib.elem c.programs.hypr2d.package c.wayland.windowManager.hyprland.plugins;
    assert c.programs.waybar.settings.mainBar."custom/hypr2d".exec-on-event == false;
    assert !(c.programs.waybar.settings.mainBar."custom/hypr2d" ? interval);
    assert c.programs.waybar.settings.mainBar.modules-left == [ "custom/hypr2d" ];
    assert lib.hasInfix ("--monitor " + lib.escapeShellArg "WAYLAND-1") c.programs.waybar.settings.mainBar."custom/hypr2d".exec;
    assert !(disabled.config.xdg.configFile ? "hypr/hypr2d.json");
    assert !(builtins.tryEval invalid.activationPackage.drvPath).success;
    true;
in {
  inherit lua;
  check = assert valid; pkgs.runCommand "hypr2d-home-manager-check" { nativeBuildInputs = [ pkgs.lua pkgs.python3 ]; } ''
    # Parse generated Lua without running a compositor or activating a home.
    luac -p ${lua}
    luac -p ${pkgs.writeText "hyprland.lua" c.xdg.configFile."hypr/hyprland.lua".text}
    python3 - <<'PY'
    import json
    config=json.loads(${builtins.toJSON c.xdg.configFile."hypr/hypr2d.json".text})
    assert config["theme"]["font_size"] == 18
    assert config["theme"]["rounding"] == 2
    assert config["grid"]["stride"] == 10
    assert config["gestures"]["fingers"] == 4
    PY
    mkdir -p $out
    cp ${lua} $out/hypr2d.lua
  '';
}
