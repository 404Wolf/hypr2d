"""M4 rendering probe. All IPC and clients belong to the child PID below."""
import json
import os
from pathlib import Path
import signal
import socket
import select
import subprocess
import sys
import time
from PIL import Image, ImageChops

config, plugin, cli = sys.argv[1:]
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
        # Full production UI: real virtual-keyboard/pointer protocol clients,
        # all scoped to the nested child Wayland socket in this VM.
        def ui_state():
            return json.loads(ctl("hypr2d", "overview", "state"))
        def keypress(name):
            subprocess.run(["wtype", "-k", name], env=env, check=True, timeout=10)
        def open_ui():
            lua('hl.plugin.hypr2d.overview()')
            wait(lambda: ui_state().get("pending") == 0, "UI captures settle")
            time.sleep(0.2)
        open_ui()
        fitted=ui_state()["ui"]
        assert fitted["fit"] and len(fitted["cells"])<130
        assert all(c["label"] != c["key"] for c in fitted["cells"] if not c["key"].startswith("special:"))
        screenshot("ui-fit-default")
        keypress("a")
        wait(lambda:len(ui_state()["ui"]["cells"])==130,"full grid toggle")
        screenshot("ui-full-default")
        ok("hypr2d","overview-config",json.dumps({"fit":False,"font":"DejaVu Sans","font_size":18,"rounding":2,"border":"#4A4A4A"}))
        wait(lambda:ui_state().get("pending")==0,"theme refresh")
        screenshot("ui-theme")
        ui = ui_state()["ui"]
        assert len(ui["cells"]) == 130, ui
        assert any(c["key"] == "special:probe-0" and c["label"] == "probe-0" for c in ui["cells"])
        assert ui_state()["captures"] >= 10, ui_state()
        screenshot("ui-open")
        ui_image=Image.open(artifacts / "ui-open-0.png").convert("RGB")
        for panel,m in enumerate(mons):
            keys=[str(10*x+m["id"]+1) for x in range(4)]+[f"special:probe-{panel}"]
            for j,k in enumerate(keys):
                c=next(c for c in ui["cells"] if c["key"]==k)
                x,y,w,h=c["rect"]
                actual=ui_image.getpixel((round(x+w/2),round(y+h/2)))
                expected=tuple(bytes.fromhex(colors[panel*5+j]))
                assert max(abs(a-b) for a,b in zip(actual,expected)) <= 3, (k,actual,expected)
        # Keep newly created empty specials in the strip until overview close.
        lua('hl.dsp.workspace.toggle_special("empty")')
        lua('hl.dsp.workspace.toggle_special("empty")')
        wait(lambda:any(c["key"]=="special:empty" and not c["occupied"] for c in ui_state()["ui"]["cells"]),"empty special retained")
        head=mons[-1]["name"]
        ok("eval",f'hl.monitor({{output="{head}",mode="1280x720@60",position="0x-720",scale=1}})')
        wait(lambda:ui_state()["ui"]["panels"][1]["rect"][1] < ui_state()["ui"]["panels"][0]["rect"][1],"physical vertical arrangement")
        ok("eval",f'hl.monitor({{output="{head}",mode="1280x720@60",position="1280x0",scale=1}})')
        wait(lambda:ui_state().get("pending")==0,"layout capture settle")
        captures = ui_state()["captures"]
        time.sleep(0.25)
        assert ui_state()["captures"] == captures, "idle UI must not recapture"
        hidden=next(w for w in mapped() if w["class"]=="hypr2d-probe-0-3")
        children=Path(f'/proc/{hidden["pid"]}/task/{hidden["pid"]}/children').read_text().split()
        shell_pid=children[0]
        old_capture_count=ui_state()["captures"]
        fd=os.open(f"/proc/{shell_pid}/fd/1",os.O_WRONLY)
        os.write(fd,b"\x1b]11;#207040\x07\x1b[2J")
        os.close(fd)
        wait(lambda:ui_state()["captures"]>old_capture_count,"hidden surface commit refreshes thumbnail")
        time.sleep(0.2)
        assert ui_state()["captures"] <= old_capture_count+3,"only the changed cell should refresh"
        screenshot("ui-live")
        live=Image.open(artifacts / "ui-live-0.png").convert("RGB")
        changed=next(c for c in ui_state()["ui"]["cells"] if c["key"]==str(31+mons[0]["id"]))
        x,y,w,h=changed["rect"]
        assert live.getpixel((round(x+w/2),round(y+h/2)))==(32,112,64)
        assert next(w["title"] for w in mapped() if w["address"]==hidden["address"])==hidden["title"]
        keypress("l")
        wait(lambda: any(c["selected"] and c["key"] in ("11","12") for c in ui_state()["ui"]["cells"]), "keyboard right")
        for _ in range(3):
            before=next(c["key"] for c in ui_state()["ui"]["cells"] if c["selected"])
            keypress("Tab")
            wait(lambda:next(c["key"] for c in ui_state()["ui"]["cells"] if c["selected"]) != before,"Tab changes group")
            if any(c["selected"] and c["key"].startswith("special:") for c in ui_state()["ui"]["cells"]):
                break
        assert any(c["selected"] and c["key"].startswith("special:") for c in ui_state()["ui"]["cells"]),"special keyboard strip"
        ok("eval",'return hl.plugin.hypr2d._test_swipe("begin",3,0,true)')
        ok("eval",'return hl.plugin.hypr2d._test_swipe("update",20,0,true)')
        keypress("Escape")
        wait(lambda: ui_state().get("open") is False, "Esc closes")
        ok("eval",'return hl.plugin.hypr2d._test_swipe("update",20,0,true)')
        ok("eval",'return hl.plugin.hypr2d._test_swipe("end",true,0,true)')
        open_ui()
        target = next(c for c in ui_state()["ui"]["cells"] if c["key"] == str(mons[-1]["id"]+81))
        x,y,w,h = target["rect"]
        focused = next(m for m in monitors() if m["focused"])
        lua(f'hl.dsp.cursor.move({{x={focused["x"]+x+w/2},y={focused["y"]+y+h/2}}})')
        subprocess.run(["wlrctl", "pointer", "click", "left"], env=env, check=True, timeout=10)
        wait(lambda: ui_state().get("open") is False, "click activates cell")
        assert any(m["activeWorkspace"]["id"] == mons[-1]["id"]+81 for m in monitors())
        open_ui()
        ok("hypr2d", "overview", "close")
        wait(lambda: ui_state().get("open") is False, "close animation completes")
        print("PASS M4 UI: physical panels, special labels, idle cache, keyboard groups, Esc and pointer activation")
        # M5 uses a persistent, real virtual-pointer client so button-down spans
        # multiple motion frames. No direct calls to the Rust drag API are used.
        ptr = subprocess.Popen(["hypr2d-test-pointer"],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,start_new_session=True)
        clients.append(ptr)
        def pointer_ack(expected):
            assert select.select([ptr.stdout],[],[],10)[0],"virtual pointer acknowledgement"
            assert ptr.stdout.readline().strip()==expected
        pointer_ack(b"ready")
        def pointer_command(command):
            ptr.stdin.write((command+"\n").encode());ptr.stdin.flush();pointer_ack(b"ok")
        def real_window(address):
            return next(w for w in mapped() if w["address"]==address)
        def drawn_window(address):
            return next(w for w in ui_state()["ui"]["windows"] if w["id"]==address)
        def cell_point(key,fx=0.5,fy=0.5):
            c=next(c for c in ui_state()["ui"]["cells"] if c["key"]==key)
            x,y,w,h=c["rect"];return (x+w*fx,y+h*fy)
        def begin_drag(address):
            r=drawn_window(address)["rect"]
            x,y=r[0]+r[2]/2,r[1]+r[3]/2
            lua(f'hl.dsp.cursor.move({{x={mons[0]["x"]+x},y={mons[0]["y"]+y}}})')
            pointer_command("down")
            real=real_window(address)
            mon=next(m for m in mons if m["id"]==real["monitor"])
            workspace=real["workspace"]["name"] if real["workspace"]["name"].startswith("special:") else str(real["workspace"]["id"])
            cell=next(c for c in ui_state()["ui"]["cells"] if c["key"]==workspace)
            px,py=ui_state()["ui"]["pointer"]
            cx,cy,cw,ch=cell["rect"]
            return ((px-cx)/cw*mon["width"]-(real["at"][0]-mon["x"]),(py-cy)/ch*mon["height"]-(real["at"][1]-mon["y"]))
        def move_drag(point,display=0):
            current=json.loads(ctl("-j","cursorpos"))
            x,y=point
            pointer_command(f'move {mons[display]["x"]+x-current["x"]} {mons[display]["y"]+y-current["y"]}')
            wait(lambda:ui_state()["ui"]["dragging"],"drag threshold")
        def finish_drag():
            pointer_command("up")
            wait(lambda:not ui_state()["ui"]["dragging"] and ui_state()["pending"]==0,"drop settles")
        a=next(w["address"] for w in mapped() if w["class"]=="hypr2d-probe-0-0")
        b=next(w["address"] for w in mapped() if w["class"]=="hypr2d-probe-0-1")
        c=next(w["address"] for w in mapped() if w["class"]=="hypr2d-probe-1-1")
        open_ui()
        begin_drag(a);move_drag(cell_point(str(81+mons[0]["id"])))
        screenshot("ui-drag-ghost")
        ghost_image=Image.open(artifacts / "ui-drag-ghost-0.png").convert("RGB")
        gx,gy=cell_point(str(81+mons[0]["id"]))
        gp=ghost_image.getpixel((round(gx),round(gy)))
        assert gp[0]>gp[1]+20 and gp[0]>40,("ghost must render source pixels",gp)
        assert ui_state()["ui"]["target"]==str(81+mons[0]["id"])
        finish_drag()
        wait(lambda:real_window(a)["workspace"]["id"]==81+mons[0]["id"],"same-monitor drop")
        begin_drag(a);move_drag(cell_point(str(81+mons[1]["id"])),display=1);finish_drag()
        wait(lambda:real_window(a)["workspace"]["id"]==81+mons[1]["id"] and real_window(a)["monitor"]==mons[1]["id"],"cross-monitor drop")
        begin_drag(a);move_drag(cell_point("special:probe-0",0.02,0.02));finish_drag()
        wait(lambda:real_window(a)["workspace"]["name"]=="special:probe-0","drop into special")
        begin_drag(a);move_drag(cell_point(str(171+mons[0]["id"])));finish_drag()
        wait(lambda:real_window(a)["workspace"]["id"]==171+mons[0]["id"],"drop out of special")
        begin_drag(a)
        r=drawn_window(b)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2))
        assert any(w["id"]==b and w["target"] for w in ui_state()["ui"]["windows"])
        screenshot("ui-swap-target")
        finish_drag()
        wait(lambda:real_window(a)["workspace"]["id"]==11+mons[0]["id"] and real_window(b)["workspace"]["id"]==171+mons[0]["id"],"window swap")
        begin_drag(a);r=drawn_window(c)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));finish_drag()
        wait(lambda:real_window(a)["monitor"]==mons[1]["id"] and real_window(c)["monitor"]==mons[0]["id"],"cross-monitor swap")
        # Swap a fullscreen target through native state transitions, retaining its mode.
        lua(f'hl.dsp.window.fullscreen({{window="address:{b}",mode="fullscreen",action="set"}})')
        wait(lambda:real_window(b)["fullscreen"]>0 and ui_state()["pending"]==0,"fullscreen target")
        begin_drag(a);r=drawn_window(b)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));finish_drag()
        wait(lambda:real_window(a)["workspace"]["id"]==171+mons[0]["id"],"fullscreen swap")
        assert real_window(b)["fullscreen"]>0
        # Floating placement retains the press point within the window.
        lua(f'hl.dsp.window.float({{window="address:{c}",action="set"}})')
        wait(lambda:real_window(c)["floating"] and ui_state()["pending"]==0,"floating source")
        floating=real_window(c);fw,fh=floating["size"]
        def floating_drop(fx,fy):
            grab_x,grab_y=begin_drag(c)
            target=next(cell for cell in ui_state()["ui"]["cells"] if cell["key"]==str(91+mons[1]["id"]))
            cx,cy,cw,ch=target["rect"]
            move_drag(cell_point(str(91+mons[1]["id"]),fx,fy))
            px,py=ui_state()["ui"]["pointer"]
            expected_x=mons[1]["x"]+(px-cx)/cw*mons[1]["width"]-grab_x
            expected_y=mons[1]["y"]+(py-cy)/ch*mons[1]["height"]-grab_y
            finish_drag()
            wait(lambda:real_window(c)["workspace"]["id"]==91+mons[1]["id"],"floating cross-monitor drop")
            actual=real_window(c)
            assert abs(actual["at"][0]-expected_x)<=2 and abs(actual["at"][1]-expected_y)<=2,(actual["at"],expected_x,expected_y,grab_x,grab_y)
            assert actual["floating"] and actual["monitor"]==mons[1]["id"]
        floating_drop(0.7,0.6)
        # The source can now be partly outside the viewport: retain the actual
        # grab point within its clipped thumbnail on the next edge drop.
        floating_drop(0.02,0.02)
        before_cancel=real_window(c)["workspace"]
        begin_drag(c);move_drag(cell_point(str(251+mons[0]["id"])))
        screenshot("ui-cancel-before")
        keypress("Escape")
        assert ui_state()["ui"]["open"] and not ui_state()["ui"]["dragging"]
        pointer_command("up")
        assert real_window(c)["workspace"]==before_cancel
        screenshot("ui-cancel-after")
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"second Esc closes")
        # Closing the source during a held drag cannot leave a stale window pointer.
        open_ui();begin_drag(a);move_drag(cell_point(str(251+mons[1]["id"])))
        lua(f'hl.dsp.window.close({{window="address:{a}"}})')
        wait(lambda:all(w["address"]!=a for w in mapped()) and not ui_state()["ui"]["dragging"],"closed drag source")
        pointer_command("up")
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"close after cancelled source")
        screenshot("ui-clean-before")
        ui_runs=[]
        for cycle in range(20):
            open_ui();keypress("Escape");wait(lambda:ui_state().get("open") is False,"UI repeat close")
            assert ui_state()["deleted_gl_objects"],"production capture/label GL objects released"
            probe("close")  # Test-only GPU retirement barrier before closed RSS sampling.
            ui_runs.append(process_memory())
        screenshot("ui-clean-after");compare("ui-clean-after","ui-clean-before")
        rss=[int(r["rss"].split()[1]) for r in ui_runs[5:]]
        assert max(rss)-min(rss)<16*1024,rss
        assert len({r["fds"] for r in ui_runs[5:]})==1
        ptr.stdin.write(b"quit\n");ptr.stdin.flush();ptr.wait(timeout=10)
        print("PASS M5: real pointer drags, cross-monitor moves/swaps, specials, fullscreen swap, floating placement, ghost/target, two-stage Esc, disappearing source, 20 clean UI cycles")
        # M6 protocol, native placement, event routing and rendered annotations.
        with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as ext, socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as foreign:
            ext.connect(str(instance / "hypr2d.sock"));foreign.connect(str(instance / "hypr2d.sock"))
            ext.settimeout(10);foreign.settimeout(10)
            reader=ext.makefile("r"); other=foreign.makefile("r"); callbacks=[];seq=0
            def rpc(op,peer=ext,stream=reader,**fields):
                request={"v":1,"id":time.monotonic_ns(),"op":op,**fields}
                peer.sendall((json.dumps(request)+"\n").encode())
                while True:
                    reply=json.loads(stream.readline())
                    if reply.get("id")==request["id"]:return reply
                    callbacks.append(reply)
            assert not rpc("state",v=99)["ok"]
            assert not rpc("annotate",key="bad",target={"kind":"cell","monitor":mons[0]["id"],"x":99,"y":0},annotation={"label":"bad"})["ok"]
            assert not rpc("annotate",key="bad",target={"kind":"cell","monitor":mons[0]["id"],"x":0,"y":0},annotation={"colour":"bad"})["ok"]
            assert rpc("reserve",key="tools",monitor=mons[0]["id"],row=7,rows=1,title="Tools row",skip=True)["ok"]
            assert not rpc("reserve",peer=foreign,stream=other,key="conflict",monitor=mons[0]["id"],row=7,rows=1,title="Conflict")["ok"]
            for peer,stream in [(ext,reader),(foreign,other)]:assert rpc("subscribe",peer=peer,stream=stream,events=["activate","drop"])["ok"]
            target={"kind":"cell","monitor":mons[0]["id"],"x":1,"y":0}
            assert rpc("annotate",key="cell",target=target,annotation={"label":"Build <safe>","badge":"2","dot":True,"colour":"#FFCB6B","tooltip":"Jobs & checks"})["ok"]
            assert rpc("annotate",key="special",target={"kind":"special","name":"special:probe-0"},annotation={"label":"Chat","badge":"1","dot":True})["ok"]
            editor=next(w["address"] for w in mapped() if w["class"]=="hypr2d-probe-0-2")
            assert rpc("annotate",key="window",target={"kind":"window","address":editor},annotation={"label":"Editor","badge":"W","colour":"#82AAFF"})["ok"]
            assert len(rpc("state")["data"]["annotations"])==3
            module=subprocess.Popen([cli,"--instance",instance.name,"waybar","--monitor",mons[0]["name"]],stdout=subprocess.PIPE,stderr=log,start_new_session=True)
            clients.append(module)
            assert select.select([module.stdout],[],[],10)[0]
            record=json.loads(module.stdout.readline())
            assert "annotated" in record["class"] and "Build &lt;safe&gt;" in record["tooltip"] and "Jobs &amp; checks" in record["tooltip"] and "Editor" in record["tooltip"] and "Chat" in record["tooltip"],record
            open_ui()
            wait(lambda:any("Build <safe>" in cell["label"] for cell in ui_state()["ui"]["cells"]),"extension cell caption")
            assert any(l["text"]=="2 ●" for l in ui_state()["ui"]["labels"]),"badge/dot has independent space"
            screenshot("ui-extensions")
            keypress("Escape");wait(lambda:ui_state().get("open") is False,"close extensions UI")
            assert not rpc("place",window="0x1234",target=target)["ok"]
            assert not rpc("place",window=b,target={"kind":"special","name":"special:absent"})["ok"]
            placement={"kind":"cell","monitor":mons[0]["id"],"x":0,"y":7}
            assert rpc("place",window=b,target=placement)["ok"]
            wait(lambda:real_window(b)["workspace"]["id"]==561+mons[0]["id"],"extension placement")
            assert any(r.get("event")=="drop" and r["data"]["regions"]==["tools"] for r in callbacks),callbacks
            ok("hypr2d","goto","0","7",str(mons[0]["id"]))
            assert rpc("state")["ok"]
            assert any(r.get("event")=="activate" for r in callbacks),callbacks
            assert not select.select([foreign],[],[],0.1)[0],"callbacks are restricted to owned regions"
            ok("hypr2d","goto","0","6",str(mons[0]["id"]))
            ok("hypr2d","move","down","monitor",str(mons[0]["id"]))
            assert next(m for m in monitors() if m["id"]==mons[0]["id"])["activeWorkspace"]["id"]==1+mons[0]["id"],"reserved row skipped"
            assert rpc("clear",key="special")["ok"]
            reader.close();other.close()
        wait(lambda:not json.loads(ctl("hypr2d","state","-j"))["extensions"]["annotations"],"disconnect annotations cleanup")
        assert not json.loads(ctl("hypr2d","state","-j"))["extensions"]["regions"]
        module.terminate();module.wait(timeout=10)
        print("PASS M6: versioned socket, annotations in overview/waybar, reservations, native placement, owned callbacks, disconnect cleanup")
        if os.environ.get("HYPR2D_TEST_MODULE"):
            ok("eval",f'assert(loadfile({json.dumps(os.environ["HYPR2D_TEST_MODULE"])}))()')
            configured=json.loads(ctl("hypr2d","state","-j"))
            assert configured["overview_config"]["font_size"]==18 and configured["overview_config"]["rounding"]==2
            assert configured["swipe_config"]["fingers"]==4 and not configured["swipe_config"]["natural"]
            assert configured["swipe_enabled"]
            configured_grid=next(m["grid"] for m in configured["monitors"] if m["name"]=="WAYLAND-1")
            assert configured_grid["rows"]==9 and not configured_grid["wrap"]
            ok("hypr2d","swipe-enabled","false")
            ok("eval",'return hl.plugin.hypr2d._test_swipe("begin",4,0,false)')
            ok("hypr2d","swipe-enabled","true")
            ok("eval",'return hl.plugin.hypr2d._test_swipe("begin",4,0,true)')
            ok("eval",'return hl.plugin.hypr2d._test_swipe("end",true,0,true)')
            wait(lambda:all(abs(v)<0.01 for m in json.loads(ctl("hypr2d","state","-j"))["monitors"] for v in m["offset"]),"configured gesture settles")
            # wtype supplies a synthetic keymap, not physical evdev positions.
            ok("eval",'hl.config({input={resolve_binds_by_sym=true}})')
            (artifacts / "module-binds.json").write_text(ctl("-j","binds"))
            subprocess.run(["wtype","-M","logo","-P","Super_L","-s","50","-k","Tab","-s","50","-p","Super_L","-m","logo"],env=env,check=True,timeout=10)
            try:
                wait(lambda:ui_state().get("ui",{}).get("open") and ui_state().get("pending")==0,"generated SUPER+TAB bind")
            except AssertionError:
                print("M7 bind diagnostic",ctl("-j","binds"),ui_state(),flush=True)
                print((artifacts / "compositor.log").read_text()[-12000:],flush=True)
                raise
            time.sleep(0.2);screenshot("ui-home-manager")
            keypress("Escape");wait(lambda:ui_state().get("open") is False,"HM overview closes")
            print("PASS M7 module: generated Lua config, theme, grid, gesture enable/settings and overview bind")
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
        results = {"instance":instance.name,"small":small,"runs":runs,"screenshot_changed_pixels":diffs,"ui_cycles":ui_runs,"final_memory":process_memory()}
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
