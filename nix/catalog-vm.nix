# Focused development runner; the complete integration suite remains the check.
{ pkgs, nested, compositor }:
let
  parent = pkgs.writeText "hypr2d-catalog-parent.lua" ''
    hl.monitor({output="",mode="1280x720@60",position="auto",scale=1})
    hl.config({xwayland={enabled=false}})
    hl.on("hyprland.start",function()
      hl.exec_cmd("${nested}/bin/hypr2d-nested --catalog-test >/tmp/hypr2d-catalog.log 2>&1; echo $? >/tmp/hypr2d-catalog-status")
    end)
  '';
in pkgs.testers.runNixOSTest {
  name = "hypr2d-catalog";
  nodes.machine = { pkgs, ... }: {
    environment.systemPackages = [ nested compositor ];
    hardware.graphics.enable = true;
    fonts.packages = [ pkgs.dejavu_fonts ];
    services.getty.autologinUser = "root";
    virtualisation = {cores=2;memorySize=4096;qemu.options=["-vga none -device virtio-gpu-pci"];};
    system.stateVersion="26.05";
  };
  testScript = ''
    machine.wait_for_unit("multi-user.target")
    machine.wait_until_succeeds("test -d /run/user/0")
    machine.succeed("rm -f /dev/dri/renderD128; ln -s card0 /dev/dri/renderD128")
    machine.send_chars("Hyprland --i-am-really-stupid --config ${parent} >/tmp/hypr2d-parent.log 2>&1\n")
    try:
        machine.wait_until_succeeds("test -f /tmp/hypr2d-catalog-status", timeout=300)
    finally:
        print(machine.execute("tail -c 30000 /tmp/hypr2d-catalog.log")[1])
        machine.copy_from_machine("/tmp/hypr2d-catalog-results")
    machine.succeed("grep '^0$' /tmp/hypr2d-catalog-status")
    machine.succeed("grep 'PASS catalogue:' /tmp/hypr2d-catalog.log")
  '';
}
