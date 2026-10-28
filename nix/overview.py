"""M4 rendering probe. All IPC and clients belong to the child PID below."""
import json
import math
import os
from pathlib import Path
import signal
import socket
import select
import subprocess
import statistics
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
                if time.monotonic() >= deadline:
                    print("TIMEOUT",label,ctl("hypr2d","overview","state"),ctl("-j","monitors"),ctl("-j","binds"),ctl("-j","activewindow"),flush=True)
                    print("\n".join(line for line in (artifacts/"compositor.log").read_text().splitlines() if "accepted fd" not in line)[-20000:],flush=True)
                    raise AssertionError(label)
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
            result=subprocess.run(["hyprctl", "-i", instance.name, *args], capture_output=True, text=True, timeout=60)
            assert result.returncode==0,f"child IPC {args}: {result.stdout} {result.stderr}"
            return result.stdout.strip()
        def ok(*args):
            result = ctl(*args)
            assert result == "ok", result
        def lua(expression):
            ok("dispatch", expression)
        def monitors():
            slots={m["id"]:m["slot"] for m in json.loads(ctl("hypr2d","state","-j"))["monitors"]}
            return [dict(m,slot=slots.get(m["id"],0)) for m in json.loads(ctl("-j", "monitors"))]
        def probe(action):
            return json.loads(ctl("hypr2d", "probe", action))
        # Aquamarine occasionally retires its bootstrap fallback before making
        # the first Wayland output after the preceding child exits. Explicitly
        # create a headless replacement if bootstrap left none.
        ready_deadline=time.monotonic()+2
        while time.monotonic()<ready_deadline and not any(m["name"]!="FALLBACK" for m in monitors()):
            time.sleep(.05)
        if not any(m["name"]!="FALLBACK" for m in monitors()):
            ok("output","create","headless","WAYLAND-1")
        try:
            wait(lambda: len(monitors()) == 1 and monitors()[0]["name"]!="FALLBACK", "initial monitor")
        except AssertionError:
            print("Initial outputs:",monitors(),flush=True)
            print((artifacts / "compositor.log").read_text()[-16000:],flush=True)
            raise
        ok("hypr2d","overview-config",'{"supersample":1}')
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
        def first_minimap(process):
            # Startup deliberately emits a neutral record before subscribing.
            deadline=time.monotonic()+10;buffer=b""
            while time.monotonic()<deadline:
                while b"\n" in buffer:
                    line,buffer=buffer.split(b"\n",1)
                    record=json.loads(line)
                    if record["alt"]!="unavailable":return record
                assert select.select([process.stdout],[],[],max(0,deadline-time.monotonic()))[0],"ready minimap line"
                data=os.read(process.stdout.fileno(),65536)
                assert data,"minimap process exited"
                buffer+=data
            raise AssertionError("minimap remained unavailable")
        fixture_count = 0
        for panel, m in enumerate(mons):
            for x in range(4):
                ok("hypr2d", "goto", str(x), "0", str(m["id"]))
                app = f"hypr2d-probe-{panel}-{x}"
                clients.append(subprocess.Popen(["foot", "--config=/dev/null", f"--app-id={app}", "--override=cursor.blink=no", f"--override=colors-dark.background={colors[panel*5+x]}", "sh", "-c", f"printf '\\033[?25lmonitor {panel} workspace {x}'; sleep 600"], env=env, stdout=log, stderr=log, start_new_session=True))
                fixture_count += 1
                wait(lambda: len(mapped()) == fixture_count, "fixture map")
                target = 10*x + m["slot"] + 1
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
            # IPC and explicit-sync descriptors can retire just after their
            # completion barrier. Count retained FDs over a bounded quiet window;
            # permanent leaks remain in EVERY sample and still fail equality.
            fd_samples=[]
            for _ in range(5):
                fd_samples.append(len(list(Path(f"/proc/{child.pid}/fd").iterdir())))
                time.sleep(.02)
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
            return {"rss":rss, "drm_fdinfo": fdinfo, "fds":min(fd_samples),"fd_samples":fd_samples}
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
        assert len({r["closed_memory"]["fds"] for r in warm}) == 1, [r["closed_memory"] for r in warm]
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
            try:
                wait(lambda: ui_state().get("pending") == 0, "UI captures settle")
            except AssertionError:
                print("UI diagnostic",ui_state(),ctl("-j","monitors"),flush=True)
                print((artifacts/"compositor.log").read_text()[-20000:],flush=True)
                raise
            time.sleep(0.2)
        multi_before=json.loads(ctl("hypr2d","overview","invariants"))
        open_ui()
        keypress("Right");keypress("Tab")
        ok("eval",'hl.plugin.hypr2d._test_pinch("begin",4); hl.plugin.hypr2d._test_pinch("update",4,0.8); hl.plugin.hypr2d._test_pinch("end",false)')
        wait(lambda:ui_state().get("open") is False,"two-monitor pinch cancel")
        assert json.loads(ctl("hypr2d","overview","invariants"))==multi_before,"pinch preserves every monitor workspace and focus"
        open_ui()
        fitted=ui_state()["ui"]
        assert fitted["fit"] and len(fitted["cells"])<130
        assert all(c["label"] != c["key"] for c in fitted["cells"] if not c["key"].startswith("special:"))
        screenshot("ui-fit-default")
        def check_composition(ui):
            top=min(l["rect"][1] for l in ui["labels"])
            bottom=max(c["rect"][1]+c["rect"][3] for c in ui["cells"])
            assert abs(top-(720-bottom))<2,(top,bottom)
            assert all(not c["label"] for c in ui["cells"] if not c["occupied"] and not c["key"].startswith("special:"))
        check_composition(fitted)
        empty=next(c for c in fitted["cells"] if not c["occupied"] and not c["selected"] and not c["shown"] and not c["key"].startswith("special:"))
        x,y,w,h=empty["rect"]
        empty_image=Image.open(artifacts / "ui-fit-default-0.png").convert("RGB")
        assert all(abs(c-26)<=2 for c in empty_image.getpixel((round(x+w/2),round(y+h/2)))),"empty background must show translucent tile fill"
        empty_edges=[c["rect"] for c in fitted["cells"] if not c["occupied"] and not c["shown"] and not c["selected"] and not c["key"].startswith("special:")]
        assert any(empty_image.getpixel((round(cx)+offset,round(cy+ch/2)))==(53,53,53) for cx,cy,cw,ch in empty_edges for offset in (-1,0,1)),"unshadowed empty outlines must be base02"
        # Real mapped fixtures exercise short reverse-DNS class labels and +N.
        label_windows=[]
        for i in range(3):
            before={w["address"] for w in mapped()}
            process=subprocess.Popen(["foot","--config=/dev/null","--app-id=org.mozilla.firefox","--override=cursor.blink=no","--override=colors-dark.background=701010","sh","-c","printf '\\033[?25lhypr2d display-sized thumbnail\\nFine text and grouped window fixture\\n'; sleep 600"],env=env,stdout=log,stderr=log,start_new_session=True)
            clients.append(process)
            wait(lambda:any(w["class"]=="org.mozilla.firefox" and w["address"] not in before for w in mapped()),"label fixture mapped")
            address=next(w["address"] for w in mapped() if w["class"]=="org.mozilla.firefox" and w["address"] not in before)
            label_windows.append(address)
            lua(f'hl.dsp.window.move({{window="address:{address}",workspace={mons[0]["slot"]+1},follow=false}})')
        wait(lambda:any(c["label"]=="firefox +3" for c in ui_state()["ui"]["cells"]) and ui_state()["pending"]==0,"short main app plus extra window count")
        time.sleep(0.2);screenshot("ui-short-labels")
        check_composition(ui_state()["ui"])
        for address in label_windows:lua(f'hl.dsp.window.close({{window="address:{address}"}})')
        wait(lambda:all(w["address"] not in label_windows for w in mapped()) and ui_state()["pending"]==0,"label fixtures removed")
        keypress("a")
        wait(lambda:len(ui_state()["ui"]["cells"])==130,"full grid toggle")
        check_composition(ui_state()["ui"])
        screenshot("ui-full-default")
        ok("hypr2d","overview-config",json.dumps({"fit":False,"font":"DejaVu Sans","font_size":18,"rounding":2,"supersample":1,"border":"#4A4A4A"}))
        wait(lambda:ui_state().get("pending")==0,"theme refresh")
        screenshot("ui-theme")
        ui = ui_state()["ui"]
        assert len(ui["cells"]) == 130, ui
        assert any(c["key"] == "special:probe-0" and c["label"] == "probe-0" for c in ui["cells"])
        assert ui_state()["captures"] >= 10, ui_state()
        screenshot("ui-open")
        ui_image=Image.open(artifacts / "ui-open-0.png").convert("RGB")
        for panel,m in enumerate(mons):
            keys=[str(10*x+m["slot"]+1) for x in range(4)]+[f"special:probe-{panel}"]
            for j,k in enumerate(keys):
                c=next(c for c in ui["cells"] if c["key"]==k)
                x,y,w,h=c["rect"]
                actual=ui_image.getpixel((round(x+w/2),round(y+h*.75)))
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
        changed=next(c for c in ui_state()["ui"]["cells"] if c["key"]==str(31+mons[0]["slot"]))
        x,y,w,h=changed["rect"]
        assert live.getpixel((round(x+w/2),round(y+h/2)))==(32,112,64)
        assert next(w["title"] for w in mapped() if w["address"]==hidden["address"])==hidden["title"]
        keypress("l")
        wait(lambda: any(c["selected"] and c["key"] in [str(11+m["slot"]) for m in mons] for c in ui_state()["ui"]["cells"]), "keyboard right")
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
        target = next(c for c in ui_state()["ui"]["cells"] if c["key"] == str(mons[-1]["slot"]+81))
        x,y,w,h = target["rect"]
        focused = next(m for m in monitors() if m["focused"])
        lua(f'hl.dsp.cursor.move({{x={focused["x"]+x+w/2},y={focused["y"]+y+h/2}}})')
        subprocess.run(["wlrctl", "pointer", "click", "left"], env=env, check=True, timeout=10)
        wait(lambda: ui_state().get("open") is False, "click activates cell")
        assert any(m["activeWorkspace"]["id"] == mons[-1]["slot"]+81 for m in monitors())
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
        def hold_alt():
            modifier=subprocess.Popen(["wtype","-M","alt","-s","4000","-m","alt"],env=env,stdout=log,stderr=log,start_new_session=True)
            clients.append(modifier);time.sleep(.15)
            pointer_command("move 1 0")
            wait(lambda:ui_state()["ui"]["drop_mode"]=="swap","Alt selects swap")
            return modifier
        def finish_drag(alt=False):
            living={w["address"] for w in mapped()}
            processes=[p for p in clients if isinstance(p.args,list) and p.args[0]=="foot" and p.poll() is None]
            modifier=hold_alt() if alt else None
            pointer_command("up")
            wait(lambda:not ui_state()["ui"]["dragging"] and ui_state()["pending"]==0,"drop settles")
            if modifier:modifier.wait(timeout=10)
            diagnosis=json.loads(ctl("hypr2d","doctor"))
            assert diagnosis["ok"],diagnosis
            assert {w["address"] for w in mapped()}==living,"drop killed or unmapped a client"
            assert all(p.poll() is None for p in processes),"drop terminated a client process"
        a=next(w["address"] for w in mapped() if w["class"]=="hypr2d-probe-0-0")
        b=next(w["address"] for w in mapped() if w["class"]=="hypr2d-probe-0-1")
        c=next(w["address"] for w in mapped() if w["class"]=="hypr2d-probe-1-1")
        exec(Path(os.environ["HYPR2D_LATENCY_HELPER"]).read_text())
        drag_latency = MinimapLatency(instance, cli, log)
        open_ui()
        begin_drag(a);move_drag(cell_point(str(81+mons[0]["slot"])))
        screenshot("ui-drag-ghost")
        ghost_image=Image.open(artifacts / "ui-drag-ghost-0.png").convert("RGB")
        gx,gy=cell_point(str(81+mons[0]["slot"]))
        gp=ghost_image.getpixel((round(gx),round(gy)))
        assert gp[0]>gp[1]+20 and gp[0]>40,("ghost must render source pixels",gp)
        assert ui_state()["ui"]["target"]==str(81+mons[0]["slot"])
        drag_latency.expect("overview-drop",lambda:pointer_command("up"),
            lambda r:f'workspace {81+mons[0]["slot"]}</b>\n' in r["tooltip"] and "hypr2d-probe-0-0" in r["tooltip"],event="movewindowv2")
        (artifacts/"minimap-drag-latency.json").write_text(json.dumps(drag_latency.results,indent=2))
        drag_latency.close()
        wait(lambda:ui_state()["pending"]==0,"latency drop settles")
        wait(lambda:real_window(a)["workspace"]["id"]==81+mons[0]["slot"],"same-monitor drop")
        begin_drag(a);move_drag(cell_point(str(81+mons[1]["slot"])),display=1);finish_drag()
        wait(lambda:real_window(a)["workspace"]["id"]==81+mons[1]["slot"] and real_window(a)["monitor"]==mons[1]["id"],"cross-monitor drop")
        begin_drag(a);move_drag(cell_point("special:probe-0",0.02,0.02));finish_drag()
        wait(lambda:real_window(a)["workspace"]["name"]=="special:probe-0","drop into special")
        begin_drag(a);move_drag(cell_point(str(171+mons[0]["slot"])));finish_drag()
        wait(lambda:real_window(a)["workspace"]["id"]==171+mons[0]["slot"],"drop out of special")
        begin_drag(a)
        r=drawn_window(b)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2))
        assert any(w["id"]==b and w["target"] for w in ui_state()["ui"]["windows"])
        modifier=hold_alt();screenshot("ui-swap-target")
        finish_drag();modifier.wait(timeout=10)
        wait(lambda:real_window(a)["workspace"]["id"]==11+mons[0]["slot"] and real_window(b)["workspace"]["id"]==171+mons[0]["slot"],"window swap")
        begin_drag(a);r=drawn_window(c)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));finish_drag(alt=True)
        wait(lambda:real_window(a)["monitor"]==mons[1]["id"] and real_window(c)["monitor"]==mons[0]["id"],"cross-monitor swap")
        # Swap a fullscreen target through native state transitions, retaining its mode.
        lua(f'hl.dsp.window.fullscreen({{window="address:{b}",mode="fullscreen",action="set"}})')
        wait(lambda:real_window(b)["fullscreen"]>0 and ui_state()["pending"]==0,"fullscreen target")
        begin_drag(a);r=drawn_window(b)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));finish_drag(alt=True)
        wait(lambda:real_window(a)["workspace"]["id"]==171+mons[0]["slot"],"fullscreen swap")
        assert real_window(b)["fullscreen"]>0
        # Floating placement retains the press point within the window.
        lua(f'hl.dsp.window.float({{window="address:{c}",action="set"}})')
        wait(lambda:real_window(c)["floating"] and ui_state()["pending"]==0,"floating source")
        floating=real_window(c);fw,fh=floating["size"]
        def floating_drop(fx,fy):
            grab_x,grab_y=begin_drag(c)
            target=next(cell for cell in ui_state()["ui"]["cells"] if cell["key"]==str(91+mons[1]["slot"]))
            cx,cy,cw,ch=target["rect"]
            move_drag(cell_point(str(91+mons[1]["slot"]),fx,fy))
            px,py=ui_state()["ui"]["pointer"]
            expected_x=mons[1]["x"]+(px-cx)/cw*mons[1]["width"]-grab_x
            expected_y=mons[1]["y"]+(py-cy)/ch*mons[1]["height"]-grab_y
            finish_drag()
            wait(lambda:real_window(c)["workspace"]["id"]==91+mons[1]["slot"],"floating cross-monitor drop")
            actual=real_window(c)
            assert abs(actual["at"][0]-expected_x)<=2 and abs(actual["at"][1]-expected_y)<=2,(actual["at"],expected_x,expected_y,grab_x,grab_y)
            assert actual["floating"] and actual["monitor"]==mons[1]["id"]
        floating_drop(0.7,0.6)
        # The source can now be partly outside the viewport: retain the actual
        # grab point within its clipped thumbnail on the next edge drop.
        floating_drop(0.02,0.02)
        before_cancel=real_window(c)["workspace"]
        begin_drag(c);move_drag(cell_point(str(251+mons[0]["slot"])))
        screenshot("ui-cancel-before")
        keypress("Escape")
        assert ui_state()["ui"]["open"] and not ui_state()["ui"]["dragging"]
        pointer_command("up")
        assert real_window(c)["workspace"]==before_cancel
        screenshot("ui-cancel-after")
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"second Esc closes")
        # Closing the source during a held drag cannot leave a stale window pointer.
        open_ui();begin_drag(a);move_drag(cell_point(str(251+mons[1]["slot"])))
        lua(f'hl.dsp.window.close({{window="address:{a}"}})')
        wait(lambda:all(w["address"]!=a for w in mapped()) and not ui_state()["ui"]["dragging"],"closed drag source")
        pointer_command("up")
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"close after cancelled source")
        def stack_fixture(app,x,y):
            ok("hypr2d","goto",str(x),str(y),str(mons[0]["id"]))
            client=subprocess.Popen(["foot","--config=/dev/null",f"--app-id={app}","--override=cursor.blink=no","sh","-c","sleep 600"],env=env,stdout=log,stderr=log,start_new_session=True)
            clients.append(client)
            wait(lambda:any(w["class"]==app for w in mapped()),"stack fixture")
            return next(w["address"] for w in mapped() if w["class"]==app)
        # Also exercise the native detach path that focuses the remaining group.
        ok("eval",'hl.config({group={focus_removed_window=false}})')
        # Reproduce live normal/special swaps before the group feature tests.
        sx=stack_fixture("swap-normal",2,4)
        sy=stack_fixture("swap-special",3,4)
        sentinel=stack_fixture("special-sentinel",4,4)
        lua(f'hl.dsp.window.move({{window="address:{sy}",workspace="special:swap-accounting",follow=false}})')
        lua(f'hl.dsp.window.move({{window="address:{sentinel}",workspace="special:sentinel",follow=false}})')
        open_ui()
        for grouped in [False,True]:
            if grouped:
                lua(f'hl.dsp.group.toggle({{window="address:{sx}"}})')
                wait(lambda:bool(real_window(sx)["grouped"]),"grouped swap source")
                partner=stack_fixture("swap-partner",5,4)
                ok("eval",f'local a=assert(hl.get_window("address:{sx}")); local b=assert(hl.get_window("address:{partner}")); a.group:add(b)')
                wait(lambda:len(real_window(sx)["grouped"])==2,"two-member swap source")
                # Select the visible member for the drag, as on the user's laptop.
                sx,partner=partner,sx
            before_addresses={w["address"] for w in mapped()}
            begin_drag(sx);r=drawn_window(sy)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));finish_drag(alt=True)
            diagnosis=json.loads(ctl("hypr2d","doctor"))
            (artifacts/f"drop-doctor-grouped-{grouped}.json").write_text(json.dumps(diagnosis,indent=2))
            assert diagnosis["ok"],diagnosis
            assert {w["address"] for w in mapped()}==before_addresses,"drop killed or unmapped a client"
            assert real_window(sentinel)["workspace"]["name"]=="special:sentinel"
            if not grouped:assert not real_window(sx)["grouped"] and not real_window(sy)["grouped"],"swap created singleton groups"
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"accounting test close")
        accounting_addresses=[sx,sy,sentinel,partner]
        for address in accounting_addresses:lua(f'hl.dsp.window.close({{window="address:{address}"}})')
        wait(lambda:all(w["address"] not in accounting_addresses for w in mapped()),"accounting fixture cleanup")
        sa=stack_fixture("stack-a",3,3)
        sb=stack_fixture("stack-b",4,3)
        sc=stack_fixture("stack-c",5,3)
        open_ui()
        begin_drag(sa);r=drawn_window(sb)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2))
        assert ui_state()["ui"]["drop_mode"]=="stack" and not ui_state()["ui"]["target"]
        screenshot("ui-stack-target")
        finish_drag()
        wait(lambda:set(real_window(sa)["grouped"])=={sa,sb},"cross-cell stack")
        assert not next(cell for cell in ui_state()["ui"]["cells"] if cell["key"]==str(271+mons[0]["slot"]))["occupied"],"empty source grid cell updates immediately"
        groups=json.loads(ctl("hypr2d","overview","invariants"))["groups"]
        assert next(g for g in groups if sa in g["members"])["current"]==sa
        begin_drag(sc);r=drawn_window(sa)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));finish_drag()
        wait(lambda:set(real_window(sc)["grouped"])=={sa,sb,sc},"stack onto existing group")
        assert next(g for g in json.loads(ctl("hypr2d","overview","invariants"))["groups"] if sc in g["members"])["current"]==sc
        # Alt exchanges only the visible member of a three-window source stack.
        se=stack_fixture("alt-three-target",7,3)
        wait(lambda:ui_state()["pending"]==0,"Alt group fixture capture")
        original_group_ws=real_window(sc)["workspace"]["id"]
        target_ws=real_window(se)["workspace"]["id"]
        begin_drag(sc);r=drawn_window(se)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));finish_drag(alt=True)
        wait(lambda:real_window(sc)["workspace"]["id"]==target_ws and real_window(se)["workspace"]["id"]==original_group_ws,"Alt swaps visible member only")
        assert not real_window(sc)["grouped"] and set(real_window(sa)["grouped"])=={sa,sb}
        assert real_window(sa)["workspace"]["id"]==original_group_ws and real_window(sb)["workspace"]["id"]==original_group_ws
        remaining_current=next(g for g in json.loads(ctl("hypr2d","overview","invariants"))["groups"] if sa in g["members"])["current"]
        begin_drag(sc);r=drawn_window(remaining_current)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));finish_drag()
        wait(lambda:set(real_window(sc)["grouped"])=={sa,sb,sc},"restore three-member source for Shift test")
        lua(f'hl.dsp.window.close({{window="address:{se}"}})')
        wait(lambda:all(w["address"]!=se for w in mapped()),"Alt fixture cleanup")
        # Refuse moving a locked three-member stack, including Shift whole-stack mode.
        lua(f'hl.dsp.focus({{window="address:{sc}"}})')
        lua('hl.dsp.group.lock_active({action="set"})')
        locked_three=json.loads(ctl("hypr2d","overview","invariants"))
        begin_drag(sc);move_drag(cell_point(str(331+mons[0]["slot"])))
        modifier=subprocess.Popen(["wtype","-M","shift","-s","1200","-m","shift"],env=env,stdout=log,stderr=log,start_new_session=True)
        clients.append(modifier);time.sleep(.15);pointer_command("up")
        wait(lambda:ui_state()["ui"]["last_refused"]==sc,"locked whole-stack refusal")
        modifier.wait(timeout=10)
        assert json.loads(ctl("hypr2d","overview","invariants"))==locked_three
        lua('hl.dsp.group.lock_active({action="unset"})')
        # A held Shift moves all three members; release changes the ghost live.
        begin_drag(sc);move_drag(cell_point(str(331+mons[0]["slot"])))
        assert ui_state()["ui"]["ghost_count"]==1
        modifier=subprocess.Popen(["wtype","-M","shift","-s","1000","-m","shift"],env=env,stdout=log,stderr=log,start_new_session=True)
        clients.append(modifier)
        wait(lambda:ui_state()["ui"]["ghost_count"]==3,"Shift stacked-cards ghost")
        modifier.wait(timeout=10)
        wait(lambda:ui_state()["ui"]["ghost_count"]==1,"release Shift single ghost")
        modifier=subprocess.Popen(["wtype","-M","shift","-s","2000","-m","shift"],env=env,stdout=log,stderr=log,start_new_session=True)
        clients.append(modifier)
        wait(lambda:ui_state()["ui"]["ghost_count"]==3,"whole-stack drop modifier")
        screenshot("ui-whole-stack-ghost")
        finish_drag();modifier.wait(timeout=10)
        wait(lambda:all(real_window(w)["workspace"]["id"]==331+mons[0]["slot"] for w in [sa,sb,sc]),"Shift moves every group member")
        assert set(real_window(sc)["grouped"])=={sa,sb,sc}
        whole_group=next(g for g in json.loads(ctl("hypr2d","overview","invariants"))["groups"] if sc in g["members"])
        assert whole_group["current"]==sc,"Shift preserves stack current member"
        next_member=whole_group["members"][(whole_group["members"].index(sc)+1)%3]
        # Moving a member out leaves the rest of its former group behind.
        begin_drag(sc);move_drag(cell_point(str(291+mons[0]["slot"])));finish_drag()
        wait(lambda:not real_window(sc)["grouped"] and set(real_window(sa)["grouped"])=={sa,sb},"detach only dragged member")
        remaining=next(g for g in json.loads(ctl("hypr2d","overview","invariants"))["groups"] if sa in g["members"])
        assert remaining["current"]==next_member,"plain drop advances the remaining stack"
        assert real_window(sa)["workspace"]["id"]==331+mons[0]["slot"] and real_window(sb)["workspace"]["id"]==331+mons[0]["slot"]
        # A dedicated special loses its last tile as soon as the drop completes.
        lua('hl.dsp.workspace.toggle_special("drop-last")')
        sd=stack_fixture("stack-special",6,3)
        lua(f'hl.dsp.window.move({{window="address:{sd}",workspace="special:drop-last",follow=false}})')
        wait(lambda:any(cell["key"]=="special:drop-last" for cell in ui_state()["ui"]["cells"]) and ui_state()["pending"]==0,"last-window special")
        begin_drag(sc);r=drawn_window(sd)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));finish_drag()
        wait(lambda:set(real_window(sc)["grouped"])=={sc,sd} and real_window(sc)["workspace"]["name"]=="special:drop-last","stack into special")
        begin_drag(sc);move_drag(cell_point(str(291+mons[0]["slot"])));finish_drag()
        wait(lambda:not real_window(sc)["grouped"] and real_window(sc)["workspace"].get("id")==291+mons[0]["slot"],"detach group member out of special")
        begin_drag(sd);move_drag(cell_point(str(311+mons[0]["slot"])));finish_drag()
        wait(lambda:all(cell["key"]!="special:drop-last" for cell in ui_state()["ui"]["cells"]),"empty special tile removed immediately")
        screenshot("ui-stack-special-removed")
        # Native active-group locks are checked before any detach/move.
        lua(f'hl.dsp.focus({{window="address:{sa}"}})')
        lua('hl.dsp.group.lock_active({action="set"})')
        locked=json.loads(ctl("hypr2d","overview","invariants"))
        begin_drag(sc);r=drawn_window(sa)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2))
        pointer_command("up")
        wait(lambda:ui_state()["ui"]["last_refused"]==sa,"locked-group refusal")
        assert json.loads(ctl("hypr2d","overview","invariants"))==locked,"refusal preserves focus, group order/members/locks"
        assert not real_window(sc)["grouped"] and real_window(sc)["workspace"].get("id")==291+mons[0]["slot"]
        screenshot("ui-stack-locked-refusal")
        # A locked source group cannot be detached even onto empty space.
        begin_drag(sa);move_drag(cell_point(str(321+mons[0]["slot"])))
        pointer_command("up")
        assert json.loads(ctl("hypr2d","overview","invariants"))==locked,"locked source cannot leave its group"
        lua('hl.dsp.group.lock_active({action="unset"})')
        lua('hl.dsp.group.lock({action="set"})')
        global_locked=json.loads(ctl("hypr2d","overview","invariants"))
        begin_drag(sc);r=drawn_window(sa)["rect"];move_drag((r[0]+r[2]/2,r[1]+r[3]/2));pointer_command("up")
        assert json.loads(ctl("hypr2d","overview","invariants"))==global_locked,"global group lock cannot be bypassed"
        lua('hl.dsp.group.lock({action="unset"})')
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"stack test close")
        for address in [sa,sb,sc,sd]:lua(f'hl.dsp.window.close({{window="address:{address}"}})')
        wait(lambda:all(w["address"] not in [sa,sb,sc,sd] for w in mapped()),"stack fixture cleanup")
        print("PASS stack: cross-cell/new/existing group, active member, detach source member, Alt swaps, empty-special removal, locked refusal",flush=True)
        # Region selection uses the same real pointer/keyboard event hooks as single drags.
        ra=stack_fixture("region-a",0,5)
        rb=stack_fixture("region-b",1,5)
        rc=stack_fixture("region-occupied",2,5)
        open_ui()
        def pointer_to(point):
            cursor=json.loads(ctl("-j","cursorpos"))
            pointer_command(f'move {mons[0]["x"]+point[0]-cursor["x"]} {mons[0]["y"]+point[1]-cursor["y"]}')
        def select_block(first,last):
            modifier=subprocess.Popen(["wtype","-M","ctrl","-s","1000","-m","ctrl"],env=env,stdout=log,stderr=log,start_new_session=True)
            clients.append(modifier);time.sleep(.15)
            pointer_to(cell_point(str(first)));pointer_command("down")
            pointer_to(cell_point(str(last)));pointer_command("up")
            wait(lambda:ui_state()["ui"]["region_mode"]==2,"marquee selected")
            modifier.wait(timeout=10)
        def move_block(first,target,swap=False):
            pointer_to(cell_point(str(first)));pointer_command("down")
            pointer_to(cell_point(str(target)))
            wait(lambda:ui_state()["ui"]["region_mode"]==3,"region picked up")
            modifier=None
            if swap:
                modifier=subprocess.Popen(["wtype","-M","alt","-s","1500","-m","alt"],env=env,stdout=log,stderr=log,start_new_session=True)
                clients.append(modifier);time.sleep(.15)
            screenshot("ui-region-swap" if swap else "ui-region-ghost")
            pointer_command("up")
            wait(lambda:ui_state()["ui"]["region_mode"]==0 and ui_state()["pending"]==0,"region dispatcher settles")
            if modifier:modifier.wait(timeout=10)
            assert json.loads(ctl("hypr2d","doctor"))["ok"]
        def undo_block():
            subprocess.run(["wtype","-M","ctrl","-k","z","-m","ctrl"],env=env,check=True)
            wait(lambda:ui_state()["pending"]==0,"region undo settles")
        slot=mons[0]["slot"]
        first,second,dest=401+slot,411+slot,421+slot
        select_block(first,second);screenshot("ui-region-selection")
        move_block(first,dest)
        wait(lambda:real_window(ra)["workspace"]["id"]==dest and real_window(rb)["workspace"]["id"]==dest+10,"region preserves cell offsets")
        assert real_window(rc)["workspace"]["id"]==dest,"occupied destinations merge"
        undo_block()
        wait(lambda:real_window(ra)["workspace"]["id"]==first and real_window(rb)["workspace"]["id"]==second,"region undo restores block")
        assert real_window(rc)["workspace"]["id"]==dest
        select_block(first,second);move_block(first,dest,swap=True)
        wait(lambda:real_window(ra)["workspace"]["id"]==dest and real_window(rc)["workspace"]["id"]==first,"Alt swaps whole blocks")
        undo_block()
        wait(lambda:real_window(ra)["workspace"]["id"]==first and real_window(rc)["workspace"]["id"]==dest,"block swap undo")
        select_block(first,second);move_block(first,401+mons[1]["slot"])
        wait(lambda:real_window(ra)["monitor"]==mons[1]["id"] and real_window(rb)["workspace"]["id"]==411+mons[1]["slot"],"cross-monitor region")
        undo_block()
        wait(lambda:real_window(ra)["workspace"]["id"]==first and real_window(rb)["workspace"]["id"]==second,"cross-monitor undo")
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"region close")
        ok("hypr2d","goto","2","5",str(mons[0]["id"]));open_ui()
        for key in ["v","h","m","k","Return"]:keypress(key)
        wait(lambda:real_window(rb)["workspace"]["id"]==331+slot and real_window(rc)["workspace"]["id"]==341+slot,"keyboard region extension and move")
        undo_block();wait(lambda:real_window(rb)["workspace"]["id"]==second and real_window(rc)["workspace"]["id"]==dest,"keyboard region undo")
        keypress("v");keypress("h");keypress("m")
        for _ in range(6):keypress("l")
        keypress("Return")
        wait(lambda:real_window(rb)["workspace"]["id"]==471+slot and real_window(rc)["workspace"]["id"]==401+slot,"wrapped block keeps relative offsets")
        undo_block();wait(lambda:real_window(rb)["workspace"]["id"]==second and real_window(rc)["workspace"]["id"]==dest,"wrapped block undo")
        keypress("Escape");wait(lambda:not ui_state().get("open",False),"keyboard region close")
        ok("hypr2d","configure",mons[0]["name"],"8","8","false","10");open_ui()
        keypress("v");keypress("h");keypress("m")
        for _ in range(6):keypress("l")
        keypress("Return")
        assert ui_state()["ui"]["region_mode"]==2 and ui_state()["ui"]["last_refused"]=="region"
        assert real_window(rb)["workspace"]["id"]==second and real_window(rc)["workspace"]["id"]==dest
        screenshot("ui-region-bounds-refused")
        keypress("Escape");keypress("Escape");wait(lambda:not ui_state().get("open",False),"bounds refusal close")
        ok("hypr2d","configure",mons[0]["name"],"8","8","true","10")
        for address in [ra,rb,rc]:lua(f'hl.dsp.window.close({{window="address:{address}"}})')
        wait(lambda:all(w["address"] not in [ra,rb,rc] for w in mapped()),"region fixture cleanup")
        print("PASS regions: marquee, block ghost, occupied merge, Alt swap, cross-monitor move and atomic undo",flush=True)
        # Live grid swapping keeps native workspace IDs and group membership intact.
        old_grid={m["id"]:m["slot"] for m in monitors()}
        numbered_before={w["address"]:(w["workspace"]["id"],w["monitor"]) for w in mapped() if w["workspace"].get("id",0)>0}
        active_before={m["id"]:m["activeWorkspace"]["id"] for m in monitors()}
        ok("hypr2d","swap-grids",mons[0]["name"],mons[1]["name"])
        wait(lambda:next(m for m in monitors() if m["id"]==mons[0]["id"])["slot"]==old_grid[mons[1]["id"]],"CLI swaps grid assignments")
        wait(lambda:all(real_window(address)["workspace"]["id"]==workspace and real_window(address)["monitor"]==next(m["id"] for m in mons if m["id"]!=old_monitor) for address,(workspace,old_monitor) in numbered_before.items()),"native workspaces follow swapped grids")
        assert next(m for m in monitors() if m["id"]==mons[0]["id"])["activeWorkspace"]["id"]==active_before[mons[1]["id"]]
        assert json.loads(ctl("hypr2d","doctor"))["ok"]
        open_ui();screenshot("ui-grids-swapped")
        headers=ui_state()["ui"]["panels"]
        def header_point(i):
            x,y,w,h=headers[i]["rect"];return (x+min(w,60)/2,y+h/2)
        pointer_to(header_point(0));pointer_command("down");pointer_to(header_point(1))
        screenshot("ui-header-grid-swap")
        pointer_command("up")
        wait(lambda:all(next(m for m in monitors() if m["id"]==mid)["slot"]==slot for mid,slot in old_grid.items()),"header drop swaps grids back")
        wait(lambda:all((real_window(address)["workspace"]["id"],real_window(address)["monitor"])==position for address,position in numbered_before.items()),"header swap restores every workspace host")
        assert json.loads(ctl("hypr2d","doctor"))["ok"]
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"grid swap close")
        print("PASS grid swapping: two live outputs, native workspace IDs/hosts/current cells and header drag",flush=True)
        screenshot("ui-clean-before")
        # Screencopy forces damage_tracking=0. Restore normal tracking so a
        # closed RSS sample isn't taken in a continuously rendering pipeline.
        ok("eval",'hl.config({debug={damage_tracking=2}})')
        ui_runs=[]
        for cycle in range(60):
            open_ui();keypress("Escape");wait(lambda:ui_state().get("open") is False,"UI repeat close")
            assert ui_state()["deleted_gl_objects"],"production capture/label GL objects released"
            time.sleep(.2) # Finish the native frames requested by overview close.
            samples=[]
            for sample in range(3):
                probe("retire") # GPU barrier + allocator trim, without fresh damage.
                samples.append(process_memory())
                if sample<2:time.sleep(.05)
            retained=min(samples,key=lambda s:int(s["rss"].split()[1])).copy()
            retained["rss_samples"]=[s["rss"] for s in samples]
            ui_runs.append(retained)
        screenshot("ui-clean-after");compare("ui-clean-after","ui-clean-before")
        warm=ui_runs[20:]
        rss=[int(r["rss"].split()[1]) for r in warm]
        floors=[min(rss[i:i+10]) for i in range(0,len(rss),10)]
        slope=statistics.linear_regression(range(len(rss)),rss).slope
        (artifacts/"ui-memory.json").write_text(json.dumps({"samples":ui_runs,"warm_rss_kib":rss,"block_floors_kib":floors,"growth_kib_per_cycle":slope},indent=2))
        assert max(floors)-min(floors)<16*1024,(floors,rss)
        assert floors[-1]<=floors[0]+4*1024,("retained closed footprint grows",floors,rss)
        raw_rss=[int(sample.split()[1]) for run in warm for sample in run["rss_samples"]]
        assert max(raw_rss)-min(raw_rss)<64*1024,raw_rss
        assert len({r["fds"] for r in warm})==1,ui_runs
        ptr.stdin.write(b"quit\n");ptr.stdin.flush();ptr.wait(timeout=10)
        print("PASS M5: real pointer drags, cross-monitor moves/swaps, specials, fullscreen swap, floating placement, ghost/target, two-stage Esc, disappearing source, 60 clean UI cycles")
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
            record=first_minimap(module)
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
            wait(lambda:real_window(b)["workspace"]["id"]==561+mons[0]["slot"],"extension placement")
            assert any(r.get("event")=="drop" and r["data"]["regions"]==["tools"] for r in callbacks),callbacks
            ok("hypr2d","goto","0","7",str(mons[0]["id"]))
            assert rpc("state")["ok"]
            assert any(r.get("event")=="activate" for r in callbacks),callbacks
            assert not select.select([foreign],[],[],0.1)[0],"callbacks are restricted to owned regions"
            ok("hypr2d","goto","0","6",str(mons[0]["id"]))
            ok("hypr2d","move","down","monitor",str(mons[0]["id"]))
            assert next(m for m in monitors() if m["id"]==mons[0]["id"])["activeWorkspace"]["id"]==1+mons[0]["slot"],"reserved row skipped"
            assert rpc("clear",key="special")["ok"]
            reader.close();other.close()
        wait(lambda:not json.loads(ctl("hypr2d","state","-j"))["extensions"]["annotations"],"disconnect annotations cleanup")
        assert not json.loads(ctl("hypr2d","state","-j"))["extensions"]["regions"]
        module.terminate();module.wait(timeout=10)
        print("PASS M6: versioned socket, annotations in overview/waybar, reservations, native placement, owned callbacks, disconnect cleanup")
        if os.environ.get("HYPR2D_TEST_MODULE"):
            ok("eval",'hl.bind("SUPER + TAB",hl.dsp.group.next())')
            ok("eval",f'assert(loadfile({json.dumps(os.environ["HYPR2D_TEST_MODULE"])}))()')
            configured=json.loads(ctl("hypr2d","state","-j"))
            assert configured["overview_config"]["font_size"]==18 and configured["overview_config"]["rounding"]==2
            assert configured["swipe_config"]["fingers"]==4 and not configured["swipe_config"]["natural"]
            assert configured["swipe_config"]["diagonal_width"]==14 and configured["swipe_config"]["hysteresis"]==7
            assert configured["swipe_config"]["hysteresis_distance"]==16 and configured["swipe_config"]["projection_decel"]==0.0008
            assert configured["swipe_config"]["spring_stiffness"]==625
            assert configured["swipe_config"]["sensitivity"]==1.3
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
        # Real-hardware regressions: one named 3:2 output at a non-zero origin,
        # forced software cursor, with the active window in a two-member group.
        ok("eval",'hl.config({animations={enabled=false},debug={vfr=false}})')
        for w in mapped():
            lua(f'hl.dsp.window.close({{window="address:{w["address"]}"}})')
        wait(lambda:not mapped(),"old fixtures close")
        # Stable identity and parked grids: all outputs and clients are VM-owned.
        live, dock=monitors()
        def hot_fixture(app,monitor,x,y):
            ok("hypr2d","goto",str(x),str(y),str(monitor["id"]))
            process=subprocess.Popen(["foot","--config=/dev/null",f"--app-id={app}","sh","-c","sleep 600"],env=env,stdout=log,stderr=log,start_new_session=True)
            clients.append(process)
            wait(lambda:any(w["class"]==app for w in mapped()),"hotplug fixture")
            return next(w["address"] for w in mapped() if w["class"]==app)
        keeper=hot_fixture("live-keeper",live,0,0)
        special_keeper=hot_fixture("special-keeper",live,0,0)
        lua(f'hl.dsp.window.move({{window="address:{special_keeper}",workspace="special:parking-test",follow=false}})')
        parked_a=hot_fixture("parked-a",dock,1,1)
        parked_hidden=hot_fixture("parked-group-member",dock,1,1)
        lua(f'hl.dsp.group.toggle({{window="address:{parked_a}"}})')
        ok("eval",f'local a=assert(hl.get_window("address:{parked_a}")); a.group:add(assert(hl.get_window("address:{parked_hidden}")))')
        lua(f'hl.dsp.group.active({{window="address:{parked_a}",index=1}})')
        wait(lambda:len(real_window(parked_a)["grouped"])==2,"grouped parked fixture")
        parked_b=hot_fixture("parked-b",dock,3,2)
        original={a:real_window(a)["workspace"]["id"] for a in [parked_a,parked_b,parked_hidden]}
        ok("hypr2d","goto","1","1",str(dock["id"]))
        ok("output","remove",dock["name"])
        wait(lambda:len(monitors())==1 and len(json.loads(ctl("hypr2d","state","-j"))["parked"])==1,"unplug parks")
        mons=monitors()
        assert all(real_window(a)["workspace"]["id"]==ws and real_window(a)["monitor"]==live["id"] for a,ws in original.items())
        state=json.loads(ctl("hypr2d","state","-j"))
        assert state["monitors"][0]["focused"] and state["monitors"][0]["workspace"]==1+live["slot"],state
        assert state["parked"][0]["windows"]==3 and state["parked"][0]["slot"]==dock["slot"],state
        group=next(g for g in json.loads(ctl("hypr2d","overview","invariants"))["groups"] if parked_a in g["members"])
        assert group["current"]==parked_a and set(group["members"])=={parked_a,parked_hidden},"unplug preserves group membership/current"
        minimap=subprocess.Popen([cli,"--instance",instance.name,"waybar"],env=env,stdout=subprocess.PIPE,stderr=log,start_new_session=True)
        clients.append(minimap)
        record=first_minimap(minimap)
        minimap.terminate();minimap.wait(timeout=10)
        assert "Parked" in record["tooltip"] and dock["name"] in record["tooltip"],record
        ok("hypr2d","move","right")
        assert monitors()[0]["activeWorkspace"]["id"]==11+live["slot"],"live moves never visit parked residue"
        open_ui()
        assert len(ui_state()["ui"]["cards"])==1
        assert all(c["monitor"]==live["id"] or c["monitor"]>=0x80000000 for c in ui_state()["ui"]["cells"])
        screenshot("ui-parked-card")
        ptr=subprocess.Popen(["hypr2d-test-pointer"],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,start_new_session=True)
        clients.append(ptr);pointer_ack(b"ready")
        card=ui_state()["ui"]["cards"][0]["rect"]
        lua(f'hl.dsp.cursor.move({{x={live["x"]+card[0]+card[2]/2},y={live["y"]+card[1]+card[3]/2}}})')
        pointer_command("down");pointer_command("up")
        wait(lambda:ui_state()["ui"]["expanded_parked"] is not None and ui_state()["pending"]==0,"click expands parked")
        assert any("disconnected" in l["text"] for l in ui_state()["ui"]["labels"])
        screenshot("ui-parked-expanded")
        keypress("Escape");assert ui_state()["ui"]["expanded_parked"] is None
        # Tab order reaches the card after the live grid and special strip.
        def selected_cell():return next(c for c in ui_state()["ui"]["cells"] if c["selected"])
        assert selected_cell()["monitor"]==live["id"],"live grid remains keyboard origin after card click/Esc"
        keypress("Tab");assert selected_cell()["key"].startswith("special:"),"Tab visits specials after live grids"
        keypress("Tab");assert selected_cell()["monitor"]>=0x80000000,"Tab visits parked cards after specials"
        keypress("Return")
        wait(lambda:ui_state()["ui"]["expanded_parked"] is not None,"keyboard expands parked")
        begin_drag(parked_a)
        tab=next(l["rect"] for l in ui_state()["ui"]["labels"] if l["text"]=="live")
        move_drag((tab[0]+tab[2]/2,tab[1]+tab[3]/2))
        wait(lambda:ui_state()["ui"]["expanded_parked"] is None,"drag over live tab")
        move_drag(cell_point(str(21+live["slot"])));finish_drag()
        assert real_window(parked_a)["workspace"]["id"]==21+live["slot"]
        assert monitors()[0]["activeWorkspace"]["id"]==11+live["slot"],"group detach must not leave the parked workspace active"
        # Restore it through the parked card, then exercise direct card-to-live.
        lua(f'hl.dsp.window.move({{window="address:{parked_a}",workspace={original[parked_a]},follow=false}})')
        wait(lambda:any(w["id"]==parked_a for w in ui_state()["ui"]["windows"]) and ui_state()["pending"]==0,"return fixture to parked card")
        ok("hypr2d","overview","parked")
        wait(lambda:ui_state()["ui"]["expanded_parked"] is not None and ui_state()["pending"]==0,"expanded parked for special drop")
        begin_drag(parked_a);move_drag(cell_point("special:parking-test",.01,.01));finish_drag()
        assert real_window(parked_a)["workspace"]["name"]=="special:parking-test"
        lua(f'hl.dsp.window.move({{window="address:{parked_a}",workspace={original[parked_a]},follow=false}})')
        wait(lambda:any(w["id"]==parked_a and w["workspace"]==str(original[parked_a]) for w in ui_state()["ui"]["windows"]) and ui_state()["pending"]==0,"return parked member from special")
        keypress("Escape");assert ui_state()["ui"]["expanded_parked"] is None
        begin_drag(parked_b);move_drag(cell_point(str(81+live["slot"])));finish_drag()
        assert real_window(parked_b)["workspace"]["id"]==81+live["slot"]
        lua(f'hl.dsp.window.move({{window="address:{parked_b}",workspace={original[parked_b]},follow=false}})')
        wait(lambda:ui_state()["pending"]==0,"parked restore settles")
        # The live grid can target an empty parked cell, and regions use the
        # same tab transition in both directions (no native monitor stealing).
        def drag_tab(label,region=False):
            tab=next(l["rect"] for l in ui_state()["ui"]["labels"] if l["text"]==label)
            point=(tab[0]+tab[2]/2,tab[1]+tab[3]/2)
            if region:pointer_to(point)
            else:move_drag(point)
        begin_drag(keeper);drag_tab("parked")
        wait(lambda:ui_state()["ui"]["expanded_parked"] is not None,"live drag expands parked target")
        move_drag(cell_point(str(1+dock["slot"]),.01,.01));screenshot("ui-live-to-parked");finish_drag()
        assert real_window(keeper)["workspace"]["id"]==1+dock["slot"]
        begin_drag(keeper);drag_tab("live");move_drag(cell_point(str(1+live["slot"]),.01,.01));finish_drag()
        assert real_window(keeper)["workspace"]["id"]==1+live["slot"]
        select_block(1+live["slot"],11+live["slot"])
        pointer_to(cell_point(str(1+live["slot"])));pointer_command("down");drag_tab("parked",True)
        wait(lambda:ui_state()["ui"]["expanded_parked"] is not None,"region enters parked view")
        pointer_to(cell_point(str(1+dock["slot"])));screenshot("ui-region-parked-target");pointer_command("up")
        wait(lambda:ui_state()["ui"]["region_mode"]==0 and ui_state()["pending"]==0,"region into parked")
        assert real_window(keeper)["workspace"]["id"]==1+dock["slot"]
        select_block(1+dock["slot"],11+dock["slot"])
        pointer_to(cell_point(str(1+dock["slot"])));pointer_command("down");drag_tab("live",True)
        pointer_to(cell_point(str(1+live["slot"])));pointer_command("up")
        wait(lambda:ui_state()["ui"]["region_mode"]==0 and ui_state()["pending"]==0,"region out of parked")
        assert real_window(keeper)["workspace"]["id"]==1+live["slot"]
        assert json.loads(ctl("hypr2d","doctor"))["ok"]
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"parked UI closes")
        # Adopt the disconnected grid; its own current cell follows the grid.
        remembered=91+dock["slot"]
        lua(f'hl.dsp.window.move({{window="address:{keeper}",workspace={1+dock["slot"]},follow=false}})')
        ok("hypr2d","adopt",str(dock["slot"]),live["name"])
        wait(lambda:monitors()[0]["slot"]==dock["slot"],"adopt parked grid")
        assert monitors()[0]["activeWorkspace"]["id"]==remembered
        assert real_window(keeper)["workspace"]["id"]==1+dock["slot"]
        open_ui();screenshot("ui-adopted-parked-grid")
        begin_drag(keeper);drag_tab("parked");move_drag(cell_point(str(1+live["slot"]),.01,.01));screenshot("ui-empty-parked-target");finish_drag()
        begin_drag(keeper);drag_tab("live");move_drag(cell_point(str(1+dock["slot"]),.01,.01));finish_drag()
        select_block(1+dock["slot"],11+dock["slot"])
        pointer_to(cell_point(str(1+dock["slot"])));pointer_command("down");drag_tab("parked",True)
        pointer_to(cell_point(str(1+live["slot"])));pointer_command("up")
        wait(lambda:ui_state()["ui"]["region_mode"]==0 and ui_state()["pending"]==0,"region into entirely empty parked grid")
        select_block(1+live["slot"],11+live["slot"])
        pointer_to(cell_point(str(1+live["slot"])));pointer_command("down");drag_tab("live",True)
        pointer_to(cell_point(str(1+dock["slot"])));pointer_command("up")
        wait(lambda:ui_state()["ui"]["region_mode"]==0 and ui_state()["pending"]==0,"region out of formerly empty parked grid")
        assert real_window(keeper)["workspace"]["id"]==1+dock["slot"]
        # Drag the now empty old grid card onto the live header to adopt it back.
        card=ui_state()["ui"]["cards"][0]["rect"]
        panel=ui_state()["ui"]["panels"][0]["rect"]
        card_header=next(l["rect"] for l in ui_state()["ui"]["labels"] if l["text"].startswith(live["name"]+" ·"))
        pointer_to((card_header[0]+min(card_header[2],60)/2,card_header[1]+card_header[3]/2));pointer_command("down")
        pointer_to((panel[0]+min(panel[2],60)/2,panel[1]+panel[3]/2));screenshot("ui-card-adoption")
        pointer_command("up")
        wait(lambda:monitors()[0]["slot"]==live["slot"],"card header adoption")
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"adoption close")
        ptr.stdin.write(b"quit\n");ptr.stdin.flush();ptr.wait(timeout=10)
        ok("output","create","headless",dock["name"])
        wait(lambda:len(monitors())==2 and any(m["name"]==dock["name"] for m in monitors()),"replug output")
        restored=next(m for m in monitors() if m["name"]==dock["name"])
        wait(lambda:all(real_window(a)["monitor"]==restored["id"] for a in original),"exact workspace reattach")
        assert restored["slot"]==dock["slot"] and all(real_window(a)["workspace"]["id"]==ws for a,ws in original.items())
        assert restored["activeWorkspace"]["id"]==91+dock["slot"],restored
        assert not json.loads(ctl("hypr2d","state","-j"))["parked"]
        # Recreate the first output second: slot numbering must not follow IDs/order.
        ok("output","remove",live["name"])
        wait(lambda:len(monitors())==1,"remove first output")
        ok("output","create","headless",live["name"])
        wait(lambda:len(monitors())==2,"reversed plug order")
        assert next(m for m in monitors() if m["name"]==live["name"])["slot"]==live["slot"]
        # Simulate a renamed dock port with the same physical serial.
        ok("hypr2d","monitor-test","identity",dock["name"],"Vendor","Model","serial-a")
        ok("output","remove",dock["name"])
        wait(lambda:len(monitors())==1,"dock detached before rename")
        ok("hypr2d","monitor-test","identity","DOCK-RENAMED","Vendor","Model","serial-a")
        ok("output","create","headless","DOCK-RENAMED")
        wait(lambda:any(m["name"]=="DOCK-RENAMED" for m in monitors()),"renamed dock output")
        renamed=next(m for m in monitors() if m["name"]=="DOCK-RENAMED")
        wait(lambda:all(real_window(a)["monitor"]==renamed["id"] for a in original),"serial fallback exact restore")
        assert renamed["slot"]==dock["slot"] and all(real_window(a)["workspace"]["id"]==ws for a,ws in original.items())
        assert json.loads(ctl("hypr2d","doctor"))["ok"]
        for a in [keeper,special_keeper,parked_a,parked_b,parked_hidden]:lua(f'hl.dsp.window.close({{window="address:{a}"}})')
        wait(lambda:not mapped(),"hotplug fixture cleanup")
        mons=monitors()
        print("PASS hotplug: parked focus/cells/minimap, cards/expand/drag, exact replug, reversed order, serial rename",flush=True)
        ok("output","create","headless","eDP-1")
        wait(lambda:any(m["name"]=="eDP-1" for m in monitors()),"named laptop output")
        ok("eval",'hl.monitor({output="eDP-1",mode="2256x1504@60",position="8934x6682",scale=1})')
        wait(lambda:any(m["name"]=="eDP-1" and m["enabled"] for m in json.loads(ctl("hypr2d","overview","invariants"))["outputs"]),"new laptop output enabled before removing old outputs")
        for m in mons:
            ok("output","remove",m["name"])
        ok("eval",'hl.monitor({output="eDP-1",mode="2256x1504@60",position="8934x6682",scale=1,disabled=false})')
        wait(lambda:len(monitors())==1 and monitors()[0]["name"]=="eDP-1" and json.loads(ctl("hypr2d","overview","invariants"))["outputs"][0]["enabled"],"single enabled eDP-1")
        wait(lambda:(monitors()[0]["width"],monitors()[0]["height"],monitors()[0]["x"],monitors()[0]["y"] )==(2256,1504,8934,6682),"laptop mode/origin applied")
        mons=monitors(); edp=mons[0]
        assert (edp["width"],edp["height"],edp["x"],edp["y"],edp["scale"])==(2256,1504,8934,6682,1),edp
        ok("eval",'hl.config({animations={enabled=false},group={groupbar={height=4,render_titles=false,gradients=false,scrolling=true}},input={follow_mouse=1,resolve_binds_by_sym=true},cursor={invisible=false,no_hardware_cursors=1,hide_on_key_press=false}})')
        ok("hypr2d","overview-config",'{"fit":true,"font_size":16,"rounding":3,"supersample":1}')
        ok("hypr2d","goto","0","0",str(edp["id"]))
        def laptop_fixture(app):
            pattern = "\x1b[?25l\x1b[97mPIXEL CAPTURE\n0123456789 abcdef\nSharp thumbnail detail\nGroups stay unchanged\n"
            clients.append(subprocess.Popen(["foot","--config=/dev/null",f"--app-id={app}","--override=font=monospace:size=32","--override=cursor.blink=no","--override=mouse.hide-when-typing=yes","sh","-c",'printf "%s" "$1"; sleep 600',"fixture",pattern],env=env,stdout=log,stderr=log,start_new_session=True))
            wait(lambda:any(w["class"]==app for w in mapped()),"laptop fixture")
            return next(w["address"] for w in mapped() if w["class"]==app)
        first=laptop_fixture("kitty")
        lua('hl.dsp.group.toggle()')
        second=laptop_fixture("firefox")
        wait(lambda:len(real_window(second)["grouped"])==2,"active two-window group")
        # Deliberately choose the other member: restoring to the group's head
        # or most recently mapped window would fail this regression.
        lua('hl.dsp.group.active({index=1})')
        current=json.loads(ctl("-j","activewindow"))["address"]
        assert current==first,(current,first)
        ok("hypr2d","goto","1","0",str(edp["id"]))
        hidden=laptop_fixture("hidden-cell")
        ok("hypr2d","goto","0","0",str(edp["id"]))
        lua('hl.dsp.workspace.toggle_special("8")')
        special=laptop_fixture("obsidian")
        lua('hl.dsp.workspace.toggle_special("8")')
        # A hidden group too, to cover EVERY group's current member.
        ok("hypr2d","goto","2","0",str(edp["id"]))
        hidden_first=laptop_fixture("hidden-group-first")
        lua('hl.dsp.group.toggle()')
        hidden_second=laptop_fixture("hidden-group-second")
        wait(lambda:len(real_window(hidden_second)["grouped"])==2,"hidden group")
        lua('hl.dsp.group.active({index=1})')
        ok("hypr2d","goto","0","0",str(edp["id"]))
        lua(f'hl.dsp.focus({{window="address:{first}"}})')
        def invariants():
            return {"native":json.loads(ctl("hypr2d","overview","invariants")),"monitors":snapshot(),
                    "windows":[{k:w[k] for k in ("address","hidden","grouped","workspace","monitor")} for w in mapped()]}
        # Keep cursor on an empty margin, not over the underlying group's tab bar.
        lua('hl.dsp.cursor.move({x=8944,y=6692})')
        time.sleep(.3)
        def super_tab():
            subprocess.run(["wtype","-M","logo","-P","Super_L","-s","50","-k","Tab","-s","50","-p","Super_L","-m","logo"],env=env,check=True,timeout=10)
        # Reproduce the user's duplicate chord: both native dispatchers run.
        ok("eval",'hl.unbind("SUPER + TAB"); hl.bind("SUPER + TAB",hl.dsp.group.next()); hl.bind("SUPER + TAB",hl.plugin.hypr2d.overview())')
        super_tab()
        wait(lambda:ui_state().get("pending")==0,"duplicate chord capture")
        assert json.loads(ctl("-j","activewindow"))["address"]==second,"duplicate chord must reproduce group-next bug"
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"duplicate chord close")
        lua('hl.dsp.group.active({index=1})')
        assert os.environ.get("HYPR2D_TEST_MODULE"),"hardware bind regression uses generated Home Manager config"
        ok("eval",f'assert(loadfile({json.dumps(os.environ["HYPR2D_TEST_MODULE"])}))()')
        # Keep the original native-resolution regression explicit after HM's
        # new default (2x). The wallpaper benchmark below covers supersampling.
        ok("hypr2d","overview-config",'{"fit":true,"font_size":16,"rounding":3,"supersample":1}')
        baseline=invariants()
        assert baseline["native"]["focus"]==first,baseline
        screenshot("hardware-before")
        super_tab()
        wait(lambda:ui_state().get("ui",{}).get("open") and ui_state().get("pending")==0,"grouped SUPER+TAB overview")
        time.sleep(.2)
        assert invariants()==baseline,"opening/capturing must not mutate focus/groups/order/workspaces"
        state=ui_state(); geometries=state["ui"]
        (artifacts/"hardware-fit.json").write_text(json.dumps(state,indent=2))
        rects=[c["rect"] for c in geometries["cells"]]+[l["rect"] for l in geometries["labels"]]
        left=min(r[0] for r in rects);right=max(r[0]+r[2] for r in rects)
        top=min(r[1] for r in rects);bottom=max(r[1]+r[3] for r in rects)
        assert abs(left-(2256-right))<.01 and abs(top-(1504-bottom))<.01,(left,right,top,bottom)
        assert all(abs(c["rect"][2]/c["rect"][3]-1.5)<.001 for c in geometries["cells"])

        for image in state["images"]:
            cell=next(c for c in geometries["cells"] if c["key"]==image["key"])
            assert image["direct"] and image["width"]==image["drawn_width"] and image["height"]==image["drawn_height"]
            assert image["width"]==math.ceil(cell["rect"][2]) and image["height"]==math.ceil(cell["rect"][3]),(image,cell)
        assert next(c for c in geometries["cells"] if c["key"]=="special:8")["label"]=="obsidian"
        for label,special_panel in [("eDP-1",False),("Special workspaces",True)]:
            header=next(h for h in geometries["labels"] if h["text"]==label)["rect"]
            cells=[c["rect"] for c in geometries["cells"] if c["key"].startswith("special:")==special_panel]
            assert abs(header[0]-min(c[0] for c in cells))<.01,(header,cells)
            assert 15<=header[3]<=25,header
            assert abs(min(c[1] for c in cells)-header[1]-header[3]-8)<.01,(header,cells)
            reported=next(h for h in geometries["panels"] if h["label"]==label)["rect"]
            assert reported==header,(reported,header)
        fitted_width=state["images"][0]["width"]
        assert fitted_width>160,state["images"]
        assert sum(i["width"]*i["height"]*4*4/3 for i in state["images"])<=64*1024*1024
        # Software cursors are baked into normal screencopy; no grim -c path
        # that could independently synthesize a cursor and mask a render bug.
        screenshot("hardware-fit-cursor")
        before=Image.open(artifacts/"hardware-fit-cursor-0.png").convert("RGB")
        assert before.size==(2256,1504),before.size
        assert any(max(before.getpixel((x,y)))>60 for x in range(10,40) for y in range(10,45)),"software cursor must appear over opaque overview"
        def images_fit(fit):
            s=ui_state()
            if s.get("pending")!=0 or s.get("ui",{}).get("fit")!=fit:return False
            cells={c["key"]:c for c in s["ui"]["cells"] if c["occupied"]}
            return {i["key"] for i in s["images"]}==cells.keys() and all((i["width"],i["height"])==(math.ceil(cells[i["key"]]["rect"][2]),math.ceil(cells[i["key"]]["rect"][3])) for i in s["images"])
        keypress("a");wait(lambda:images_fit(False),"hardware full recapture")
        full=ui_state()
        assert max(i["width"] for i in full["images"])<fitted_width,(full["images"],fitted_width)
        for image in full["images"]:
            cell=next(c for c in full["ui"]["cells"] if c["key"]==image["key"])
            assert (image["width"],image["height"])==(math.ceil(cell["rect"][2]),math.ceil(cell["rect"][3]))
        screenshot("hardware-full")
        keypress("a");wait(lambda:images_fit(True),"hardware fit recapture")
        assert ui_state()["images"][0]["width"]==fitted_width
        screenshot("hardware-fit")
        keypress("Escape");wait(lambda:ui_state().get("open") is False,"hardware close")
        assert invariants()==baseline,"closing must preserve group state"
        screenshot("hardware-after")
        assert ImageChops.difference(Image.open(artifacts/"hardware-before-0.png").convert("RGB"),Image.open(artifacts/"hardware-after-0.png").convert("RGB")).getbbox() is None,"group/cursor scene unchanged"
        open_ui();keypress("Right")
        ok("eval",'hl.plugin.hypr2d._test_pinch("begin",4); hl.plugin.hypr2d._test_pinch("update",4,1.2); hl.plugin.hypr2d._test_pinch("end",false)')
        wait(lambda:ui_state().get("open") is False,"pinch with non-head group member focused")
        assert invariants()==baseline,"pinch must preserve originally focused group member"
        # Already-visible special: opening/closing may not toggle it or focus it.
        lua('hl.dsp.workspace.toggle_special("8")')
        time.sleep(.2); shown=invariants()
        for _ in range(5):
            open_ui();assert invariants()==shown
            keypress("Escape");wait(lambda:ui_state().get("open") is False,"group repeat close")
            assert invariants()==shown
            assert ui_state()["deleted_gl_objects"]
        # Pinches run through InputManager, not just EventBus: a native Lua
        # opener is present, so an incorrectly forwarded end would reopen it.
        ok("eval", 'pinch_opens=0; hl.gesture({fingers=4,direction="pinchin",action=function() pinch_opens=pinch_opens+1; hl.plugin.hypr2d.overview()() end})')
        def pinch(phase, value=None):
            args=f'"{phase}"'
            if phase=="begin": args+=f",{value or 4}"
            elif phase=="update": args+=f",4,{value}"
            elif phase=="end": args+=",false"
            ok("eval",f'hl.plugin.hypr2d._test_pinch({args})')
        ok("hypr2d","pinch-config","0.08")
        ok("eval",'assert(hl.plugin.hypr2d.swipe_config({fingers=4}))')
        for scale in (0.85,1.15):
            pinch("begin");pinch("update",1.2);pinch("end")
            wait(lambda:ui_state().get("pending")==0,"native pinch opens overview")
            # All monitors, specials, focus, every group/current, window order.
            assert invariants()==shown
            keypress("Right");keypress("Tab") # uncommitted selection must not apply
            pinch("begin")
            pinch("update",1.03)
            assert ui_state().get("ui",{}).get("open"),"subthreshold pinch must not close"
            # A concurrent swipe sequence must never claim pinch translation.
            ok("eval",'hl.plugin.hypr2d._test_swipe("begin",4,nil,false); hl.plugin.hypr2d._test_swipe("update",900,0,false)')
            pinch("update",scale)
            wait(lambda:ui_state().get("open") is False,"pinch cancels overview")
            # Keep fingers down past GL cleanup, then update/end: still claimed.
            pinch("update",1.3);pinch("end")
            ok("eval",'hl.plugin.hypr2d._test_swipe("end",false,nil,false)')
            time.sleep(.2)
            assert ui_state().get("open") is False,"closing pinch must not reopen"
            assert invariants()==shown,"pinch restores focus/groups/order/workspaces/specials"
        ok("eval",'assert(pinch_opens==2,"cancellation leaked to native Lua gesture")')
        # Native opening callback at gesture START must likewise finish untouched.
        ok("eval",'hl.gesture({fingers=4,direction="pinchin",action="unset"}); hl.gesture({fingers=4,direction="pinchin",action={start=function() hl.plugin.hypr2d.overview()() end}})')
        pinch("begin");pinch("update",1.2);pinch("update",1.4);pinch("end")
        wait(lambda:ui_state().get("pending")==0,"start-callback opening pinch survives updates")
        pinch("begin");pinch("update",0.85);pinch("end")
        wait(lambda:ui_state().get("open") is False,"start-callback bind cannot reopen cancellation")
        assert invariants()==shown
        ok("eval",'hl.gesture({fingers=4,direction="pinchin",action="unset"})')
        print("PASS pinch: native Lua opening, in/out cancel, no reopen, swipe isolation, grouped focus/special/workspace/order preservation")
        lua('hl.dsp.workspace.toggle_special("8")')
        lua(f'hl.dsp.focus({{window="address:{first}"}})')
        # Retiring the drag pointer above can remove wl_seat's pointer
        # capability. Keep a real virtual pointer alive for Foot's cursor test;
        # a dispatcher warp alone cannot create a client wl_pointer.
        ptr=subprocess.Popen(["hypr2d-test-pointer"],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,start_new_session=True)
        clients.append(ptr);pointer_ack(b"ready")
        pointer_command("move 1 0") # establish the native seat mouse before dispatcher warps
        ok("hypr2d","goto","7","7",str(edp["id"]))
        cursor_fixture=laptop_fixture("cursor-fixture")
        # Force a real pointer leave/enter: an unchanged center warp does not
        # deliver motion to Foot after native pinch forwarding.
        lua('hl.dsp.cursor.move({x=8935,y=6683})')
        time.sleep(.1)
        win=real_window(cursor_fixture)
        lua(f'hl.dsp.cursor.move({{x={win["at"][0]+win["size"][0]/2},y={win["at"][1]+win["size"][1]/2}}})')
        pointer_command("move 1 0")
        # Cursor enter/shape is asynchronous in the client. Let Foot receive the
        # enter before typing, then wait for its actual hide request.
        time.sleep(.2)
        typing=subprocess.Popen(["wtype","-s","200","-k","x","-s","60000"],env=env,stdout=log,stderr=log,start_new_session=True)
        clients.append(typing)
        wait(lambda:not json.loads(ctl("hypr2d","overview","cursor"))["texture"],"client hides cursor after typing")
        hidden_cursor=json.loads(ctl("hypr2d","overview","cursor"))
        assert not hidden_cursor["texture"],("fixture must reproduce a missing app cursor image",hidden_cursor)
        screenshot("app-hidden-cursor-before")
        ok("hypr2d","overview","open")
        wait(lambda:ui_state().get("pending")==0,"app-hidden cursor capture")
        time.sleep(.2)
        visible_cursor=json.loads(ctl("hypr2d","overview","cursor"))
        assert visible_cursor["texture"] and visible_cursor["should_render"],visible_cursor
        (artifacts/"app-cursor-state.json").write_text(json.dumps({"before":hidden_cursor,"open":visible_cursor},indent=2))
        screenshot("app-hidden-cursor-overview")
        cursor=json.loads(ctl("-j","cursorpos"))
        point=(cursor["x"]-edp["x"],cursor["y"]-edp["y"])
        hidden_image=Image.open(artifacts/"app-hidden-cursor-overview-0.png").convert("RGB")
        cx,cy=map(round,point)
        assert any(max(hidden_image.getpixel((x,y)))>180 for x in range(cx,cx+20) for y in range(cy,cy+30)),"overview must own a visible arrow even when app cursor was hidden"
        ok("eval",'hl.config({cursor={invisible=true}})')
        wait(lambda:not json.loads(ctl("hypr2d","overview","cursor"))["should_render"],"native cursor hiding timer")
        ok("hypr2d","overview","refresh")
        wait(lambda:ui_state().get("pending")==0,"cursor-free overlay repaint")
        time.sleep(.2)
        screenshot("app-hidden-cursor-overlay-without-arrow")
        no_arrow=Image.open(artifacts/"app-hidden-cursor-overlay-without-arrow-0.png").convert("RGB")
        cursor_difference=ImageChops.difference(hidden_image,no_arrow)
        changed=sum(p!=(0,0,0) for p in cursor_difference.get_flattened_data())
        assert 20<changed<2500,("cursor must be composited above the overview content",changed)
        ok("eval",'hl.config({cursor={invisible=false}})')
        wait(lambda:json.loads(ctl("hypr2d","overview","cursor"))["should_render"],"native cursor showing timer")
        ok("hypr2d","overview","close")
        wait(lambda:ui_state().get("open") is False,"app-hidden cursor close")
        assert not json.loads(ctl("hypr2d","overview","cursor"))["texture"],"restore hidden app cursor on close"
        typing.terminate();typing.wait(timeout=5)
        lua(f'hl.dsp.window.close({{window="address:{cursor_fixture}"}})')
        wait(lambda:all(w["address"]!=cursor_fixture for w in mapped()),"cursor fixture closes")
        ok("hypr2d","goto","0","0",str(edp["id"]))
        lua(f'hl.dsp.focus({{window="address:{first}"}})')
        ptr.stdin.write(b"quit\n");ptr.stdin.flush();ptr.wait(timeout=10)
        (artifacts/"hardware-invariants.json").write_text(json.dumps({"before":baseline,"after":invariants(),"shown":shown},indent=2))
        # Isolate the actual single-output workload, not the parked grids left
        # by earlier hotplug coverage. These clients still close via dispatchers.
        keep={first,second,hidden,special,hidden_first,hidden_second}
        for stale in mapped():
            if stale["address"] not in keep:
                lua(f'hl.dsp.window.close({{window=assert(hl.get_window("address:{stale["address"]}"))}})')
        wait(lambda:{w["address"] for w in mapped()}==keep,"single-grid benchmark clients")
        # Match the live strip width: five specials alongside the full 9x9.
        for i in range(4):
            lua(f'hl.dsp.workspace.toggle_special("visual-{i}")')
            laptop_fixture(f"visual-special-{i}")
            lua(f'hl.dsp.workspace.toggle_special("visual-{i}")')
        lua(f'hl.dsp.focus({{window="address:{first}"}})')
        # Visual upgrade: real layer-shell wallpaper, 2256x1504, 9x9 grid.
        wallpaper=Image.new("RGB",(2256,1504))
        from PIL import ImageDraw
        painter=ImageDraw.Draw(wallpaper)
        painter.rectangle((0,0,1127,751),fill="#406080")
        painter.rectangle((1128,0,2255,751),fill="#805040")
        painter.rectangle((0,752,1127,1503),fill="#406050")
        painter.rectangle((1128,752,2255,1503),fill="#604080")
        wallpaper.save(artifacts/"wallpaper.png")
        paper=subprocess.Popen(["swaybg","-i",str(artifacts/"wallpaper.png"),"-m","stretch"],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
        clients.append(paper)
        wait(lambda:"wallpaper" in ctl("-j","layers"),"background layer wallpaper")
        ok("hypr2d","configure","eDP-1","9","9","true","10")
        ok("hypr2d","overview-config",'{"supersample":2,"memory_budget_mib":128,"background_darken":0.35}')
        visual_metrics=[]
        for full_grid,completed_gpu in ((False,False),(True,False),(False,True),(True,True)):
            ok("hypr2d","overview","profile" if completed_gpu else "profile off")
            ok("hypr2d","overview-config",json.dumps({"fit":not full_grid,"supersample":2,"memory_budget_mib":128,"background_darken":0.35}))
            visual_before=invariants()
            began=time.monotonic()
            open_ui()
            state=ui_state()
            state["open_wall_ms"]=(time.monotonic()-began)*1000
            assert state["first_frame_capture_ms"]==0,"first overview frame must not capture"
            assert state["opening_dispatch_ms"]<30,"overview opening must defer heavy work"
            assert state["captures"]==8 and state["backdrop_captures"]==1,state
            time.sleep(.3)
            assert ui_state()["captures"]==state["captures"],"static clients must not enter a capture/frame-callback loop"
            for rect in [c["rect"] for c in state["ui"]["cells"]]+[l["rect"] for l in state["ui"]["labels"]]:
                x,y,w,h=rect
                assert x>=11.9 and y>=11.9 and x+w<=2244.1 and y+h<=1492.1,rect
            assert state["max_frame_ms"]>0,"native frame timing must be measured after render data reset"
            assert state["gpu_bytes_bound"]<=128*1024*1024,state
            assert state["backdrop_captures"]>=1
            for image in state["images"]:
                assert image["width"]>=image["drawn_width"] and image["height"]>=image["drawn_height"],image
            visual_name=("visual-full" if full_grid else "visual-fit")+("-profile" if completed_gpu else "")
            screenshot(visual_name)
            image=Image.open(artifacts/(visual_name+"-0.png")).convert("RGB")
            # Exposed backdrop must contain the test wallpaper, not flat black.
            assert sum(image.getpixel((10,10)))>50,image.getpixel((10,10))
            empty=next(c for c in state["ui"]["cells"] if not c["occupied"] and not c["shown"])
            x,y,w,h=empty["rect"]
            px,py=round(x+w*.25),round(y+h*.25)
            colour=(64,96,128) if px<1128 and py<752 else (128,80,64) if py<752 else (64,96,80) if px<1128 else (96,64,128)
            assert all(abs(a-(b*.65*(1-26/255)+26))<5 for a,b in zip(image.getpixel((px,py)),colour)),"empty cells must show continuous screen wallpaper"
            if not full_grid and not completed_gpu:
                target=next(c for c in state["ui"]["cells"] if c["occupied"] and not c["shown"])
                x,y,w,h=target["rect"]
                count=state["captures"]
                lua(f'hl.dsp.cursor.move({{x={8934+x+w/2},y={6682+y+h/2}}})')
                time.sleep(.15)
                hovered=next(c for c in ui_state()["ui"]["cells"] if c["key"]==target["key"])
                assert abs(hovered["rect"][2]/w-1.02)<.001,hovered
                assert ui_state()["captures"]==count,"hover must reuse supersampled textures"
                screenshot("visual-hover")
                lua('hl.dsp.cursor.move({x=8944,y=6692})')
                time.sleep(.15)
            # Changing fit/full re-plans only visible occupied cells at 2x.
            original_widths={i["key"]:i["width"] for i in state["images"]}
            keypress("a")
            wait(lambda:ui_state().get("pending")==0,"supersampled fit/full recapture")
            time.sleep(.2)
            resized=ui_state()
            assert any(i["width"]!=original_widths.get(i["key"]) for i in resized["images"]),"layout changes must resize supersampled targets"
            visual_metrics.append({k:state[k] for k in ("opening_dispatch_ms","first_frame_ms","first_frame_capture_ms","max_frame_ms","opening_max_frame_gap_ms","completed_gpu","gpu_rgba_bytes","gpu_bytes_bound","captures","backdrop_captures","capture_ms","max_capture_ms","max_refresh_ms","open_wall_ms")}|{"full_grid":full_grid,"images":state["images"]})
            keypress("Escape");wait(lambda:ui_state().get("open") is False,"visual close")
            assert ui_state()["deleted_gl_objects"],"backdrop and mipmap resources deleted"
            assert invariants()==visual_before,"wallpaper/supersampling preserve focus/group/workspace state"
        (artifacts/"visual-cost.json").write_text(json.dumps(visual_metrics,indent=2))
        # Four headless outputs: three equal neighbours and an unequal fourth.
        # Check horizontal-only and then vertical adjacency, including drag ink.
        assert not ui_state().get("open",False)
        ok("eval",'hl.monitor({output="eDP-1",mode="1280x720@60",position="0x0",scale=1})')
        for name,position,mode in [("GUIDE-MIDDLE","1280x0","1280x720@60"),("GUIDE-RIGHT","2560x0","1280x720@60"),("GUIDE-EXTRA","3840x0","1024x768@60")]:
            ok("output","create","headless",name)
            ok("eval",f'hl.monitor({{output="{name}",mode="{mode}",position="{position}",scale=1}})')
        wait(lambda:len(monitors())==4,"four guide outputs")
        mons=monitors()
        for m in mons:ok("hypr2d","configure",m["name"],"9","9","true","10")
        ok("hypr2d","overview-config",'{"fit":false,"tile":"#FFFFFF1A"}')
        lua(f'hl.dsp.focus({{window="address:{first}"}})')
        lua('hl.dsp.cursor.move({x=10,y=10})')
        time.sleep(.3)
        open_ui()
        horizontal=ui_state()["ui"]["guides"]
        assert len(horizontal)==27 and not any(g["vertical"] for g in horizontal),horizontal
        ordered=sorted(mons,key=lambda m:m["x"])
        pairs={(a["id"],b["id"]) for a,b in zip(ordered,ordered[1:])}
        assert {(g["from"],g["to"]) for g in horizontal}==pairs,"never skip middle monitor"
        screenshot("guides-horizontal")
        keypress("Escape");wait(lambda:not ui_state().get("open",False),"horizontal guides close")
        ok("eval",'hl.monitor({output="GUIDE-EXTRA",mode="1024x768@60",position="1280x720",scale=1})')
        mons=monitors()
        time.sleep(.3)
        open_ui()
        guided=ui_state()["ui"]
        assert len(guided["guides"])==27
        assert sum(g["vertical"] for g in guided["guides"])==9
        screenshot("guides-stacked")
        ptr=subprocess.Popen(["hypr2d-test-pointer"],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,start_new_session=True)
        clients.append(ptr);pointer_ack(b"ready")
        begin_drag(first)
        r=drawn_window(first)["rect"]
        move_drag((r[0]+r[2]/2+10,r[1]+r[3]/2+10))
        screenshot("guides-drag")
        # A true guide pixel must change as opacity rises while dragging.
        normal=Image.open(artifacts/"guides-stacked-0.png").convert("RGB")
        dragging=Image.open(artifacts/"guides-drag-0.png").convert("RGB")
        guide=next(g for g in guided["guides"] if not g["vertical"] and g["index"]==7)
        x1,y1=guide["start"];x2,y2=guide["end"]
        gx,gy=round((3*x1+x2)/4),round(y1)
        assert any(normal.getpixel((x,y))!=dragging.getpixel((x,y)) for x in range(gx-1,gx+2) for y in range(gy-1,gy+2)),"drag must brighten guide ink"
        keypress("Escape");pointer_command("up")
        keypress("Escape");wait(lambda:not ui_state().get("open",False),"guide drag cancel")
        ptr.stdin.write(b"quit\n");ptr.stdin.flush();ptr.wait(timeout=10)
        (artifacts/"guides.json").write_text(json.dumps({"horizontal":horizontal,"stacked":guided},indent=2))
        print("PASS guides: 4 headless outputs, unequal resolution, adjacent row/column indices, drag opacity and screenshots",flush=True)
        # Three live grids can cycle assignments; workspace IDs never change.
        three=sorted(monitors(),key=lambda m:m["name"])[:3]
        original_slots={m["name"]:m["slot"] for m in three}
        grid_currents={m["slot"]:m["activeWorkspace"]["id"] for m in monitors()}
        grid_hosts_before={w["address"]:w["workspace"]["id"] for w in mapped() if w["workspace"].get("id",0)>0}
        for a,b in [(three[0],three[1]),(three[1],three[2])]:
            ok("hypr2d","swap-grids",a["name"],b["name"])
            previous=dict(original_slots);original_slots[a["name"]],original_slots[b["name"]]=previous[b["name"]],previous[a["name"]]
            wait(lambda:all(m["slot"]==original_slots[m["name"]] for m in monitors() if m["name"] in original_slots),"three-output grid permutation")
            assert json.loads(ctl("hypr2d","doctor"))["ok"]
            assert all(m["activeWorkspace"]["id"]==grid_currents[m["slot"]] for m in monitors())
            current_state=json.loads(ctl("hypr2d","state","-j"))
            hosts={cell["workspace"]:m["id"] for m in current_state["monitors"] for cell in m["cells"]}
            assert all(real_window(a)["workspace"]["id"]==ws and real_window(a)["monitor"]==hosts[ws] for a,ws in grid_hosts_before.items())
        for a,b in [(three[1],three[2]),(three[0],three[1])]:
            ok("hypr2d","swap-grids",a["name"],b["name"])
            original_slots[a["name"]],original_slots[b["name"]]=original_slots[b["name"]],original_slots[a["name"]]
            wait(lambda:all(m["slot"]==original_slots[m["name"]] for m in monitors() if m["name"] in original_slots),"three-output restore")
        paper.terminate();paper.wait(timeout=5)
        clients.remove(paper)
        print("PASS visual: wallpaper background/empty desktops, budgeted 2x supersampling, fit/full 9x9 screenshots and completed-GPU timing",visual_metrics,flush=True)
        print("PASS hardware: eDP-1 2256x1504 scale1 at 8934x6682, display-sized captures/fit toggle, software cursor screenshot, group/focus/order/workspace/special preservation")
        # Close fixtures normally before compositor shutdown. This also proves
        # group/special bookkeeping survives native unmap after the drop tests.
        assert json.loads(ctl("hypr2d","doctor"))["ok"]
        for win in mapped():lua(f'hl.dsp.window.close({{window="address:{win["address"]}"}})')
        wait(lambda:not mapped(),"all fixture clients close normally")
        assert json.loads(ctl("hypr2d","doctor"))["ok"]
        # Unload/reload comes LAST: loading a plugin may reload Lua config,
        # which must not race the exact monitor and grouped-window fixture.
        probe("full")
        ok("plugin", "unload", plugin)
        time.sleep(0.5)
        ok("plugin", "load", plugin)
        # Startup/reconnect restore policies are tested against persisted divergent
        # preferences and fallback assignments, not merely the Rust planner.
        state_root=Path(env.get("XDG_STATE_HOME",str(Path.home()/".local/state")))/"hypr2d"
        config_root=Path(env.get("XDG_CONFIG_HOME",str(Path.home()/".config")))/"hypr"
        config_root.mkdir(parents=True,exist_ok=True)
        slot_state=json.loads((state_root/"slots.json").read_text())
        live_slots={m["slot"] for m in monitors()}
        free_slot=next(e["slot"] for e in slot_state["entries"] if e["slot"] not in live_slots)
        for policy,expected in [("auto",0),("keep",free_slot)]:
            ok("plugin","unload",plugin)
            stored=json.loads((state_root/"grids.json").read_text())
            stored["preferred"]["0"]=0;stored["last"]["0"]=free_slot
            (state_root/"grids.json").write_text(json.dumps(stored))
            (config_root/"hypr2d.json").write_text(json.dumps({"gridRestore":policy,"monitors":{"labels":{"GUIDE-MIDDLE":"centre configured"},"autoLabel":"position"},"specials":{"names":{"visual-0":"configured special"}}}))
            ok("plugin","load",plugin)
            wait(lambda:next(m for m in monitors() if m["name"]=="eDP-1")["slot"]==expected,"persisted restore "+policy)
            ok("output","remove","eDP-1")
            wait(lambda:all(m["name"]!="eDP-1" for m in monitors()),"restore policy unplug")
            ok("output","create","headless","eDP-1")
            wait(lambda:any(m["name"]=="eDP-1" and m["slot"]==expected for m in monitors()),"reconnect restore "+policy)
            assert json.loads(ctl("hypr2d","doctor"))["ok"]
        print("PASS assignment persistence: native restart/reconnect auto/keep, three live swaps",flush=True)
        results = {"instance":instance.name,"small":small,"runs":runs,"screenshot_changed_pixels":diffs,"ui_cycles":ui_runs,"final_memory":process_memory()}
        (artifacts / "results.json").write_text(json.dumps(results, indent=2))
        ctl("dispatch","hl.dsp.exit()")
        child.wait(timeout=10)
        assert child.returncode==0,("clean compositor exit",child.returncode)
        print(f"PASS M4 rendering prototype: 2 outputs, normal/special capture, 25 full cycles, deleted GL objects, exact screenshots; {artifacts}")
    except Exception:
        if child.poll() is None and instance:
            try:
                print("FAILED overview diagnostics:",ctl("hypr2d","overview","state"),ctl("-j","clients"),ctl("hypr2d","doctor"),flush=True)
            except (subprocess.SubprocessError, OSError):
                pass
        print("\n".join(line for line in (artifacts/"compositor.log").read_text().splitlines() if "accepted fd" not in line)[-24000:],flush=True)
        raise
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
