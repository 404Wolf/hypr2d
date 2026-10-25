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
    gap = cfg.overview.gap;
  };
  lua = ''
    -- hypr2d: plugin loads before this section through HM's plugins option.
    if hl.plugin.hypr2d then
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList (name: grid: ''
        assert(hl.plugin.hypr2d.configure(${luaString name}, {columns=${toString grid.columns},rows=${toString grid.rows},wrap=${bool grid.wrap},stride=${toString cfg.grid.stride}}))
      '') cfg.grids)}
      hl.plugin.hypr2d._execute(${luaString ("overview-config " + builtins.toJSON theme)})
      hl.plugin.hypr2d._execute("swipe-enabled ${bool cfg.gestures.enable}")
      assert(hl.plugin.hypr2d.swipe_config({fingers=${toString cfg.gestures.fingers},distance=${toString cfg.gestures.distance},velocity=${toString cfg.gestures.velocity},natural=${bool cfg.gestures.natural},axis_lock=${toString cfg.gestures.axisLock},diagonal_width=${toString cfg.gestures.diagonalWidth},hysteresis=${toString cfg.gestures.hysteresis},hysteresis_distance=${toString cfg.gestures.hysteresisDistance},projection_decel=${toString cfg.gestures.projectionDecel},spring_stiffness=${toString cfg.gestures.springStiffness},sensitivity=${toString cfg.gestures.sensitivity}}))
      ${lib.optionalString cfg.overview.bind ''
        -- Replace an existing group-next chord instead of dispatching both.
        hl.unbind("SUPER + TAB")
        hl.bind("SUPER + TAB", hl.plugin.hypr2d.overview())
        hl.unbind("SUPER + ALT + TAB")
        hl.bind("SUPER + ALT + TAB", hl.plugin.hypr2d.parked())
      ''}
    end
  '';
  cli = "${cfg.package}/bin/hypr2d";
  monitorArg = lib.optionalString (cfg.waybar.monitor != null) " --monitor ${lib.escapeShellArg cfg.waybar.monitor}";
  colorArgs = lib.concatStrings (lib.mapAttrsToList (name: value:
    lib.optionalString (value != null) " --${name}-color ${lib.escapeShellArg value}"
  ) cfg.waybar.colors);
  module = {
    exec = "${cli} waybar --columns ${cfg.waybar.columns} --rows ${cfg.waybar.rows} --radius-x ${toString cfg.waybar.radiusX} --radius-y ${toString cfg.waybar.radiusY}${monitorArg}${colorArgs}";
    return-type = "json";
    restart-interval = lib.mkDefault 2;
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
    monitorSlots = mkOption {
      type = types.attrsOf (types.ints.between 0 999);
      default = {};
      description = "Permanent output-name to slot pins for new identities. Slot 0 is reserved for eDP-1; persisted slots cannot be reassigned.";
    };
    grid.stride = mkOption { type = types.ints.between 1 1000; default = 10; description = "Workspace numbering stride; must exceed every assigned monitor slot."; };
    grids = mkOption {
      default = { "0" = {}; };
      description = "Per-output grid configuration, keyed by output name or stable monitor slot. Unlisted outputs use 8x8 wrapping with the shared stride.";
      type = types.attrsOf (types.submodule { options = {
        columns = mkOption { type = types.ints.between 1 64; default = 8; description = "Grid columns."; };
        rows = mkOption { type = types.ints.between 1 64; default = 8; description = "Grid rows."; };
        wrap = mkOption { type = types.bool; default = true; description = "Wrap directional movement at edges."; };
      }; });
    };
    overview = {
      gap = mkOption { type = types.ints.between 0 32; default = 0; description = "Spacing between overview cells, including specials (before layout scaling)."; };
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
      sensitivity = mkOption { type = types.number; default = 1.0; description = "Positive swipe travel/velocity gain. All gesture thresholds remain in screen-space logical pixels after gain."; };
      fingers = mkOption { type = types.ints.between 2 5; default = 3; description = "Finger count claimed by hypr2d."; };
      distance = mkOption { type = types.ints.positive; default = 120; description = "Minimum projected per-axis commit distance in logical pixels; snapping also uses half the viewport size."; };
      velocity = mkOption { type = types.number; default = 0.8; description = "Minimum release speed used for inertia projection, in logical pixels per millisecond."; };
      natural = mkOption { type = types.bool; default = true; description = "Natural swipe direction."; };
      axisLock = mkOption { type = types.ints.positive; default = 5; description = "Euclidean distance before choosing a swipe sector, in logical pixels."; };
      diagonalWidth = mkOption { type = types.number; default = 15; description = "Diagonal sector half-width in degrees; cardinal sectors are wider."; };
      hysteresis = mkOption { type = types.number; default = 8; description = "Degrees inside a new sector required before re-targeting."; };
      hysteresisDistance = mkOption { type = types.number; default = 12; description = "Sustained travel inside the new sector before re-targeting, in logical pixels."; };
      projectionDecel = mkOption { type = types.number; default = 0.0006; description = "Release projection deceleration, in logical pixels per millisecond squared."; };
      springStiffness = mkOption { type = types.number; default = 576; description = "Critical spring stiffness, in inverse seconds squared (omega squared)."; };
    };
    waybar = {
      enable = mkEnableOption "the event-driven Waybar module";
      bar = mkOption { type = types.str; default = "mainBar"; description = "Named Waybar settings entry to extend."; };
      position = mkOption { type = types.enum [ "left" "center" "right" ]; default = "left"; description = "Bar module list to append to."; };
      monitor = mkOption { type = types.nullOr types.str; default = null; description = "Output name/id, or focused output when null."; };
      columns = mkOption { type = types.enum [ "centered" "full" ]; default = "centered"; description = "Centered radius or every column at fixed grid positions."; };
      rows = mkOption { type = types.enum [ "centered" "full" ]; default = "centered"; description = "Centered radius or every row at fixed grid positions."; };
      radiusX = mkOption { type = types.ints.between 0 4; default = 4; description = "Horizontal minimap radius; ignored for full columns."; };
      radiusY = mkOption { type = types.ints.between 0 4; default = 1; description = "Vertical minimap radius; ignored for full rows."; };
      placement = mkOption { type = types.enum [ "start" "end" ]; default = "end"; description = "Insert before or after existing modules in the selected bar section."; };
      colors = lib.genAttrs [ "current" "occupied" "empty" "urgent" ] (name: mkOption {
        type = types.nullOr (types.strMatching "#[0-9a-fA-F]{6}");
        default = null;
        description = "Minimap ${name} RGB colour; null preserves the theme-derived default (fixed red for urgent).";
      });
      style = mkOption { type = types.bool; default = true; description = "Append the black OneDark example CSS."; };
    };
  };
  config = mkIf cfg.enable {
    assertions = [
      { assertion = cfg.gestures.sensitivity > 0 && cfg.gestures.diagonalWidth > 0 && cfg.gestures.diagonalWidth < 22.5 && cfg.gestures.hysteresis >= 0 && cfg.gestures.hysteresis < cfg.gestures.diagonalWidth && cfg.gestures.hysteresisDistance > 0 && cfg.gestures.projectionDecel > 0 && cfg.gestures.springStiffness >= 1 && cfg.gestures.springStiffness <= 10000; message = "hypr2d requires positive sensitivity, diagonalWidth 0..22.5, hysteresis below diagonalWidth, positive hysteresisDistance/projectionDecel and springStiffness 1..10000."; }
      { assertion = lib.all (slot: slot < cfg.grid.stride) (builtins.attrValues cfg.monitorSlots); message = "hypr2d monitor slots must be less than the shared stride."; }
      { assertion = builtins.length (lib.unique (builtins.attrValues cfg.monitorSlots)) == builtins.length (builtins.attrValues cfg.monitorSlots); message = "hypr2d pinned monitor slots must be unique."; }
      { assertion = lib.all (name: if name == "eDP-1" then cfg.monitorSlots.${name} == 0 else cfg.monitorSlots.${name} != 0) (builtins.attrNames cfg.monitorSlots); message = "hypr2d slot 0 is reserved for eDP-1."; }
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
    xdg.configFile."hypr/hypr2d.json".text = builtins.toJSON { version = 1; inherit theme; monitorSlots = cfg.monitorSlots; grid = { inherit (cfg.grid) stride; monitors = cfg.grids; }; gestures = cfg.gestures; };
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
