"""Test only the nested child we launch, identified by its PID lock file."""
import os
from pathlib import Path
import subprocess
import sys
import time

config, plugin = sys.argv[1:]
root = Path(os.environ["XDG_RUNTIME_DIR"]) / "hypr"
with open(Path(config).parent / "compositor.log", "w+") as output:
    child = subprocess.Popen(["Hyprland", "--config", config], stdout=output, stderr=output)
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

        def ctl(*args):
            result = subprocess.run(["hyprctl", "-i", instance.name, *args],
                                    check=True, text=True, capture_output=True, timeout=10)
            return result.stdout.strip()

        errors = ctl("configerrors")
        assert not errors, errors
        assert "hypr2d" in ctl("plugin", "list")
        log = instance / "hyprland.log"
        assert "hypr2d M0: C++ -> Rust -> C++: 41 -> 42" in log.read_text()
        assert ctl("plugin", "unload", plugin) == "ok"
        assert "hypr2d" not in ctl("plugin", "list")
        assert ctl("plugin", "load", plugin) == "ok"
        assert "hypr2d" in ctl("plugin", "list")
        # The logger flushes asynchronously relative to the IPC response.
        deadline = time.monotonic() + 5
        while log.read_text().count("hypr2d M0: C++ -> Rust -> C++: 41 -> 42") < 2:
            assert time.monotonic() < deadline, "reload bridge log did not flush"
            time.sleep(0.05)
        ctl("dispatch", "hl.dsp.exit()")
        child.wait(timeout=10)
        assert child.returncode == 0, child.returncode
        print(f"PASS: nested {instance.name}: config, preload, Rust round trip, unload/reload, clean exit")
    except BaseException:
        output.flush()
        print(Path(output.name).read_text(), file=sys.stderr)
        raise
    finally:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
