"""Test only the nested child we launch, identified by its PID lock file."""
import concurrent.futures
import json
import socket
import signal
import select
import shlex
import os
from pathlib import Path
import subprocess
import sys
import time
from PIL import Image

config, plugin, cli, examples = sys.argv[1:]
root = Path(os.environ["XDG_RUNTIME_DIR"]) / "hypr"
with open(Path(config).parent / "compositor.log", "w+") as output:
    command_line = ["Hyprland", "--config", config]
    # VM-only root login authenticates dumb-buffer allocation on the virtual GPU.
    if os.geteuid() == 0:
        command_line.append("--i-am-really-stupid")
    child = subprocess.Popen(command_line, stdout=output, stderr=output, start_new_session=True)
    clients = []
    expected_loads = 1
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
        expected_loads += 1
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
        wait_for(lambda: len(state()) > 0, "initial output did not become ready")
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
        def gesture():
            return json.loads(ctl("hypr2d", "state", "-j"))["swipe"]

        def synthetic(phase, a=0, b=0, claimed=True):
            def value(v):
                return str(v).lower() if isinstance(v, bool) else str(v)
            expression = f'return hl.plugin.hypr2d._test_swipe("{phase}", {value(a)}, {value(b)}, {value(claimed)})'
            result = ctl("eval", expression)
            assert result == "ok", result

        def settle():
            wait_for(lambda: gesture() is None, "swipe spring did not finish")
            assert all(abs(v) < 0.01 for v in cell(mid)["offset"])

        command("goto", "0", "0", str(mid))
        synthetic("begin", 2, claimed=False)
        synthetic("update", 0, -300, claimed=False)
        synthetic("end", False, claimed=False)
        assert gesture() is None
        synthetic("begin", 3)
        synthetic("update", -40, 2)
        assert cell(mid)["offset"] == [-40, 0]
        synthetic("update", 100, 150)
        assert cell(mid)["offset"] == [60, 0]
        assert gesture()["neighbor"] == 71 + mid
        synthetic("end", True)
        assert cell(mid)["workspace"] == 1 + mid
        settle()
        synthetic("begin", 3)
        synthetic("update", 0, -150)
        assert cell(mid)["offset"] == [0, -150]
        assert gesture()["neighbor"] == 81 + mid
        synthetic("end", False)
        assert cell(mid)["workspace"] == 81 + mid
        settle()
        command("goto", "0", "0")
        command("configure", first["name"], "8", "8", "false", "10")
        synthetic("begin", 3)
        synthetic("update", 400, 0)
        assert 0 < cell(mid)["offset"][0] < 400
        assert gesture()["neighbor"] == 0
        synthetic("end", False)
        settle()
        assert cell(mid)["workspace"] == 1 + mid
        command("configure", first["name"], "8", "8", "true", "10")
        # A keyboard move must cancel an in-progress gesture and clear its offsets.
        synthetic("begin", 3)
        synthetic("update", -40, 0)
        command("move", "down")
        assert gesture() is None
        synthetic("end", True)
        assert cell(mid)["workspace"] == 81 + mid
        # Unloading during tracking releases force-rendering and all callbacks.
        synthetic("begin", 3)
        synthetic("update", -40, 0)
        assert ctl("plugin", "unload", plugin) == "ok"
        assert ctl("plugin", "load", plugin) == "ok"
        expected_loads += 1
        assert gesture() is None
        assert cell(mid)["offset"] == [0, 0]
        command("goto", "0", "0", str(sid))
        assert cell(sid)["focused"]
        synthetic("begin", 3)
        synthetic("update", -40, 0)
        assert ctl("output", "remove", second["name"]) == "ok"
        wait_for(lambda: len(state()) == 1, "headless monitor removal did not finish")
        assert gesture() is None
        synthetic("end", True)
        print("PASS M2: production event listeners, cancellation, 1:1 offsets, axis lock/reversal, natural direction, commit/bounce, rubber band, interruption/unload")
        def cli_command(*args):
            result = subprocess.run([cli, "--instance", instance.name, *args], check=True, capture_output=True, text=True, timeout=10)
            return result.stdout.strip()

        cli_state = json.loads(cli_command("state"))
        assert cli_state["version"] == 1
        cli_command("goto", "0", "0")
        module = subprocess.Popen([cli, "--instance", instance.name, "waybar", "--monitor", first["name"]], stdout=subprocess.PIPE, stderr=output, start_new_session=True)
        clients.append(module)
        buffer = b""
        def record_when(predicate, timeout=10):
            # The test script's top-level buffer is intentionally shared here.
            global buffer
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                while b"\n" in buffer:
                    line, buffer = buffer.split(b"\n", 1)
                    record = json.loads(line)
                    if predicate(record):
                        return record
                ready, _, _ = select.select([module.stdout], [], [], max(0, deadline-time.monotonic()))
                assert ready, "Waybar stream did not respond to an event"
                data = os.read(module.stdout.fileno(), 65536)
                assert data, "Waybar module exited unexpectedly"
                buffer += data
            raise AssertionError("Waybar record predicate did not match")

        initial = record_when(lambda r: r["alt"] == "0,0")
        assert "current" in initial["class"] and "empty" in initial["class"]
        assert "foot" in initial["tooltip"]
        assert "occupied" in initial["class"]
        assert not buffer
        assert not select.select([module.stdout], [], [], 0.2)[0], "Waybar emitted duplicate idle records"
        cli_command("move", "right", "--monitor", first["name"])
        record_when(lambda r: r["alt"] == "1,0")
        assert cell(mid)["workspace"] == 11 + mid
        # Window titles arrive on native socket2 events and must be markup-escaped.
        title_env = dict(env)
        title_client = subprocess.Popen(["foot", "--config=/dev/null", "--app-id=hypr2d-title-test", "sh", "-c", "printf '\033]2;hypr2d <title> & text\007'; sleep 120"], env=title_env, stdout=output, stderr=output, start_new_session=True)
        clients.append(title_client)
        titled = record_when(lambda r: "hypr2d &lt;title&gt; &amp; text" in r["tooltip"])
        assert "<title>" not in titled["tooltip"]
        # A native move, independent of hypr2d's own events, must move the dot.
        state_before_move=json.loads(cli_command("state"))
        view=next(m for m in state_before_move["monitors"] if m["id"]==mid)
        destination=next(c["workspace"] for c in view["cells"] if not c["windows"] and c["y"]==view["cell"]["y"] and c["x"]!=view["cell"]["x"])
        move_expression=f'hl.dsp.window.move({{window="class:^hypr2d-title-test$",workspace={destination},follow=false}})'
        assert ctl("dispatch",move_expression)=="ok"
        moved=record_when(lambda r:r["text"]!=titled["text"] and "hypr2d &lt;title&gt;" in r["tooltip"])
        assert f"workspace {destination}</b>\n" in moved["tooltip"],moved
        title_client.terminate()
        title_client.wait(timeout=10)
        record_when(lambda r: "hypr2d &lt;title&gt;" not in r["tooltip"])
        # Exercise all native/plugin paths, measuring actual streamed lines.
        exec(Path(os.environ["HYPR2D_LATENCY_HELPER"]).read_text())
        command("goto","0","0",str(mid))
        assert ctl("output","create","headless")=="ok"
        wait_for(lambda:len(state())==2,"latency second monitor")
        second=next(m for m in state() if m["id"]!=mid)
        latency = MinimapLatency(instance, cli, output)
        command("goto", "0", "0", str(mid))
        synthetic("begin", 3)
        synthetic("update", -300, 0)
        latency.expect("tracked-swipe", lambda:synthetic("end", False), lambda r:r["alt"]=="1,0",event="workspace")
        settle()
        command("goto","0","1",str(mid))
        latency.expect("carry", lambda:lua('hl.plugin.hypr2d.carry("right")'), lambda r:r["alt"]=="1,1",event="movewindowv2")
        # Current carried window supplies an unambiguous title on a hidden cell.
        win=next(w for w in json.loads(ctl("-j","clients")) if w["class"]=="hypr2d-test")
        address=win["address"]
        target=21+mid
        latency.expect("external-workspace-focus", lambda:lua(f'hl.dsp.focus({{workspace={target}}})'), lambda r:r["alt"]=="2,0",event="workspace")
        latency.expect("focused-monitor", lambda:lua(f'hl.dsp.focus({{monitor="{second["name"]}"}})'), lambda r:second["name"] in r["tooltip"],event="focusedmon")
        lua(f'hl.dsp.focus({{monitor="{first["name"]}"}})')
        # Move into/out of special changes occupancy in the grid tooltip.
        latency.expect("window-to-special", lambda:lua(f'hl.dsp.window.move({{window="address:{address}",workspace="special:latency",follow=false}})'),
            lambda r:"hypr2d-test" not in r["tooltip"], event="movewindowv2")
        latency.expect("window-from-special", lambda:lua(f'hl.dsp.window.move({{window="address:{address}",workspace={81+mid},follow=false}})'),
            lambda r:"hypr2d-test" in r["tooltip"], event="movewindowv2")
        # A new client is assigned to a different cell before mapping.
        ok_rule=ctl("eval",f'hl.window_rule({{match={{class="^hypr2d-latency-open$"}},workspace="{31+mid} silent"}})')
        assert ok_rule=="ok",ok_rule
        def map_latency():
            client=subprocess.Popen(["foot","--config=/dev/null","--app-id=hypr2d-latency-open","sh","-c","sleep 120"],env=env,stdout=output,stderr=output,start_new_session=True)
            clients.append(client)
        latency.expect("openwindow-other-cell",map_latency,lambda r:"hypr2d-latency-open" in r["tooltip"],event="openwindow")
        opened=next(w for w in json.loads(ctl("-j","clients")) if w["class"]=="hypr2d-latency-open")
        latency.expect("closewindow-other-cell",lambda:lua(f'hl.dsp.window.close({{window="address:{opened["address"]}"}})'),
            lambda r:"hypr2d-latency-open" not in r["tooltip"],event="closewindow")
        Path("/tmp/hypr2d-waybar-results").mkdir(exist_ok=True)
        Path("/tmp/hypr2d-waybar-results/latency.json").write_text(json.dumps(latency.results,indent=2))
        latency.close()
        assert ctl("output","remove",second["name"])=="ok"
        command("goto","1","0",str(mid))
        # Drain the first stream to its restored cell before lifecycle checks.
        buffer=b""
        while select.select([module.stdout],[],[],.08)[0]:os.read(module.stdout.fileno(),65536)
        assert ctl("plugin", "unload", plugin) == "ok"
        record_when(lambda r: "unavailable" in r["class"])
        assert module.poll() is None
        assert ctl("plugin", "load", plugin) == "ok"
        expected_loads += 1
        record_when(lambda r: r["alt"] == "1,0")
        module.terminate()
        module.wait(timeout=10)
        # Real Waybar: no explicit height. Compare the user's 13px/min-height:0
        # single-line bar with/without the minimap, then inspect all three rows.
        artifacts=Path("/tmp/hypr2d-waybar-results");artifacts.mkdir(exist_ok=True)
        bar_env=dict(env,GDK_BACKEND="wayland",HYPRLAND_INSTANCE_SIGNATURE=instance.name)
        bar_style=artifacts/"style.css"
        bar_style.write_text('window#waybar { border: none; border-radius: 0; font-family: "monospace"; font-size: 13px; min-height: 0; background: black; color: white; }\n#cpu { padding: 0 8px; }\n#custom-hypr2d { font-family: monospace; padding: 0 8px; margin: 0 4px; }\n')
        def layers_for_bar():
            layers=json.loads(ctl("-j","layers"))
            return [layer for level in layers.get(first["name"],{}).get("levels",{}).values() for layer in level if layer["namespace"]=="waybar"]
        heights=[]
        for enabled in [False,True]:
            bar_config=json.loads((Path(examples)/"waybar.jsonc").read_text())
            bar_config["output"]=first["name"]
            bar_config["modules-left"]=["custom/hypr2d","cpu"] if enabled else ["cpu"]
            bar_config["cpu"]={"format":"CPU"}
            bar_config["custom/hypr2d"]["exec"]=shlex.join([cli,"--instance",instance.name,"waybar","--monitor",first["name"]])
            bar_path=artifacts/("with.json" if enabled else "without.json")
            bar_path.write_text(json.dumps(bar_config))
            with (artifacts/("with.log" if enabled else "without.log")).open("w+") as bar_log:
                bar=subprocess.Popen(["waybar","--config",str(bar_path),"--style",str(bar_style)],env=bar_env,stdout=bar_log,stderr=bar_log,start_new_session=True)
                clients.append(bar)
                wait_for(lambda:bool(layers_for_bar()),"Waybar maps")
                time.sleep(.6)
                assert bar.poll() is None
                layer=layers_for_bar()[0];heights.append(layer["h"])
                shot=artifacts/("with.png" if enabled else "without.png")
                subprocess.run(["grim","-o",first["name"],str(shot)],env=bar_env,check=True,timeout=10)
                if enabled:
                    image=Image.open(shot).convert("RGB")
                    # The dot block precedes CPU. Detect dot-coloured rows and
                    # columns in its first 85px, excluding antialiasing fringes.
                    rows=[];points=[]
                    for y in range(layer["y"],layer["y"]+layer["h"]):
                        for x in range(0,80):
                            r,g,b=image.getpixel((x,y))
                            if max(r,g,b)>20 and ((abs(r-g)<8 and abs(g-b)<8) or (b>r*1.1 and b>g)):
                                points.append((x,y))
                    # Limit to the dot block using the CLI's fixed monospace
                    # span: CPU starts after it and has the larger 13px glyphs.
                    for y in sorted({y for x,y in points if x<55}):
                        if not rows or y>rows[-1][-1]+1:rows.append([y])
                        else:rows[-1].append(y)
                    assert len(rows)==3,(rows,layer)
                    assert all(len(row)>=2 for row in rows) and rows[0][0]>0 and rows[-1][-1]<layer["h"]-1,(rows,layer)
                    columns=[]
                    for x in sorted({x for x,y in points if y in rows[1]}):
                        if not columns or x>columns[-1][-1]+1:columns.append([x])
                        else:columns[-1].append(x)
                    assert len(columns)==9,(columns,rows,layer)
                    (artifacts/"dot-rows.json").write_text(json.dumps(rows))
                    image.crop((0,0,160,layer["h"])).save(artifacts/"minimap-crop.png")
                bar.terminate();bar.wait(timeout=10)
                wait_for(lambda:not layers_for_bar(),"Waybar closes")
                bar_log.flush();text=Path(bar_log.name).read_text()
                assert "Error parsing" not in text and "not a valid" not in text,text
        assert heights[0]==heights[1],f"minimap changed automatic bar height: {heights}"
        (artifacts/"heights.json").write_text(json.dumps({"without":heights[0],"with":heights[1],"font_px":13,"min_height":0}))
        print(f"PASS Waybar minimap: native move refresh, 9x3 dots, three rendered rows, unchanged automatic bar height {heights[0]}px, before CPU")
        print("PASS M3: CLI state/actions, event-driven JSON, idle deduplication, occupied/title/close events, escaping, unload/reload, example Waybar/CSS")
        ctl("dispatch", "hl.dsp.exit()")
        child.wait(timeout=10)
        assert child.returncode == 0, child.returncode
        assert log.read_text().count("hypr2d M0: C++ -> Rust -> C++: 41 -> 42") == expected_loads
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
