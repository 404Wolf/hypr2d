# Import in your own Home Manager configuration; this file changes nothing itself.
{ inputs, ... }:
{
  imports = [ inputs.hypr2d.homeManagerModules.default ];
  programs.hypr2d = {
    enable = true;
    grid.stride = 10;
    grids = {
      "eDP-1" = { columns = 8; rows = 8; wrap = true; };
      "DP-1" = { columns = 8; rows = 8; wrap = true; };
    };
    overview = { fit = true; bind = true; };
    theme = {
      background = "#000000";
      cell = "#303030";
      border = "#353535";
      hover = "#4A4A4A";
      label = "#EEFFFF";
      current = "#82AAFF";
      target = "#FFCB6B";
      font = "sans-serif";
      fontSize = 16;
      rounding = 3;
    };
    gestures = { enable = true; fingers = 3; distance = 120; velocity = 0.8; natural = true; axisLock = 5; };
    waybar = { enable = true; bar = "mainBar"; position = "left"; placement = "start"; radiusX = 4; radiusY = 1; };
  };
}
