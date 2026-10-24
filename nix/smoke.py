"""Test only the nested child we launch, identified by its PID lock file."""
import concurrent.futures
import json
import socket
import signal
import os
from pathlib import Path
import subprocess
import sys
import time

config, plugin = sys.argv[1:]
root = Path(os.environ["XDG_RUNTIME_DIR"]) / "hypr"
with open(Path(config).parent / "compositor.log", "w+") as output:
    command_line = ["Hyprland", "--config", config]
    # VM-only root login authenticates dumb-buffer allocation on the virtual GPU.
    if os.geteuid() == 0:
        command_line.append("--i-am-really-stupid")
    child = subprocess.Popen(command_line, stdout=output, stderr=output, start_new_session=True)
    clients = []
    try:
        deadline = time.monotonic() + 30
        instance = None
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError(f"nested compositor exited: {child.returncode}")
            for lock in root.glob("*/hyprland.lock"):
                try:
                    if lock.read_text().splitlines()[0] == str(child.pid):
                        instance = lock.parent
                except (OSError, IndexError):
                    continue
            if instance and (instance / ".socket.sock").exists():
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("nested compositor IPC did not become ready")

        def ctl(*args, check=True):
            result = subprocess.run(["hyprctl", "-i", instance.name, *args],
                                    check=check, text=True, capture_output=True, timeout=10)
            return result.stdout.strip()

        errors = ctl("configerrors")
        assert not errors, errors
        assert "hypr2d" in ctl("plugin", "list")
        log = instance / "hyprland.log"
        assert "hypr2d M0: C++ -> Rust -> C++: 41 -> 42" in log.read_text()
        assert ctl("eval", 'savedHypr2dMove = hl.plugin.hypr2d.move("right")') == "ok"
        assert ctl("plugin", "unload", plugin) == "ok"
        assert "hypr2d" not in ctl("plugin", "list")
        assert ctl("plugin", "load", plugin) == "ok"
        assert "hypr2d" in ctl("plugin", "list")
        def wait_for(test, message):
            deadline = time.monotonic() + 10
            while not test():
                assert time.monotonic() < deadline, message
                time.sleep(0.02)

        def state():
            return json.loads(ctl("hypr2d", "state", "-j"))["monitors"]

        def command(*args):
            result = ctl("hypr2d", *args)
            assert result == "ok", result

        def lua(expression):
            result = ctl("dispatch", expression)
            assert result == "ok", result

        def cell(mid):
            return next(m for m in state() if m["id"] == mid)

        # Reload can replace the Lua environment; a stale bind must never crash.
        ctl("dispatch", "savedHypr2dMove", check=False)
        animations_before = ctl("-j", "animations")
        first = state()[0]
        mid = first["id"]
        command("goto", "0", "0", str(mid))
        assert cell(mid)["workspace"] == mid + 1
        # socket2 is connected before the move: no missed initial event.
        events = socket.socket(socket.AF_UNIX)
        events.settimeout(5)
        events.connect(str(instance / ".socket2.sock"))
        lua('hl.plugin.hypr2d.move("left")')
        assert cell(mid)["workspace"] == 71 + mid
        data = b""
        while f"hypr2d>>cell>>{mid},7,0\n".encode() not in data:
            data += events.recv(65536)
        events.close()
        lua('hl.plugin.hypr2d.move("up")')
        assert cell(mid)["workspace"] == 631 + mid
        command("goto", "0", "0")
        command("move", "down")
        offset = cell(mid)["offset"]
        assert offset[0] == 0 and offset[1] >= 0, offset
        command("move", "right")
        offset = cell(mid)["offset"]
        assert offset[1] == 0 and offset[0] >= 0, offset
        command("goto", "0", "0")
        # Requests overlap while animations are still running; Hyprland serializes plans.
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            list(pool.map(lambda d: command("move", d), ["right"] * 40 + ["down"] * 17))
        assert cell(mid)["cell"] == {"x": 0, "y": 1}
        assert cell(mid)["workspace"] == 81 + mid
        assert ctl("output", "create", "headless") == "ok"
        wait_for(lambda: len(state()) == 2, "second nested output did not appear")
        second = next(m for m in state() if m["id"] != mid)
        sid = second["id"]
        command("goto", "2", "3", str(sid))
        command("goto", "7", "7", str(mid))
        lua('hl.plugin.hypr2d.move("right", {all=true})')
        assert cell(mid)["workspace"] == 561 + mid
        assert cell(sid)["workspace"] == 271 + sid
        assert cell(mid)["focused"] and not cell(sid)["focused"]
        lua('hl.plugin.hypr2d.goto_cell(0, 0)')
        result = ctl("eval", f'return hl.plugin.hypr2d.configure("{first["name"]}", {{columns=8, rows=8, wrap=false}})')
        assert result == "ok", result
        command("move", "left")
        assert cell(mid)["workspace"] == mid + 1
        command("configure", first["name"], "8", "8", "true", "10")
        # A real nested client proves carry changes the window and follows it.
        env = dict(os.environ)
        env["WAYLAND_DISPLAY"] = (instance / "hyprland.lock").read_text().splitlines()[1]
        clients.append(subprocess.Popen(["foot", "--config=/dev/null", "--app-id=hypr2d-test", "sh", "-c", "sleep 120"], env=env, stdout=output, stderr=output, start_new_session=True))
        def test_windows():
            return [w for m in state() for c in m["cells"] for w in c["windows"] if w["app_id"] == "hypr2d-test"]
        wait_for(lambda: len(test_windows()) == 1, "nested test client did not map")
        lua('hl.plugin.hypr2d.carry("down")')
        assert cell(mid)["workspace"] == 81 + mid
        carried = next(c for c in cell(mid)["cells"] if c["workspace"] == 81 + mid)
        assert any(w["app_id"] == "hypr2d-test" for w in carried["windows"])
        assert ctl("hypr2d", "goto", "8", "0", check=False).startswith("error:")
        assert cell(mid)["workspace"] == 81 + mid
        assert ctl("-j", "animations") == animations_before
        print("PASS M1: exact numbering/wrap, Lua, IPC/events, axes, 57 rapid moves, two-monitor all/focus, no-wrap, carry")
        ctl("dispatch", "hl.dsp.exit()")
        child.wait(timeout=10)
        assert child.returncode == 0, child.returncode
        assert log.read_text().count("hypr2d M0: C++ -> Rust -> C++: 41 -> 42") == 2
        print(f"PASS: nested {instance.name}: config, preload, Rust round trip, unload/reload, clean exit")
    except BaseException:
        output.flush()
        print(Path(output.name).read_text(), file=sys.stderr)
        raise
    finally:
        if child.poll() is None and instance:
            try:
                ctl("dispatch", "hl.dsp.exit()")
                child.wait(timeout=5)
            except (subprocess.SubprocessError, OSError):
                pass
        for client in clients:
            if client.poll() is None:
                client.terminate()
            client.wait(timeout=10)
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()

        # Also remove crash-report helper forks on failed tests.
        for process in [child, *clients]:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
