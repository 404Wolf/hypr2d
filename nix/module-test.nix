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
      programs.waybar.settings.mainBar.modules-left = [ "cpu" ];
    } ];
  };
  enabled = evaluate {
    enable = true;
    grids = { "WAYLAND-1" = { rows = 9; wrap = false; }; "HEADLESS-1" = {}; };
    monitorSlots = { "eDP-1" = 0; "HEADLESS-1" = 2; };
    overview.bind = true;
    pinch.cancelScale = 0.12;
    theme.fontSize = 18;
    theme.rounding = 2;
    gestures = { fingers = 4; distance = 140; axisLock = 6; natural = false; diagonalWidth = 14; hysteresis=7; hysteresisDistance = 16; projectionDecel = 0.0008; springStiffness = 625; sensitivity = 1.3; };
    waybar = { enable = true; monitor = "WAYLAND-1"; radiusX = 2; radiusY = 1; columns = "full"; placement = "start"; };
  };
  disabled = evaluate {};
  invalidPinch = (evaluate {enable=true;pinch.cancelScale=1;}).config;
  invalidSlots = evaluate {enable=true;monitorSlots={ "DP-1"=0; };};
  duplicateSlots = evaluate {enable=true;monitorSlots={ "DP-1"=2; "DP-2"=2; };};
  invalidDiagonal = evaluate { enable = true; gestures.diagonalWidth = 23; };
  invalidHysteresis = evaluate { enable = true; gestures.hysteresis = 16; };
  invalidProjection = evaluate { enable = true; gestures.projectionDecel = 0; };
  invalidSensitivity = evaluate { enable = true; gestures.sensitivity = 0; };
  invalidSpring = evaluate { enable = true; gestures.springStiffness = 0; };
  invalid = evaluate { enable = true; gestures.axisLock = 1000; };
  colored = (evaluate { enable = true; overview.gap = 4; waybar = { enable = true; colors = {
    current = "#9CBEFF"; occupied = "#A8A8A8"; empty = "#123456"; urgent = "#654321";
  }; }; }).config;
  invalidColor = evaluate { enable = true; waybar.colors.current = "#xyz123"; };
  c = enabled.config;
  lua = pkgs.writeText "hypr2d-hm-generated.lua" c.wayland.windowManager.hyprland.extraConfig;
  valid = assert c.programs.hypr2d.enable;
    assert lib.all (a: a.assertion) c.assertions;
    assert lib.hasInfix ''hl.unbind("SUPER + TAB")'' c.wayland.windowManager.hyprland.extraConfig;
    assert c.wayland.windowManager.hyprland.configType == "lua";
    assert c.wayland.windowManager.hyprland.package.outPath == self.inputs.hyprland.packages.${pkgs.stdenv.hostPlatform.system}.hyprland.outPath;
    assert lib.elem c.programs.hypr2d.package c.wayland.windowManager.hyprland.plugins;
    assert c.programs.waybar.settings.mainBar."custom/hypr2d".exec-on-event == false;
    assert c.programs.waybar.settings.mainBar."custom/hypr2d".restart-interval == 2;
    assert !(c.programs.waybar.settings.mainBar."custom/hypr2d" ? interval);
    assert c.programs.waybar.settings.mainBar.modules-left == [ "custom/hypr2d" "cpu" ];
    assert lib.hasInfix ("--monitor " + lib.escapeShellArg "WAYLAND-1") c.programs.waybar.settings.mainBar."custom/hypr2d".exec;
    assert lib.all (name: c.programs.hypr2d.waybar.colors.${name} == null
      && !(lib.hasInfix "--${name}-color" c.programs.waybar.settings.mainBar."custom/hypr2d".exec)
      && lib.hasInfix ("--${name}-color " + lib.escapeShellArg colored.programs.hypr2d.waybar.colors.${name}) colored.programs.waybar.settings.mainBar."custom/hypr2d".exec
    ) [ "current" "occupied" "empty" "urgent" ];
    assert colored.programs.hypr2d.theme.current == "#82AAFF";
    assert (builtins.fromJSON colored.xdg.configFile."hypr/hypr2d.json".text).theme.gap == 4;
    assert c.programs.hypr2d.overview.gap == 0;
    assert colored.programs.hypr2d.pinch.cancelScale == 0.08;
    assert lib.hasInfix "pinch-config 0.12" c.wayland.windowManager.hyprland.extraConfig;
    assert (builtins.fromJSON c.xdg.configFile."hypr/hypr2d.json".text).pinch.cancelScale == 0.12;
    assert c.programs.hypr2d.waybar.columns == "full" && c.programs.hypr2d.waybar.rows == "centered";
    assert colored.programs.hypr2d.waybar.columns == "centered" && colored.programs.hypr2d.waybar.rows == "centered";
    assert lib.hasInfix "--columns full --rows centered" c.programs.waybar.settings.mainBar."custom/hypr2d".exec;
    assert !(builtins.tryEval invalidColor.config.programs.hypr2d.waybar.colors.current).success;
    assert !(disabled.config.xdg.configFile ? "hypr/hypr2d.json");
    assert lib.all (v: !(builtins.tryEval v.activationPackage.drvPath).success) [ invalidPinch invalid invalidDiagonal invalidHysteresis invalidProjection invalidSpring invalidSensitivity ];
    assert lib.all (v: lib.hasInfix v c.wayland.windowManager.hyprland.extraConfig) [ "diagonal_width=14" "hysteresis=7" "hysteresis_distance=16" "projection_decel=0.0008" "spring_stiffness=625" "sensitivity=1.3" ];
    assert !(builtins.tryEval invalidSlots.activationPackage.drvPath).success;
    assert !(builtins.tryEval duplicateSlots.activationPackage.drvPath).success;
    assert (builtins.fromJSON c.xdg.configFile."hypr/hypr2d.json".text).monitorSlots."HEADLESS-1" == 2;
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
