#!/usr/bin/env python3
"""Disposable protocol tests. No models, user terminals, credentials or live app driver needed.
Usage: python3 scripts/smoke-local-cards.py [path/to/debug/lantern-engine]
"""
import hashlib
import json
import os
from pathlib import Path
import queue
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
ENGINE = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / 'engine/target/debug/lantern-engine'
if os.name == 'nt':
    raise SystemExit('This transport smoke uses Unix sockets. Use native Windows smoke and engine tests on Windows.')

with tempfile.TemporaryDirectory(prefix='lantern-local-', dir='/tmp') as folder:
    profile = Path(folder)
    env = dict(os.environ, LANTERN_TEST_PROFILE=str(profile))
    # Release binaries ignore the fixture override. Refuse before starting any service.
    checked = subprocess.run([str(ENGINE), 'test-profile'], env=env, capture_output=True, text=True, check=True)
    assert Path(checked.stdout.strip()).resolve() == profile.resolve(), 'A debug build with a private profile is required'
    events = queue.Queue()
    errors = (profile / 'engine.log').open('w')
    host = subprocess.Popen([str(ENGINE), 'serve'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=errors, env=env, text=True, bufsize=1)

    def read():
        for line in host.stdout:
            events.put(json.loads(line))
    threading.Thread(target=read, daemon=True).start()

    def send(value):
        host.stdin.write(json.dumps(value) + '\n')
        host.stdin.flush()

    def until(predicate, timeout=15):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            value = events.get(timeout=max(.01, end-time.monotonic()))
            if predicate(value):
                return value
        raise AssertionError('Timed out waiting for protocol result')

    def result(request, kind):
        request['requestId'] = str(uuid.uuid4())
        send(request)
        return until(lambda v: v.get('type') == kind and v.get('requestId') == request['requestId'])

    db = None
    try:
        until(lambda v: v.get('type') == 'routines')
        db = sqlite3.connect(profile/'cards/cards.db', timeout=5)
        launch, key = str(uuid.uuid4()), uuid.uuid4().hex + uuid.uuid4().hex
        db.execute('INSERT INTO launches(id,key,kind,cwd,title,created) VALUES(?,?,?,?,?,?)',
                   (launch, hashlib.sha256(key.encode()).digest(), 'october', str(profile), 'Protocol fixture', int(time.time()*1000)))
        db.commit()

        def ipc(op, **fields):
            request = dict(launch=launch, key=key, op=op, **fields)
            with socket.socket(socket.AF_UNIX) as client:
                client.settimeout(10)
                client.connect(str(profile/'cards/engine.sock'))
                client.sendall((json.dumps(request)+'\n').encode())
                return json.loads(client.makefile().readline())

        operation = str(uuid.uuid4())
        ask = dict(operationId=operation, question='Which word?', options=['amber', 'green'])
        assert ipc('ask', **ask)['ok']
        assert ipc('ask', **ask)['ok']
        assert db.execute('SELECT COUNT(*) FROM cards').fetchone()[0] == 1
        assert not ipc('ask', **dict(ask, question='Different operation body'))['ok']
        card, revision = db.execute('SELECT id,revision FROM cards').fetchone()
        assert result(dict(type='card.answer',cardId=card,revision=revision+1,answer={'choice':'amber'}),'cardResult')['ok'] is False
        assert result(dict(type='card.answer',cardId=card,revision=revision,answer={'choice':'amber'}),'cardResult')['ok'] is True
        assert ipc('cancel', card=card)['ok']
        cancelled = ipc('wait', card=card)
        assert cancelled.get('answer') is None, 'A cancelled answer was replayed'
        assert not result(dict(type='card.answer',cardId=card,revision=revision,answer={'choice':'green'}),'cardResult')['ok']
        print('Cards: idempotency, changed-body replay, stale revisions, cancellation and late-answer refusal passed')

        draft = dict(name='Fixture', prompt='Do nothing', cwd=str(profile), kind='october', timezone='UTC',
                     cadence='daily', hour=9, minute=0, weekday=0, preferences='')
        assert result(dict(type='routine', action='save', draft=draft),'routineResult')['ok']
        routine, revision = db.execute('SELECT id,revision FROM routines').fetchone()
        assert not result(dict(type='routine',action='change',id=routine,revision=revision+1,change='delete'),'routineResult')['ok']
        assert result(dict(type='routine',action='change',id=routine,revision=revision,change='pause'),'routineResult')['ok']
        assert db.execute('SELECT enabled FROM routines').fetchone()[0] == 0
        revision = db.execute('SELECT revision FROM routines').fetchone()[0]
        assert result(dict(type='routine',action='change',id=routine,revision=revision,change='delete'),'routineResult')['ok']
        print('Routines: real protocol create, revision fencing, pause and delete passed; no agent launched')

        run = str(uuid.uuid4())
        send(dict(type='show.start',runId=run))
        capture = until(lambda v: v.get('type')=='showCapture' and v.get('runId')==run)
        send(dict(type='show.stop',runId=run))
        until(lambda v:v.get('type')=='showState' and v.get('runId')==run and v.get('phase')=='ended')
        send(dict(type='show.captured',runId=run,captureId=capture['captureId'],step=1,
                  display=dict(x=0,y=0,w=100,h=100),imageWidth=100,imageHeight=100,markedOnly=True))
        time.sleep(.4)
        queued=[]
        while not events.empty(): queued.append(events.get_nowait())
        assert not any(v.get('type')=='showQuery' and v.get('runId')==run for v in queued)
        print('Show me: Stop fences late capture before any upload request')

        assert result(dict(type='task.control',action='pause'),'controlResult')['ok']
        assert db.execute('SELECT held FROM computer_hold').fetchone()[0] == 1
        assert not ipc('ask', **dict(ask,operationId=str(uuid.uuid4())))['ok']
        print('Control: durable app-access hold and refused card creation while held passed')
    finally:
        host.stdin.close()
        try: host.wait(timeout=10)
        except subprocess.TimeoutExpired:
            host.terminate()
            host.wait(timeout=10)
        if db is not None: db.close()
        errors.close()
print('Local protocol smoke passed. This does not qualify real model or native desktop behavior.')
