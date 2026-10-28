"""Disposable child-only crash postmortem and recovery regression."""
import json
import os
from pathlib import Path
import signal
import resource
import subprocess
import sys
import time

config, plugin = sys.argv[1:]
artifacts = Path('/tmp/hypr2d-doctor-results')
artifacts.mkdir(exist_ok=True)
root = Path(os.environ['XDG_RUNTIME_DIR']) / 'hypr'
# The deliberate crash needs its text backtrace, not a gigabyte core dump.
resource.setrlimit(resource.RLIMIT_CORE,(0,0))
for unsafe in [True, False]:
    tag = 'legacy-crash' if unsafe else 'repair'
    clients = []
    with (artifacts / f'{tag}.log').open('w+') as log:
        command = ['Hyprland', '--config', config]
        if os.geteuid() == 0:
            command.append('--i-am-really-stupid')
        child = subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True)
        try:
            def wait(test, message, timeout=20):
                deadline=time.monotonic()+timeout
                while not test():
                    assert child.poll() is None, (message, child.returncode)
                    assert time.monotonic()<deadline, message
                    time.sleep(.02)
            def discover():
                for lock in root.glob('*/hyprland.lock'):
                    try:
                        if lock.read_text().splitlines()[0]==str(child.pid) and (lock.parent/'.socket.sock').exists():
                            return lock.parent
                    except (OSError,IndexError):
                        pass
            wait(discover,'doctor child IPC')
            instance=discover()
            def ctl(*args, check=True):
                r=subprocess.run(['hyprctl','-i',instance.name,*args],text=True,capture_output=True,timeout=10)
                if check: assert r.returncode==0,(args,r.stdout,r.stderr)
                return r.stdout.strip()
            def ok(*args):
                result=ctl(*args)
                assert result=='ok',(args,result)
            def lua(expr): ok('dispatch',expr)
            def mapped(): return json.loads(ctl('-j','clients'))
            def diagnosis(): return json.loads(ctl('hypr2d','doctor'))
            def outputs(): return json.loads(ctl('-j','monitors'))
            # Each child owns a reliable headless output, even if the parent
            # retires the Wayland bootstrap after the deliberate crash.
            ok('output','create','headless','eDP-1')
            wait(lambda:any(m['name']=='eDP-1' for m in outputs()),'eDP headless')
            ok('eval','hl.monitor({output="eDP-1",mode="2256x1504@60",position="8934x6682",scale=1}); hl.config({animations={enabled=false},input={follow_mouse=0},cursor={no_hardware_cursors=1},group={focus_removed_window=false}})')
            for m in outputs():
                if m['name']!='eDP-1': ok('output','remove',m['name'])
            wait(lambda:len(outputs())==1,'single laptop monitor')
            mon=outputs()[0]
            assert (mon['width'],mon['height'],mon['x'],mon['y'],mon['scale'])==(2256,1504,8934,6682,1)
            env=dict(os.environ,WAYLAND_DISPLAY=(instance/'hyprland.lock').read_text().splitlines()[1])
            def spawn(app,x):
                ok('hypr2d','goto',str(x),'0',str(mon['id']))
                p=subprocess.Popen(['foot','--config=/dev/null',f'--app-id={app}','sh','-c','sleep 600'],env=env,stdout=log,stderr=log,start_new_session=True)
                clients.append(p)
                wait(lambda:any(w['class']==app for w in mapped()),'doctor client map')
                return next(w['address'] for w in mapped() if w['class']==app)
            def win(addr): return next(w for w in mapped() if w['address']==addr)
            beeper=spawn('doctor-beeper',3)
            lua(f'hl.dsp.window.move({{window="address:{beeper}",workspace="special:3",follow=false}})')
            lua(f'hl.dsp.group.toggle({{window="address:{beeper}"}})')
            kitty=spawn('doctor-kitty',2) # workspace 21 must expire after the old swap
            single=spawn('doctor-singleton',4)
            lua(f'hl.dsp.group.toggle({{window="address:{single}"}})')
            sentinel=spawn('doctor-herdr',0)
            partner=spawn('doctor-herdr-partner',1)
            lua(f'hl.dsp.group.toggle({{window="address:{sentinel}"}})')
            ok('eval',f'local a=assert(hl.get_window("address:{sentinel}")); local b=assert(hl.get_window("address:{partner}")); a.group:add(b)')
            lua(f'hl.dsp.focus({{window="address:{partner}"}})')
            assert diagnosis()['ok'],diagnosis()
            # Execute exactly the removed internal swap. Member ghosts remain
            # special:3 while the group target goes to now-unreferenced ws 21.
            ok('hypr2d','doctor-test','swap',beeper,kitty)
            wait(lambda:not diagnosis()['ok'],'legacy corruption detected')
            before=diagnosis()
            (artifacts/f'{tag}-before.json').write_text(json.dumps(before,indent=2))
            detail=next(d for d in before['details'] if d['window']==beeper)
            assert detail['workspace']=='special:3' and detail['member_workspace']=='special:3',detail
            wait(lambda:next(d for d in diagnosis()['details'] if d['window']==beeper)['layout_workspace']=='','orphan layout workspace expires')
            assert detail['layout_space_exists'],detail # orphan parent, not a null SP<CSpace>
            if unsafe:
                try:
                    ctl('eval',f'local w=assert(hl.get_window("address:{beeper}")); hl.dispatch(hl.dsp.window.move({{window=w,out_of_group=true}}))',check=False)
                except subprocess.TimeoutExpired:
                    pass
                child.wait(timeout=10)
                assert child.returncode!=0, 'legacy corrupt detach must reproduce the crash'
                assert 'assignToSpace with a null space?' in (artifacts/f'{tag}.log').read_text()
                cache=Path(os.environ.get('XDG_CACHE_HOME',str(Path.home()/'.cache')))/'hyprland'
                for report in cache.glob(f'hyprlandCrashReport{child.pid}*'):
                    (artifacts/report.name).write_bytes(report.read_bytes())
                print('PASS doctor postmortem: legacy swap orphaned group space; public singleton detach crashes isolated child',flush=True)
                continue
            blocked=ctl('hypr2d','carry','right',check=False)
            assert blocked.startswith('error: layout inconsistent'),blocked
            alive={w['address'] for w in mapped()}
            expected={w['address']:(w['workspace']['name'],w['monitor']) for w in mapped()}
            group=win(partner)['grouped']
            focused=json.loads(ctl('-j','activewindow'))['address']
            active=outputs()[0]['activeWorkspace']['id']
            special=outputs()[0]['specialWorkspace']['name']
            def repair():
                assert json.loads(ctl('hypr2d','doctor','repair'))['queued']
                wait(lambda:diagnosis()['repair']['state']!='queued','deferred doctor repair')
                d=diagnosis()
                assert d['repair']['state']=='complete' and d['ok'],d
                assert {w['address'] for w in mapped()}==alive,'repair unmapped a client'
                assert all(p.poll() is None for p in clients),'repair killed a client'
                assert {w['address']:(w['workspace']['name'],w['monitor']) for w in mapped()}==expected,'repair moved windows'
                assert win(partner)['grouped']==group,'healthy group changed'
                assert json.loads(ctl('-j','activewindow'))['address']==focused,'repair changed focus'
                assert (outputs()[0]['activeWorkspace']['id'],outputs()[0]['specialWorkspace']['name'])==(active,special),'repair changed visibility'
                return d
            repaired=repair()
            assert not win(beeper)['grouped'] and not win(single)['grouped']
            (artifacts/'repair-after.json').write_text(json.dumps(repaired,indent=2))
            # Correctly fail both missing source and missing target lookups before
            # any dispatcher can act on the focused sentinel group.
            baseline=mapped()
            for source,target in [('0xdeadbeef',beeper),(beeper,'0xdeadbeef')]:
                error=ctl('hypr2d','doctor-test','missing-script',source,target,check=False)
                assert 'window not found' in error,error
                assert mapped()==baseline,'nil selector mutated the focused window'
            ok('hypr2d','doctor-test','null-ghost',partner)
            assert not diagnosis()['ok']
            repair() # preserve the two-member group's order and current member
            lua(f'hl.dsp.group.toggle({{window="address:{beeper}"}})')
            ok('hypr2d','doctor-test','null-target',beeper)
            assert not diagnosis()['ok']
            repair() # a literally null group space, not just an expired parent
            ok('hypr2d','doctor-test','missing',kitty)
            assert not diagnosis()['ok']
            repair() # same-space early return must not suppress reinsertion
            # Removing a real target itself unsets fullscreen. Instead corrupt
            # only a group ghost, leaving fullscreen intact for recovery to save.
            lua(f'hl.dsp.window.fullscreen_state({{window="address:{partner}",internal=2,client=0}})')
            fs=(win(partner)['fullscreen'],win(partner)['fullscreenClient'])
            ok('hypr2d','doctor-test','null-ghost',partner)
            assert (win(partner)['fullscreen'],win(partner)['fullscreenClient'])==fs
            repair()
            assert (win(partner)['fullscreen'],win(partner)['fullscreenClient'])==fs,(win(partner),fs)
            lua(f'hl.dsp.window.fullscreen_state({{window="address:{partner}",internal=0,client=0}})')
            lua(f'hl.dsp.window.float({{window="address:{kitty}",action="set"}})')
            geometry=(win(kitty)['at'],win(kitty)['size'])
            ok('hypr2d','doctor-test','missing',kitty)
            repair()
            assert win(kitty)['floating'] and (win(kitty)['at'],win(kitty)['size'])==geometry,(win(kitty),geometry)
            again=repair()
            assert again['repair']['rebuilt']==[] and again['repair']['dissolved']==[]
            for w in list(mapped()): lua(f'hl.dsp.window.close({{window="address:{w["address"]}"}})')
            wait(lambda:not mapped(),'doctor fixture normal unmap')
            assert diagnosis()['ok']
            ok('dispatch','hl.dsp.exit()')
            child.wait(timeout=15)
            assert child.returncode==0,child.returncode
            print('PASS doctor repair: orphan/null/ghost/missing spaces, safe singleton cleanup, preserved group/focus/order/visibility/clients, stale Lua selectors and idempotence',flush=True)
        finally:
            for p in clients:
                if p.poll() is None:
                    os.killpg(p.pid,signal.SIGTERM)
                try:p.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(p.pid,signal.SIGKILL);p.wait()
            if child.poll() is None:
                os.killpg(child.pid,signal.SIGTERM)
                try:child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid,signal.SIGKILL);child.wait()
