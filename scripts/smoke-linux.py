#!/usr/bin/env python3
"""Exercise Linux discovery and safe delivery against a harmless isolated tmux agent."""
import json
import os
import pathlib
import queue
import shlex
import subprocess
import tempfile
import threading
import time
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ENGINE = ROOT / "build/linux/lantern-engine"
if not ENGINE.is_file():
    raise SystemExit("Build the Linux app first: scripts/build-app.sh --debug")
SOCKET = f"lantern-smoke-{os.getpid()}"
BUS = "--bus" in sys.argv

with tempfile.TemporaryDirectory(prefix="lantern smoke-") as folder:
    folder = pathlib.Path(folder)
    fake = folder / "claude"
    received = folder / "reply.txt"
    args_file = folder / "args.json"
    # A harmless interactive fixture. No real AI agent receives any test instructions.
    fake.write_text(
        "#!/usr/bin/python3\nimport sys,pathlib,json\n"
        f"pathlib.Path({str(args_file)!r}).write_text(json.dumps(sys.argv[1:]))\n"
        "for line in sys.stdin:\n"
        f" pathlib.Path({str(received)!r}).write_text(line)\n"
    )
    fake.chmod(0o700)
    # Named bash: the engine reads the login environment only from shells it knows (zsh, bash, fish).
    (folder / "login").mkdir()
    shell = folder / "login" / "bash"
    shell.write_text(
        "#!/bin/sh\n"
        f"export PATH={shlex.quote(str(folder))}:\"$PATH\"\n"
        'exec /bin/bash --noprofile --norc "$@"\n'
    )
    shell.chmod(0o700)
    subprocess.run(
        ["tmux", "-L", SOCKET, "new-session", "-d", "-s", "smoke", "-c", str(folder),
         f"exec {shlex.quote(str(fake))}"],
        check=True,
    )
    env = dict(os.environ, XDG_DATA_HOME=str(folder / "data"), SHELL=str(shell),
               OCTOBER_BUS_RUNTIME_DIR=str(folder / "bus-runtime"),
               OCTOBER_BUS_DATA_DIR=str(folder / "bus-data"))
    proc = subprocess.Popen(
        [str(ENGINE), "serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, text=True, env=env, bufsize=1,
    )
    events = queue.Queue()
    launched_session = None

    def reader():
        for line in proc.stdout:
            events.put(json.loads(line))
        events.put(None)

    threading.Thread(target=reader, daemon=True).start()

    def read_until(predicate, seconds=20):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                value = events.get(timeout=max(0.01, deadline - time.monotonic()))
            except queue.Empty:
                break
            if value is None:
                raise RuntimeError("engine exited: " + proc.stderr.read())
            if predicate(value):
                return value
        raise RuntimeError("engine response timed out")

    def send(request):
        proc.stdin.write(json.dumps(request) + "\n")
        proc.stdin.flush()

    try:
        read_until(lambda v: v.get("type") == "hello")
        snapshot = read_until(
            lambda v: v.get("type") == "snapshot"
            and any(a.get("cwd") == str(folder) for a in v["agents"])
        )
        agent = next(a for a in snapshot["agents"] if a.get("cwd") == str(folder))
        assert agent["canReply"] and agent["route"]["via"] == "tmux", agent
        assert agent["tty"].startswith("pts/"), agent
        send({"type": "reply", "requestId": "smoke-reply", "agentId": agent["id"],
              "text": "Linux sidecar delivery works"})
        result = read_until(lambda v: v.get("requestId") == "smoke-reply")
        assert result["ok"], result
        for _ in range(30):
            if received.exists():
                break
            time.sleep(0.1)
        assert received.read_text().strip() == "Linux sidecar delivery works"

        # Launch through the actual protocol, with spaces and shell metacharacters in the prompt.
        prompt = "hello 'Linux' $HOME; this is literal"
        send({"type": "launch", "requestId": "smoke-launch", "kind": "claude",
              "cwd": str(folder), "prompt": prompt, "background": True, "bus": BUS})
        result = read_until(lambda v: v.get("requestId") == "smoke-launch")
        assert result["ok"], result
        launched_session = result["session"]
        for _ in range(50):
            if json.loads(args_file.read_text()) and prompt in json.loads(args_file.read_text())[-1]:
                break
            time.sleep(0.1)
        assert prompt in json.loads(args_file.read_text())[-1]
        if BUS:
            assert "--mcp-config" in json.loads(args_file.read_text())
            scopes = subprocess.check_output([str(ENGINE.with_name("october-bus")), "scope", "list"], env=env, text=True)
            assert '"lantern"' in scopes, scopes

        # Replace the original agent with a shell on the same tty; stale requests must be refused.
        subprocess.run(
            ["tmux", "-L", SOCKET, "respawn-pane", "-k", "-t", "smoke:0.0", "exec /bin/sh"],
            check=True,
        )
        send({"type": "reply", "requestId": "smoke-stale", "agentId": agent["id"],
              "text": "MUST NOT BE SENT"})
        stale = read_until(lambda v: v.get("requestId") == "smoke-stale")
        assert not stale["ok"], stale
        print("PASS: procfs discovery, named tmux socket, reply delivery, background launch, stale-agent refusal")
        if BUS:
            print("PASS: bundled native Bus daemon, Lantern scope, per-session MCP configuration")
    finally:
        proc.terminate()
        proc.wait(timeout=10)
        if BUS:
            subprocess.run([str(ENGINE.with_name("october-bus")), "stop"], env=env, capture_output=True, timeout=10)
        if launched_session:
            subprocess.run(["tmux", "-L", "lantern", "kill-session", "-t", launched_session],
                           check=False, stderr=subprocess.DEVNULL)
        subprocess.run(["tmux", "-L", SOCKET, "kill-server"], check=False, stderr=subprocess.DEVNULL)
