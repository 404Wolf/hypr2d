"""M4 rendering probe. All IPC and clients belong to the child PID below."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
from PIL import Image, ImageChops

config, plugin = sys.argv[1:]
benchmark = os.environ.get("HYPR2D_PROBE_BENCHMARK") == "1"
artifacts = Path(os.environ.get("HYPR2D_PROBE_RESULTS", "/tmp/hypr2d-overview-results"))
artifacts.mkdir(parents=True, exist_ok=True)
root = Path(os.environ["XDG_RUNTIME_DIR"]) / "hypr"
clients = []
instance = None
with (artifacts / "compositor.log").open("w+") as log:
    command = ["Hyprland", "--config", config]
    if os.geteuid() == 0:
        command.append("--i-am-really-stupid")
    child = subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True)
    try:
        def wait(test, label):
            deadline = time.monotonic() + 30
            while not test():
                assert child.poll() is None, f"child exited ({child.returncode}): {artifacts}"
                assert time.monotonic() < deadline, label
                time.sleep(0.05)
        def find_instance():
            global instance
            for lock in root.glob("*/hyprland.lock"):
                try:
                    if lock.read_text().splitlines()[0] == str(child.pid):
                        instance = lock.parent
                        return (instance / ".socket.sock").exists()
                except (OSError, IndexError):
                    pass
            return False
        wait(find_instance, "own IPC socket")
        def ctl(*args):
            return subprocess.run(["hyprctl", "-i", instance.name, *args], check=True, capture_output=True, text=True, timeout=60).stdout.strip()
        def ok(*args):
            result = ctl(*args)
            assert result == "ok", result
        def lua(expression):
            ok("dispatch", expression)
        def monitors():
            return json.loads(ctl("-j", "monitors"))
        def probe(action):
            return json.loads(ctl("hypr2d", "probe", action))
        wait(lambda: len(monitors()) == 1, "initial monitor")
        assert not ctl("configerrors"), ctl("configerrors")
        ok("eval", 'hl.config({input={follow_mouse=0}, animations={enabled=false}, cursor={invisible=true}})')
        ok("output", "create", "headless")
        wait(lambda: len(monitors()) == 2, "headless output")
        mons = monitors()
        for i, m in enumerate(mons):
            ok("eval", f'hl.monitor({{output="{m["name"]}",mode="1280x720@60",position="{i*1280}x0",scale=1}})')
        time.sleep(1)
        mons = monitors()
        env = dict(os.environ, WAYLAND_DISPLAY=(instance / "hyprland.lock").read_text().splitlines()[1], GDK_BACKEND="wayland")
        colors = ["701010", "107010", "101070", "707010", "701070", "107070", "703010", "307070", "504070", "407050"]
        def mapped():
            return json.loads(ctl("-j", "clients"))
        fixture_count = 0
        for panel, m in enumerate(mons):
            for x in range(4):
                ok("hypr2d", "goto", str(x), "0", str(m["id"]))
                app = f"hypr2d-probe-{panel}-{x}"
                clients.append(subprocess.Popen(["foot", "--config=/dev/null", f"--app-id={app}", "--override=cursor.blink=no", f"--override=colors-dark.background={colors[panel*5+x]}", "sh", "-c", f"printf '\\033[?25lmonitor {panel} workspace {x}'; sleep 600"], env=env, stdout=log, stderr=log, start_new_session=True))
                fixture_count += 1
                wait(lambda: len(mapped()) == fixture_count, "fixture map")
                target = 10*x + m["id"] + 1
                lua(f'hl.dsp.window.move({{window="class:^{app}$", workspace={target}, follow=false}})')
                wait(lambda: any(w["class"] == app and w["workspace"]["id"] == target and w["monitor"] == m["id"] for w in mapped()), "fixture placement")
            # A separately populated special on each output, then hide it.
            lua(f'hl.dsp.workspace.toggle_special("probe-{panel}")')
            app = f"hypr2d-probe-special-{panel}"
            clients.append(subprocess.Popen(["foot", "--config=/dev/null", f"--app-id={app}", "--override=cursor.blink=no", f"--override=colors-dark.background={colors[panel*5+4]}", "sh", "-c", f"printf '\\033[?25lspecial {panel}'; sleep 600"], env=env, stdout=log, stderr=log, start_new_session=True))
            fixture_count += 1
            wait(lambda: len(mapped()) == fixture_count, "special fixture map")
            lua(f'hl.dsp.window.move({{window="class:^{app}$", workspace="special:probe-{panel}", follow=false}})')
            lua(f'hl.dsp.workspace.toggle_special("probe-{panel}")')
            lua(f'hl.dsp.workspace.move({{workspace="special:probe-{panel}", monitor="{m["name"]}"}})')
            wait(lambda: any(w["class"] == app and w["workspace"]["name"] == f"special:probe-{panel}" and w["monitor"] == m["id"] for w in mapped()), "special placement")
            ok("hypr2d", "goto", "0", "0", str(m["id"]))
        time.sleep(2)
        def screenshot(tag):
            if benchmark:
                return
            for i, m in enumerate(mons):
                shot = subprocess.Popen(["grim", "-o", m["name"], str(artifacts / f"{tag}-{i}.png")], env=env, start_new_session=True)
                clients.append(shot)
                time.sleep(0.15)
                # Ask this child's output to render after screencopy subscribes.
                ok("eval", 'hl.config({debug={damage_tracking=0}})')
                shot.wait(timeout=10)
                assert shot.returncode == 0
                ok("eval", 'hl.config({debug={damage_tracking=2}})')
        def snapshot():
            # Focus and active normal/special workspace identities must survive.
            return [{k:m[k] for k in ("id", "activeWorkspace", "specialWorkspace", "focused")} for m in monitors()]
        ok("dismissnotify", "-1")
        time.sleep(0.3)
        baseline = snapshot()
        small = None
        if not benchmark:
            screenshot("before")
            small = probe("open")
            assert len(small["cells"]) == 10, small
            assert small["restored_captures"] == 10, small
            assert snapshot() == baseline
            time.sleep(0.5)
            screenshot("overlay-small")
            # Prove each hidden normal and special contains its own fixture, rather
            # than accepting merely allocated or partially clipped textures.
            sheet = Image.open(artifacts / "overlay-small-0.png").convert("RGB")
            for panel in range(2):
                for cell in range(5):
                    column, row = (cell, 0) if cell < 4 else (0, 9)
                    px = round((0.02 + panel*0.48 + (column+0.47)*0.06)*sheet.width)
                    py = round((0.08 + (row+0.47)*0.06*mons[panel]["height"]/mons[panel]["width"])*sheet.width)
                    expected = tuple(bytes.fromhex(colors[panel*5+cell]))
                    actual = sheet.getpixel((px,py))
                    assert max(abs(a-b) for a,b in zip(actual,expected)) <= 3, (panel,cell,actual,expected)
            small_close = probe("close")
            assert small_close["deleted_gl_objects"], small_close
            time.sleep(0.5)
            screenshot("after-small")
            def compare(tag, reference="before"):
                diffs = []
                for i in range(2):
                    before = Image.open(artifacts / f"{reference}-{i}.png").convert("RGB")
                    after = Image.open(artifacts / f"{tag}-{i}.png").convert("RGB")
                    diff = ImageChops.difference(before, after)
                    count = sum(pixel != (0,0,0) for pixel in diff.get_flattened_data())
                    diffs.append(count)
                    diff.save(artifacts / f"diff-{tag}-{i}.png")
                assert diffs == [0,0], (tag, diffs)
                return diffs
            compare("after-small")
        # Warm up allocation/shader caches, then look for a plateau, not RSS=0.
        runs = []
        def process_memory():
            status = Path(f"/proc/{child.pid}/status").read_text()
            rss = next(line for line in status.splitlines() if line.startswith("VmRSS:"))
            fdinfo = {}
            for path in Path(f"/proc/{child.pid}/fdinfo").glob("*"):
                try:
                    lines = [line for line in path.read_text().splitlines() if line.startswith(("drm-memory-", "drm-resident-", "drm-total-", "drm-shared-", "drm-client-id", "drm-driver"))]
                    if lines:
                        fdinfo[path.name] = lines
                except OSError:
                    pass
            return {"rss":rss, "drm_fdinfo": fdinfo, "fds":len(list(Path(f"/proc/{child.pid}/fd").iterdir()))}
        before_full_memory = process_memory()
        for cycle in range(1 if benchmark else 25):
            opened = probe("full")
            assert len(opened["cells"]) == 130, opened
            assert opened["restored_captures"] == 130, opened
            assert snapshot() == baseline
            if cycle == 0:
                time.sleep(0.3)
                screenshot("overlay-full")
            live_memory = process_memory()
            closed = probe("close")
            assert not closed["active"] and closed["deleted_gl_objects"], closed
            runs.append({"cycle":cycle,"capture":opened,"open_memory":live_memory,"closed_memory":process_memory()})
        if benchmark:
            (artifacts / "results.json").write_text(json.dumps({"instance":instance.name,"before_full_memory":before_full_memory,"runs":runs}, indent=2))
            print(f"PASS hardware capture benchmark: one 130-cell capture; {artifacts}")
            sys.exit(0)
        # Ignore allocator warm-up; bound retained host memory and descriptors.
        warm = runs[5:]
        rss_kib = [int(r["closed_memory"]["rss"].split()[1]) for r in warm]
        assert max(rss_kib) - min(rss_kib) < 16*1024, rss_kib
        assert len({r["closed_memory"]["fds"] for r in warm}) == 1
        time.sleep(0.5)
        screenshot("after-repeated")
        diffs = compare("after-repeated")
        # Also capture while a special is actually shown, then restore it.
        lua('hl.dsp.workspace.toggle_special("probe-1")')
        shown = snapshot()
        assert any(m["specialWorkspace"]["name"] for m in shown)
        screenshot("shown-before")
        probe("open")
        assert snapshot() == shown
        assert probe("close")["deleted_gl_objects"]
        screenshot("shown-after")
        compare("shown-after", "shown-before")
        lua('hl.dsp.workspace.toggle_special("probe-1")')
        # One IPC eval performs a real animated transition followed immediately
        # by the capture, without allowing the animation to settle first.
        ok("eval", 'hl.config({animations={enabled=true}}); hl.dispatch(hl.plugin.hypr2d.move("right")); hl.plugin.hypr2d._execute("probe open")')
        assert probe("stats")["restored_captures"] == 10
        assert probe("close")["deleted_gl_objects"]
        wait(lambda: all(abs(v) < 0.01 for m in json.loads(ctl("hypr2d", "state", "-j"))["monitors"] for v in m["offset"]), "animation resumes after capture")
        # Unload with the overlay still open: no custom vtable may survive dlclose.
        probe("full")
        ok("plugin", "unload", plugin)
        time.sleep(0.5)
        ok("plugin", "load", plugin)
        results = {"instance":instance.name,"small":small,"runs":runs,"screenshot_changed_pixels":diffs,"final_memory":process_memory()}
        (artifacts / "results.json").write_text(json.dumps(results, indent=2))
        print(f"PASS M4 rendering prototype: 2 outputs, normal/special capture, 25 full cycles, deleted GL objects, exact screenshots; {artifacts}")
    finally:
        if child.poll() is None and instance:
            try:
                ctl("dispatch", "hl.dsp.exit()")
                child.wait(timeout=5)
            except (subprocess.SubprocessError, OSError):
                pass
        for process in [*clients, child]:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=10)
