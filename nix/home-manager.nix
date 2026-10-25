flake:
{ config, lib, pkgs, ... }:
let
  inherit (lib) mkOption mkEnableOption mkIf types;
  cfg = config.programs.hypr2d;
  luaString = builtins.toJSON;
  bool = b: if b then "true" else "false";
  colour = default: mkOption { type = types.strMatching "#[0-9a-fA-F]{6}"; inherit default; description = "RGB overview colour."; };
  theme = {
    inherit (cfg.theme) background cell border hover current target label font rounding;
    font_size = cfg.theme.fontSize;
    fit = cfg.overview.fit;
  };
  lua = ''
    -- hypr2d: plugin loads before this section through HM's plugins option.
    if hl.plugin.hypr2d then
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList (name: grid: ''
        assert(hl.plugin.hypr2d.configure(${luaString name}, {columns=${toString grid.columns},rows=${toString grid.rows},wrap=${bool grid.wrap},stride=${toString cfg.grid.stride}}))
      '') cfg.grids)}
      hl.plugin.hypr2d._execute(${luaString ("overview-config " + builtins.toJSON theme)})
      hl.plugin.hypr2d._execute("swipe-enabled ${bool cfg.gestures.enable}")
      assert(hl.plugin.hypr2d.swipe_config({fingers=${toString cfg.gestures.fingers},distance=${toString cfg.gestures.distance},velocity=${toString cfg.gestures.velocity},natural=${bool cfg.gestures.natural},axis_lock=${toString cfg.gestures.axisLock}}))
      ${lib.optionalString cfg.overview.bind ''
        -- Replace an existing group-next chord instead of dispatching both.
        hl.unbind("SUPER + TAB")
        hl.bind("SUPER + TAB", hl.plugin.hypr2d.overview())
      ''}
    end
  '';
  cli = "${cfg.package}/bin/hypr2d";
  monitorArg = lib.optionalString (cfg.waybar.monitor != null) " --monitor ${lib.escapeShellArg cfg.waybar.monitor}";
  module = {
    exec = "${cli} waybar --radius-x ${toString cfg.waybar.radiusX} --radius-y ${toString cfg.waybar.radiusY}${monitorArg}";
    return-type = "json";
    format = "{text}";
    escape = false;
    exec-on-event = false;
    on-click = "${cli} move right${monitorArg}";
    on-click-right = "${cli} move left${monitorArg}";
    on-click-middle = "${cli} goto 0 0${monitorArg}";
    on-scroll-up = "${cli} move up${monitorArg}";
    on-scroll-down = "${cli} move down${monitorArg}";
    tooltip = true;
  };
in {
  options.programs.hypr2d = {
    enable = mkEnableOption "hypr2d for Lua Hyprland 0.56";
    package = mkOption { type = types.package; default = flake.packages.${pkgs.stdenv.hostPlatform.system}.plugin; description = "Plugin and CLI built against the compositor flake input."; };
    grid.stride = mkOption { type = types.ints.between 1 1000; default = 10; description = "Workspace numbering stride; must exceed every monitor id."; };
    grids = mkOption {
      default = { "0" = {}; };
      description = "Per-output grid configuration, keyed by output name or monitor id. Unlisted outputs use 8x8 wrapping with the shared stride.";
      type = types.attrsOf (types.submodule { options = {
        columns = mkOption { type = types.ints.between 1 64; default = 8; description = "Grid columns."; };
        rows = mkOption { type = types.ints.between 1 64; default = 8; description = "Grid rows."; };
        wrap = mkOption { type = types.bool; default = true; description = "Wrap directional movement at edges."; };
      }; });
    };
    overview = {
      fit = mkOption { type = types.bool; default = true; description = "Show used bounds plus a one-cell margin; a toggles the full grid."; };
      bind = mkOption { type = types.bool; default = false; description = "Replace SUPER+TAB with the overview bind."; };
    };
    theme = {
      background = colour "#000000";
      cell = colour "#303030";
      border = colour "#353535";
      hover = colour "#4A4A4A";
      current = colour "#82AAFF";
      target = colour "#FFCB6B";
      label = colour "#EEFFFF";
      font = mkOption { type = types.str; default = "sans-serif"; description = "Pango font family."; };
      fontSize = mkOption { type = types.ints.between 10 40; default = 16; description = "Label size in output pixels."; };
      rounding = mkOption { type = types.ints.between 0 4; default = 3; description = "Cell corner radius in output pixels."; };
    };
    gestures = {
      enable = mkOption { type = types.bool; default = true; description = "Claim tracked workspace swipes. Remove conflicting native/execGesture bindings when wiring this module."; };
      fingers = mkOption { type = types.ints.between 2 5; default = 3; description = "Finger count claimed by hypr2d."; };
      distance = mkOption { type = types.ints.positive; default = 120; description = "Commit distance in logical pixels."; };
      velocity = mkOption { type = types.number; default = 0.8; description = "Commit velocity in logical pixels per millisecond."; };
      natural = mkOption { type = types.bool; default = true; description = "Natural swipe direction."; };
      axisLock = mkOption { type = types.ints.positive; default = 5; description = "Distance before axis lock, in logical pixels."; };
    };
    waybar = {
      enable = mkEnableOption "the event-driven Waybar module";
      bar = mkOption { type = types.str; default = "mainBar"; description = "Named Waybar settings entry to extend."; };
      position = mkOption { type = types.enum [ "left" "center" "right" ]; default = "left"; description = "Bar module list to append to."; };
      monitor = mkOption { type = types.nullOr types.str; default = null; description = "Output name/id, or focused output when null."; };
      radiusX = mkOption { type = types.ints.between 0 4; default = 4; description = "Horizontal minimap radius."; };
      radiusY = mkOption { type = types.ints.between 0 4; default = 1; description = "Vertical minimap radius."; };
      placement = mkOption { type = types.enum [ "start" "end" ]; default = "end"; description = "Insert before or after existing modules in the selected bar section."; };
      style = mkOption { type = types.bool; default = true; description = "Append the black OneDark example CSS."; };
    };
  };
  config = mkIf cfg.enable {
    assertions = [
      { assertion = config.wayland.windowManager.hyprland.configType == "lua"; message = "hypr2d requires Hyprland configType = lua."; }
      { assertion = cfg.gestures.axisLock <= cfg.gestures.distance && cfg.gestures.velocity >= 0; message = "hypr2d gesture axisLock must not exceed distance; velocity must be non-negative."; }
      { assertion = cfg.theme.font != "" && builtins.stringLength cfg.theme.font <= 128; message = "hypr2d theme font must be nonempty and at most 128 bytes."; }
      { assertion = cfg.grids != {}; message = "hypr2d needs at least one configured grid to establish the shared stride."; }
    ];
    home.packages = [ cfg.package ];
    wayland.windowManager.hyprland = {
      enable = lib.mkDefault true;
      configType = lib.mkDefault "lua";
      package = lib.mkDefault flake.inputs.hyprland.packages.${pkgs.stdenv.hostPlatform.system}.hyprland;
      plugins = [ cfg.package ];
      extraConfig = lib.mkAfter lua;
    };
    xdg.configFile."hypr/hypr2d.json".text = builtins.toJSON { version = 1; inherit theme; grid = { inherit (cfg.grid) stride; monitors = cfg.grids; }; gestures = cfg.gestures; };
    programs.waybar = mkIf cfg.waybar.enable {
      enable = lib.mkDefault true;
      settings.${cfg.waybar.bar} = {
        "modules-${cfg.waybar.position}" = (if cfg.waybar.placement == "start" then lib.mkBefore else lib.mkAfter) [ "custom/hypr2d" ];
        "custom/hypr2d" = module;
      };
      style = lib.mkIf cfg.waybar.style (lib.mkAfter (builtins.replaceStrings [ "#abb2bf" "#4b5263" "#61afef" "#e5c07b" ] [ cfg.theme.label cfg.theme.border cfg.theme.current cfg.theme.target ] (builtins.readFile ../examples/waybar.css)));
    };
  };
}
