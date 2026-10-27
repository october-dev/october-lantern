#!/usr/bin/env python3
"""Opt-in real-driver recovery test on macOS/Linux. No model, captures or UI input.
Usage: python3 scripts/smoke-driver-recovery.py path/to/debug/lantern-engine path/to/cached/cua-driver
Use the pinned Cua binary already checksum-verified by Lantern. The driver permissions gate
is disabled only in this metadata/lifecycle fixture, so this does not qualify desktop access.
"""
import hashlib,json,os,pathlib,queue,shutil,sqlite3,subprocess,sys,tempfile,threading,time
if os.name == 'nt' or len(sys.argv) != 3:
    raise SystemExit(__doc__)
engine=str(pathlib.Path(sys.argv[1]).resolve())
driver=str(pathlib.Path(sys.argv[2]).resolve())
folder=tempfile.mkdtemp(prefix='lantern-crash-',dir='/tmp')
try:
    profile=pathlib.Path(folder)
    env=dict(os.environ,LANTERN_TEST_PROFILE=folder,LANTERN_CUA_BIN=driver,CUA_DRIVER_RS_PERMISSIONS_GATE='0')
    assert pathlib.Path(subprocess.check_output([engine,'test-profile'],env=env,text=True).strip()).resolve()==profile.resolve()
    log=(profile/'engine-qa.log').open('w')
    hosts=[]
    def start():
        events=queue.Queue()
        host=subprocess.Popen([engine,'serve'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,text=True,bufsize=1)
        hosts.append(host)
        def read():
            for line in host.stdout:events.put(json.loads(line))
        threading.Thread(target=read,daemon=True).start()
        return host,events
    def until(events,predicate,timeout=25):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            event=events.get(timeout=max(.01,end-time.monotonic()))
            if predicate(event):return event
        raise AssertionError('Protocol timeout')
    def send(host,request):host.stdin.write(json.dumps(request)+'\n');host.stdin.flush()
    daemon=None
    try:
        host,events=start()
        until(events,lambda e:e.get('type')=='routines')
        send(host,dict(type='computer.prepare',enabled=True))
        event=until(events,lambda e:e.get('type')=='computer' and e.get('status') in ['ready','failed'],timeout=40)
        assert event['status']=='ready',event
        daemon=int((profile/'cua-driver.pid').read_text().strip())
        db=sqlite3.connect(profile/'cards/cards.db',timeout=5)
        # Reproduce the exact crash gap: the hold is committed, process dies before revocation.
        db.execute('INSERT INTO computer_hold VALUES(1,1) ON CONFLICT(id) DO UPDATE SET held=1');db.commit()
        contender,contender_events=start()
        rejected=until(contender_events,lambda e:e.get('type')=='routines')
        assert rejected.get('error'), 'Second engine unexpectedly owns the task service'
        assert subprocess.run([driver,'status','--socket',str(profile/'cua-driver.sock'),'--pid-file',str(profile/'cua-driver.pid')],capture_output=True).returncode==0
        contender.stdin.close();contender.wait(timeout=10)
        host.kill();host.wait(timeout=10)
        assert subprocess.run([driver,'status','--socket',str(profile/'cua-driver.sock'),'--pid-file',str(profile/'cua-driver.pid')],capture_output=True).returncode==0
        os.kill(daemon,0)
        restored,events=start()
        held=until(events,lambda e:e.get('type')=='computerHold' and e.get('message','').startswith('App access is paused.'))
        assert held['held'] is True
        assert subprocess.run([driver,'status','--socket',str(profile/'cua-driver.sock'),'--pid-file',str(profile/'cua-driver.pid')],capture_output=True).returncode!=0
        end=time.monotonic()+8
        while time.monotonic()<end:
            try:os.kill(daemon,0)
            except ProcessLookupError:break
            time.sleep(.1)
        else:raise AssertionError('The actual orphan daemon process survived restored pause confirmation')
        assert db.execute('SELECT held FROM computer_hold WHERE id=1').fetchone()[0]==1
        send(restored,dict(type='computer.prepare',enabled=True))
        off=until(events,lambda e:e.get('type')=='computer' and e.get('status')=='off')
        assert 'paused' in off['message']
        db.close()
        record=dict(kind='Real engine SIGKILL/restart, pinned Cua metadata-only daemon; no model/UI input',checks=['second engine left the live owner driver untouched','driver survived engine SIGKILL in durable-hold/revocation gap','restored hold stopped actual orphan daemon before paused confirmation','new driver start refused while held'],engineSha256=hashlib.sha256(pathlib.Path(engine).read_bytes()).hexdigest())
        print(json.dumps(record,indent=2))
        print('PASS: actual engine crash gap, orphan Cua shutdown before pause confirmation, and held restart fencing')
    finally:
        for host in hosts:
            if host.poll() is None:
                host.stdin.close()
                try:host.wait(timeout=10)
                except subprocess.TimeoutExpired:host.terminate();host.wait(timeout=10)
        args=[driver,'stop','--socket',str(profile/'cua-driver.sock')]
        if daemon is not None:args += ['--expected-pid',str(daemon)]
        subprocess.run(args,capture_output=True,timeout=8)
        log.close()
finally:
    # Preserve diagnostic files if cleanup itself cannot be confirmed.
    if 'daemon' in locals() and daemon is not None:
        try:os.kill(daemon,0)
        except ProcessLookupError:pass
        else:raise RuntimeError(f'QA daemon exit unconfirmed; private profile retained at {folder}')
    shutil.rmtree(folder)
