"""First-class grid integration; all IPC targets this disposable child PID."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

config, plugin, cli = sys.argv[1:]
artifacts=Path('/tmp/hypr2d-catalog-results');artifacts.mkdir(exist_ok=True)
config_dir=Path(os.environ['XDG_CONFIG_HOME'])/'hypr';config_dir.mkdir(exist_ok=True)
(config_dir/'hypr2d.json').write_text(json.dumps({'grid':{'stride':10,'monitors':{'eDP-1':{'columns':9,'rows':9,'wrap':True}}}}))
root=Path(os.environ['XDG_RUNTIME_DIR'])/'hypr';clients=[];instance=None
with (artifacts/'compositor.log').open('w+') as log:
    child=subprocess.Popen(['Hyprland','--i-am-really-stupid','--config',config],stdout=log,stderr=log,start_new_session=True)
    try:
        def wait(test,label,timeout=25):
            deadline=time.monotonic()+timeout
            while not test():
                assert child.poll() is None,(label,child.returncode)
                assert time.monotonic()<deadline,label
                time.sleep(.025)
        def discover():
            global instance
            for lock in root.glob('*/hyprland.lock'):
                try:
                    if lock.read_text().splitlines()[0]==str(child.pid) and (lock.parent/'.socket.sock').exists():instance=lock.parent;return True
                except (OSError,IndexError):pass
            return False
        wait(discover,'catalogue IPC')
        def ctl(*args,check=True):
            result=subprocess.run(['hyprctl','-i',instance.name,*args],text=True,capture_output=True,timeout=15)
            if check:assert result.returncode==0,(args,result.stdout,result.stderr)
            return result.stdout.strip()
        def ok(*args):
            result=ctl(*args);assert result=='ok',(args,result)
        def lua(expr):ok('dispatch',expr)
        def state():return json.loads(ctl('hypr2d','state','-j'))
        def monitors():return json.loads(ctl('-j','monitors'))
        def mapped():return json.loads(ctl('-j','clients'))
        def win(address):return next(w for w in mapped() if w['address']==address)
        def grid(name):return next(g for g in state()['grids'] if g['name']==name)
        def run(*args,check=True):
            r=subprocess.run([cli,'--instance',instance.name,*args],capture_output=True,text=True,timeout=15)
            if check:assert r.returncode==0,(args,r.stdout,r.stderr)
            return r
        def healthy():
            result=json.loads(ctl('hypr2d','doctor'));assert result['ok'],result
            assert all(p.poll() is None for p in clients),'a client terminated'
            assert all(len(w['grouped'])!=1 for w in mapped()),'singleton group introduced'
        def invariants():return json.loads(ctl('hypr2d','overview','invariants'))
        ok('output','create','headless','eDP-1')
        wait(lambda:any(m['name']=='eDP-1' for m in monitors()),'laptop output')
        ok('eval','hl.monitor({output="eDP-1",mode="2256x1504@60",position="8934x6682",scale=1}); hl.config({animations={enabled=false},input={follow_mouse=0},cursor={no_hardware_cursors=1}})')
        for m in monitors():
            if m['name']!='eDP-1':ok('output','remove',m['name'])
        wait(lambda:len(monitors())==1,'single laptop')
        edp=monitors()[0]
        assert (edp['width'],edp['height'],edp['x'],edp['y'],edp['scale'])==(2256,1504,8934,6682,1)
        assert state()['monitors'][0]['grid']['columns']==9,'load config before restore'
        env=dict(os.environ,WAYLAND_DISPLAY=(instance/'hyprland.lock').read_text().splitlines()[1])
        def spawn(app,x,y):
            ok('hypr2d','goto',str(x),str(y),str(edp['id']))
            p=subprocess.Popen(['foot','--config=/dev/null',f'--app-id={app}','sh','-c','sleep 600'],env=env,stdout=log,stderr=log,start_new_session=True);clients.append(p)
            wait(lambda:any(w['class']==app for w in mapped()),'fixture map')
            return next(w['address'] for w in mapped() if w['class']==app)
        a=spawn('catalog-group-a',8,8);b=spawn('catalog-group-b',8,8);floating=spawn('catalog-floating',2,1)
        lua(f'hl.dsp.group.toggle({{window="address:{a}"}})')
        ok('eval',f'local a=assert(hl.get_window("address:{a}")); a.group:add(assert(hl.get_window("address:{b}")))')
        lua(f'hl.dsp.group.active({{window="address:{a}",index=1}})')
        lua(f'hl.dsp.window.float({{window="address:{floating}",action="set"}})')
        expected={w['address'] for w in mapped()};healthy()
        current=next(g for g in invariants()['groups'] if a in g['members'])['current']
        created=json.loads(run('grid','create','work',json.dumps({'columns':9,'rows':9})).stdout)
        work=created['slot'];assert work>=0x40000000
        origin=grid('work')['workspace'];assert origin>1_000_000_000
        run('adopt','work','eDP-1');wait(lambda:state()['monitors'][0]['slot']==work,'named adoption')
        assert state()['monitors'][0]['workspace']==origin
        assert win(a)['workspace']['id']==801 and win(b)['workspace']['id']==801,'legacy numbering changed'
        destination=next(c['workspace'] for c in state()['monitors'][0]['cells'] if c['x']==5 and c['y']==4)
        for address in [a,floating]:lua(f'hl.dsp.window.move({{window="address:{address}",workspace={destination},follow=false}})')
        wait(lambda:all(win(w)['workspace']['id']==destination for w in [a,b,floating]),'public move into named grid')
        healthy();assert next(g for g in invariants()['groups'] if a in g['members'])['current']==current
        ok('hypr2d','goto','8','8',str(edp['id']));ok('hypr2d','move','right')
        assert state()['monitors'][0]['cell']=={'x':0,'y':8},'virtual grid wrap'
        lua(f'hl.dsp.focus({{window="address:{floating}"}})');ok('hypr2d','carry','down')
        assert state()['monitors'][0]['cell']=={'x':5,'y':5}
        healthy()
        last=state()['monitors'][0]['workspace']
        for name,position,mode in [('RIGHT','11190x6682','1920x1080@60'),('UP','8934x5482','1600x1200@60')]:
            ok('output','create','headless',name);ok('eval',f'hl.monitor({{output="{name}",mode="{mode}",position="{position}",scale=1}})')
            wait(lambda:any(m['name']==name for m in monitors()),'additional output')
        for source,target in [('eDP-1','RIGHT'),('RIGHT','UP')]:
            run('swap-grids',source,target)
            wait(lambda:any(m['name']==target and m['slot']==work for m in state()['monitors']),'named grid follows swap')
            target_id=next(m['id'] for m in monitors() if m['name']==target)
            assert all(win(w)['monitor']==target_id for w in [a,b,floating])
            assert next(m for m in state()['monitors'] if m['name']==target)['workspace']==last
            healthy();assert next(g for g in invariants()['groups'] if a in g['members'])['current']==current
        # Adopting an already live grid is an exclusive swap, never a merge.
        run('adopt','work','eDP-1');wait(lambda:any(m['name']=='eDP-1' and m['slot']==work for m in state()['monitors']),'adopt live grid swaps')
        healthy()
        run('grid','rename','work','work renamed');assert grid('work renamed')['slot']==work
        assert run('grid','delete','work renamed',check=False).returncode!=0
        lua('hl.plugin.hypr2d.grid_create("lua-grid",{columns=3,rows=3,wrap=false})')
        wait(lambda:any(g['name']=='lua-grid' for g in state()['grids']),'Lua grid create')
        retired=grid('lua-grid')['slot']
        lua('hl.plugin.hypr2d.grid_rename("lua-grid","scratch grid")')
        wait(lambda:any(g['name']=='scratch grid' for g in state()['grids']),'Lua grid rename')
        lua('hl.plugin.hypr2d.grid_delete("scratch grid")')
        wait(lambda:all(g['slot']!=retired for g in state()['grids']),'Lua empty-grid delete')
        other=json.loads(run('grid','create','another').stdout)['slot'];assert other!=retired
        assert {w['address'] for w in mapped()}==expected
        healthy()
        lua('hl.dsp.focus({monitor="eDP-1"})');before=invariants()
        ok('hypr2d','overview','open');wait(lambda:json.loads(ctl('hypr2d','overview','state')).get('pending')==0,'catalogue overview capture')
        time.sleep(.25)
        subprocess.run(['grim','-o','eDP-1',str(artifacts/'catalog-backend-2256.png')],env=env,check=True,timeout=20)
        ok('hypr2d','overview','close');wait(lambda:not json.loads(ctl('hypr2d','overview','state')).get('open',True),'catalogue overview closes')
        assert invariants()==before,'named-grid captures changed native state'
        run('adopt','0','eDP-1');wait(lambda:any(m['name']=='eDP-1' and m['slot']==0 for m in state()['monitors']),'restore legacy grid')
        edp=next(m for m in monitors() if m['name']=='eDP-1');ok('hypr2d','goto','8','8',str(edp['id']))
        assert next(m for m in state()['monitors'] if m['name']=='eDP-1')['workspace']==801
        layout={m['name']:(m['slot'],m['workspace']) for m in state()['monitors']}
        ok('plugin','unload',plugin);ok('plugin','load',plugin)
        wait(lambda:{m['name']:(m['slot'],m['workspace']) for m in state()['monitors']}==layout,'catalogue assignment/current persistence')
        assert grid('work renamed')['slot']==work;healthy()
        assert {w['address'] for w in mapped()}==expected
        (artifacts/'state.json').write_text(json.dumps(state(),indent=2))
        for address in expected:lua(f'hl.dsp.window.close({{window="address:{address}"}})')
        wait(lambda:not mapped(),'fixture close');assert json.loads(ctl('hypr2d','doctor'))['ok']
        print('PASS catalogue: named grids, legacy IDs, grouped/floating clients, wrap/carry, 2/3-output swaps, Lua CRUD, capture and restart',flush=True)
        ctl('dispatch','hl.dsp.exit()');child.wait(timeout=10);assert child.returncode==0
    except Exception:
        print((artifacts/'compositor.log').read_text()[-24000:],flush=True)
        if child.poll() is None and instance:
            try:print('FAILED catalogue:',ctl('hypr2d','state','-j'),ctl('-j','clients'),ctl('hypr2d','doctor'),flush=True)
            except subprocess.SubprocessError:pass
        raise
    finally:
        if child.poll() is None and instance:
            try:ctl('dispatch','hl.dsp.exit()');child.wait(timeout=5)
            except (subprocess.SubprocessError,OSError):pass
        for process in [*clients,child]:
            try:os.killpg(process.pid,signal.SIGKILL)
            except ProcessLookupError:pass
            process.wait(timeout=10)
