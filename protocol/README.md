# Lantern engine protocol (v3)

The app starts `lantern-engine serve`. The two sides exchange one JSON object per line. The engine writes to stdout and reads from stdin. Anything the engine writes to stderr is a log.

## Engine → app

### `hello`
Sent once at startup. The app only talks to an engine whose `protocol` matches its own; otherwise it stops the engine and asks the user to reinstall.
```json
{"type":"hello","protocol":3,"version":"0.3.0"}
```

### `snapshot`
Sent when anything changes, and at least every 10 seconds.
```json
{
  "type": "snapshot",
  "generatedAt": 1790181234567,
  "agents": [Agent, ...]
}
```

`Agent`:

| Field | Type | Notes |
|---|---|---|
| `id` | string | `"<kind>:<pid>:<start time>"`. Stable while the process lives; a reused pid is a different agent. |
| `kind` | string | `claude`, `codex`, `opencode`, `pi`, `october` (the October harness), `gemini`, `grok`, `cursor`, `qwen`, `goose`, `aider`, `amp`, `copilot`, `kimi`, `droid`, `muse`, `crush`, `auggie`. Clients should accept unknown values. |
| `handle` | string | Display handle, e.g. `"claude-2"`. Numbered per kind. A number stays with its agent for the agent's whole life, and a new agent takes the lowest free number. |
| `pid` | number | |
| `startTime` | number | Seconds since the epoch when the process started. Delivery checks the process still has it. |
| `tty` | string? | e.g. `"ttys004"`, `"pts/3"`; on Windows `"con:<pid>"`, the agent's console host (one per console window or terminal tab) |
| `cwd` | string? | Current working directory |
| `project` | string? | Last path component of `cwd` |
| `title` | string? | Session title (Claude's `ai-title`, or the first prompt in a Codex session) |
| `sessionId` | string? | |
| `state` | `"working" \| "waiting" \| "needs_input" \| "idle" \| "unknown"` | `waiting` means the agent finished its turn and it's your turn. `needs_input` means it's blocked on a question or permission (known only from hooks). `unknown` means the session file didn't say (for Codex, a turn longer than the part of the file the engine reads). |
| `stateSince` | number? | Epoch ms of the event that produced `state`: the session file's own timestamp for that event, or the time the hook first reported this state (a repeat, such as Claude's idle reminder after `Stop`, keeps the first time). Only falls back to the file's modification time when the file has no timestamps. Together with `id` it identifies a turn. |
| `lastMessage` | string? | The agent's last message to you (truncated to 2,000 characters) |
| `question` | string? | For `needs_input`: what it's asking |
| `questionKind` | `"permission" \| "other"`? | For `needs_input` from a hook: `permission` is a tool permission prompt (`1` allows once, Escape declines); `other` is anything else, to be answered in the terminal. |
| `questionDetail` | string? | The whole request behind `question`: a permission prompt's full command (every line) or tool input. `question` is one line and says how many lines it leaves out. Cut at 20,000 characters, with a note saying so. |
| `promptId` | string? | Identifies one permission prompt. `keys` for a permission prompt must carry it; the engine refuses keys for a prompt that has since been answered or replaced. |
| `sessionMatch` | `"exact" \| "guessed" \| "ambiguous" \| "none"` | How the session file was matched to this process. `exact`: a hook, `--session-id`/`--resume`, or the open file (Codex). `guessed`: the newest session in the folder that started after the process. `ambiguous`: another agent of the same kind in the same folder is also only guessed, so `state` is `unknown` and `lastMessage`, `question`, `title` and history are withheld (replies still work: they go to the process's own terminal). |
| `host` | `{ "app": string, "pid": number, "bundlePath": string }?` | The GUI app the agent runs inside (Terminal, iTerm2, Ghostty, cmux, ...) |
| `tmux` | `{ "socket": string?, "target": string, "paneId": string }?` | Present when the agent runs inside a tmux pane |
| `canReply` | bool | `true` when the engine can type a reply into the agent (`route.via` isn't `none`) |
| `route` | object | How replies reach the agent. `{"via":"tmux"}`, `{"via":"cmux","workspace":"…","surface":"…"}`, `{"via":"terminal","tty":"/dev/ttys004"}`, `{"via":"iterm","tty":"…"}`, `{"via":"october","canvasId":"…","nodeId":"…"}` (through October Desktop, when Lantern is connected to it), `{"via":"pty","session":"…"}` (one of Lantern's own sessions on Windows) or `{"via":"none"}`. Single keys (`keys`) work for tmux, cmux, iterm and pty. |
| `stateSource` | `"hook" \| "transcript" \| "none"` | Where the state came from ("transcript" covers every session reader) |

### `installed`
Sent shortly after `hello`, and again when October Harness finishes installing. Lists the agents that can be started: those installed on this machine, as found by the user's login shell, plus `october` when Lantern's own October Harness is installed. `harness` is October Harness's status (as in the `harness` event below).
```json
{"type":"installed","installed":{"kinds":["claude","codex","opencode","october"],"tmux":true},"harness":{"status":"ready","source":"bundled","version":"0.87.1-october.3"}}
```

### `harness`
October Harness (the `october` agent, npm `@october-dev/october`) is built in: when `october` isn't on the user's PATH, the engine installs the version pinned in `engine/harness.json` into its support folder the first time it's needed (Node.js from October Desktop's runtime folder when a compatible one, >= 22.19, is there, otherwise the pinned Node.js tarball from nodejs.org checked against its SHA-256; then `npm install --ignore-scripts` of the pinned harness). Answers `harness.prepare`: once with `installing` when an install starts, and again when it's `ready` or `failed`. `status` is `ready` (`source`: `path` for the user's own `october`, `bundled` for Lantern's), `installing`, `failed` (with `message`) or `missing` (not installed yet; installed on first use). `requestId` echoes the request's, when it had one.
```json
{"type":"harness","status":"installing","version":"0.87.1-october.3"}
{"type":"harness","status":"ready","source":"bundled","version":"0.87.1-october.3"}
{"type":"harness","status":"failed","version":"0.87.1-october.3","message":"downloading Node.js: …"}
```

### `launchResult` / `attachResult`
`launchResult` comes after the first message has been typed, for agents that take it that way (up to about 30 seconds). Its `session` is the tmux session the agent runs in; the app opens that agent's conversation as soon as it appears. `attachResult` answers `attach` and `focus`.
```json
{"type":"launchResult","requestId":"l1","ok":true,"session":"codex-myproj-12345"}
{"type":"attachResult","requestId":"a1","ok":false,"message":"agent is not in a tmux session"}
```

### `historyResult`
The recent conversation with an agent (up to 120 messages from the end of its session file). `supported` is `false` for harnesses whose sessions Lantern can't read yet.
```json
{"type":"historyResult","requestId":"h1","agentId":"codex:4242:1790180000","supported":true,
 "messages":[{"role":"user","text":"fix the test","at":"2026-09-23T17:03:00Z"},
             {"role":"tool","text":"Run · npm test"},
             {"role":"agent","text":"Fixed. The test was racing the timer."}]}
```
`role` is `user`, `agent` or `tool` (a one-line summary of a command or tool call).

### `replyResult`
```json
{"type":"replyResult","requestId":"r1","ok":true}
{"type":"replyResult","requestId":"r1","ok":false,"error":"not_reachable","message":"..."}
```
`error` is one of:
- `unknown_agent`, `not_reachable`, `bad_request`: refused before anything was queued.
- `send_failed`: nothing was typed; `message` says why.
- `expired`: it couldn't start within 15 seconds (another send to the same terminal, or macOS's Automation prompt, took too long). Nothing was typed.
- `canceled`: canceled before typing started. Nothing was typed.
- `prompt_changed`: the permission prompt was answered or replaced. Nothing was pressed.
- `uncertain`: typing started, but Lantern can't tell whether it all went in (a helper ran out of time, or the text went in and Enter didn't). Sending again could repeat it.

A reply is only `ok` after the text and Enter reached the terminal. Replies, keys and focus requests run on one worker per terminal, off the scan loop. Immediately before typing (after any Automation prompt has been answered), the engine checks that the terminal still belongs to the agent: the same process (pid and start time), still running the same program (a process that `exec`ed a shell is refused), on the same tty, and in the foreground of that tty, with the route's own terminal (tmux pane, Terminal/iTerm tab) still on that tty. When the foreground is unknown it refuses.

### `october`
The connection to October Desktop, sent whenever it changes.
```json
{"type":"october","october":{"status":"connected","coreVersion":"1.0.52","paired":true,"pairingCode":null,"message":null,"agentCount":3}}
```
`status` is `notInstalled`, `notRunning`, `readOnly` (October is running; Lantern lists its terminals with the credential October gives its own command-line tool), `connected` (the user allowed Lantern in October), `pairing` (`pairingCode` is the 6-digit code October shows) or `error`. Agents that run inside October get `route.via == "october"` while `status` is `connected`.

## App → engine

```json
{"type":"refresh"}
{"type":"reply","requestId":"r1","agentId":"claude:83630:1790180000","text":"Use Postgres"}
```

```json
{"type":"launch","requestId":"l1","kind":"codex","cwd":"/Users/me/proj","prompt":"write tests","screenshot":"/Users/me/Library/Application Support/October Lantern/screenshots/screen-20260924-130501-1a2b.png","model":"gpt-6-sol","background":false}
{"type":"launch","requestId":"l2","kind":"claude","cwd":"/Users/me/Movies/video-3","prompt":"normalize the voice","context":"I'm working in DaVinci Resolve on this Mac. Its front window is \"video-3\".","toolkit":true,"background":false}
{"type":"models","kind":"codex"}
{"type":"toolkit.refresh"}
{"type":"harness.prepare","requestId":"p1"}
{"type":"october.session","accessToken":"<October Supabase access token>","expiresAt":1791234567}
{"type":"october.session"}
{"type":"attach","requestId":"a1","agentId":"codex:4242:1790180000"}
{"type":"history","requestId":"h1","agentId":"codex:4242:1790180000"}
```

```json
{"type":"keys","requestId":"k1","agentId":"claude:4242:1790180000","keys":["1"],"promptId":"9f2c41d07a3b5e18"}
{"type":"focus","requestId":"f1","agentId":"claude:4242:1790180000"}
```

```json
{"type":"october.pair"}
{"type":"october.cancelPair"}
{"type":"october.forget"}
```

`keys` presses single keys without Enter (`"1"`, `"Escape"`), e.g. to answer a permission prompt, after the same target check as `reply`; the result comes back as `replyResult`. For an agent at a permission prompt, `promptId` is required and must name the current prompt, both when the request arrives and again immediately before the key is pressed. `focus` brings the agent's own tab or pane to the front (for a tmux session nobody is attached to, it opens a Terminal window attached to it; for an agent inside a connected October Desktop, it shows the agent on the canvas); the result comes back as `attachResult`.

`launch` starts a new session. `timeoutMs` is the preparation deadline (default and maximum 45,000 ms): the engine checks it before creating a terminal/session. Once creation has begun, the app keeps the launch pending until the engine reports its outcome. Both app and engine refuse overlapping launches; a UI timeout does not mean it is safe to start another. With tmux, it runs on Lantern's tmux server (`-L lantern`), and `background: false` also opens a Terminal window attached to it. Without tmux, it runs directly in a new Terminal window, and `background: true` fails. Claude Code, Codex, Grok, Gemini CLI, Pi and October get `prompt` as the last command-line argument (after `--`, so a message starting with `-` isn't read as an option), and OpenCode as `--prompt`. Claude Code also gets a `--session-id` chosen by Lantern, so its conversation is matched exactly from the start. Other agents have it typed in once the normal scanner identifies the expected interactive harness as a descendant of the launched pane process and the delivery checks confirm that it owns the terminal foreground and its screen has stopped changing or it has been up 5 seconds, within 30 seconds; without tmux they can't be given a first message, and `launch` says so instead of dropping it. `bus: true` connects the agent to October Bus (Claude Code through `--mcp-config`, Codex through `-c` overrides, OpenCode through `OPENCODE_CONFIG`, GitHub Copilot through `--additional-mcp-config`, Goose through `--with-extension`, Gemini CLI and Qwen Code through a system-settings file of Lantern's that keeps any existing system settings, the October harness under `october-bus agent run`; Grok, Cursor and Pi start without it), links it with the other agents Lantern started in the `lantern` scope once it has registered, and adds a line about it to the first message; if the Bus can't be used, the agent starts without it and `launchResult` carries a `warning`. The first message is your `prompt`, then `context` (optional: what you're working in, sent by the app for a Task), then the screenshot's path, then the toolkit list when `toolkit` is true; with no prompt and no screenshot nothing is sent. The toolkit list is `toolkit.md` in Lantern's support folder: the engine rebuilds it at start when it's more than a day old, and `toolkit.refresh` rebuilds it now (answered with `{"type":"toolkit","ok":true,"path":"…"}`). `model` (optional) is passed as the agent's `--model` (Claude Code, Codex, Grok, OpenCode, Pi, October, Gemini CLI, Qwen Code, Aider, Cursor, Copilot); other agents refuse a model, and ids with spaces or shell characters are refused. `models` asks which models an agent offers; the engine answers `{"type":"models","kind":"codex","choosable":true,"models":[{"id":"gpt-6-sol","label":"GPT-6-Sol","group":null}]}`, listed by the agent itself where it can (`codex debug models`, `grok models`, `opencode models`, `--list-models` for Pi and October, cached ten minutes) and otherwise the short names the agent takes (Claude Code: fable, opus, sonnet, haiku; Gemini CLI: auto, pro, flash, flash-lite). `screenshot` (optional) is a PNG the app saved in `~/Library/Application Support/October Lantern/screenshots`; a path anywhere else is refused. Its path is added to the first message (a first message is made if there isn't one), Codex also gets it with `--image`, and Claude Code (`--add-dir`) and Gemini CLI (`--include-directories`) may read that folder without asking. `attach` opens a Terminal window attached to an agent's tmux session.

`kind: "october"` runs the user's own `october` when it's on their PATH, and otherwise Lantern's October Harness (see `harness`), by its full path with its Node.js `bin` folder first on `PATH`. If October Harness isn't installed yet, the launch installs it first, within `timeoutMs` (a first install usually takes longer, so the app sends `harness.prepare` ahead and launches once it's `ready`). October launches also get the October sign-in the app last sent with `october.session`, as October Desktop gives it: `OCTOBER_SUPABASE_URL`, `OCTOBER_SUPABASE_ANON_KEY`, `OCTOBER_SUPABASE_ACCESS_TOKEN` and `OCTOBER_SUPABASE_EXPIRES_AT` (unix seconds). The refresh token is never sent to the engine or the agent. Without a session (signed out, or the token has expired) October starts without it and asks you to `/login` itself. To keep a long session signed in, the harness asks `lantern-engine october-token` for a fresh token (`OCTOBER_TOKEN_COMMAND`); Lantern keeps the current access token for that in `~/Library/Application Support/October Lantern/october-token.json` (only you can read it; access token only, never the refresh token; deleted when you sign out).

`harness.prepare` installs October Harness in the background if `october` isn't on PATH and it isn't installed yet (answered with `harness`; an engine that already has it answers `ready` at once). `october.session` hands the engine the current October access token and its expiry, or no token after sign-out; the engine keeps only the latest, in memory. The app sends it after sign-in, on every refresh, after sign-out, and whenever a new engine process says `hello`.

`replyResult` may carry `delivery: "queued"` and `message: "Queued in October"` when October accepted the entire reply without confirming final typing. October replies over 8,000 Unicode scalar values fail with `message_too_long`; nothing is truncated and the draft remains.

`historyResult` may carry `busy: true` when the bounded reader is occupied; retry later without clearing the displayed conversation. Clients allow one outstanding history request per conversation and match the returned request id before applying it.

`reply` types the text into the agent's terminal (see `route`), then presses Enter. For agents with `route.via == "none"` it returns `not_reachable`, and the app falls back to copying the text and bringing the host app forward. `october.pair` asks October Desktop to allow Lantern (October shows a code to compare), `october.cancelPair` withdraws that, and `october.forget` drops the credential October issued (Lantern then goes back to listing October's terminals read-only).

## Hook events (written by `lantern-engine hook <source>`)

Hooks write one JSON file per session to `~/Library/Application Support/October Lantern/events/<source>-<session>.json`, replacing it atomically. The hook process reduces the harness's payload to Lantern's vocabulary before writing:

```json
{"source":"claude","state":"needs_input","sessionId":"...","cwd":"...","message":null,
 "question":"Permission to run Bash · rm -rf node_modules","questionKind":"permission",
 "questionDetail":"rm -rf node_modules","promptId":"9f2c41d07a3b5e18",
 "transcriptPath":"...","ancestors":[1234,1200],"at":1790181234567}
```

The engine matches a hook file to a running agent by `ancestors`, the process ids above the hook command (the agent is one of them). Claude Code hooks: `PermissionRequest` (the moment a permission prompt appears, with the tool and command), `Notification` (`permission_prompt`, `idle_prompt`, `elicitation_dialog`, `elicitation_url_dialog`, `agent_needs_input`; other notification types are ignored), `Stop` (with `last_assistant_message`), `UserPromptSubmit`, `PostToolUse` and `PostToolUseFailure`. Codex: the `notify` program (`agent-turn-complete`).

Installing and removing hooks takes a lock, backs each file up under a new name (`<file>.lantern-backup-<ms>[-n]`, never overwriting an earlier backup), writes through a temporary file of its own, and writes Claude's and Codex's files together: if the second write fails, the first gets its old contents back. `lantern-engine hooks status` prints `{"claude":bool,"claudeOutdated":bool,"codex":bool}`; `claudeOutdated` means an older Lantern's hooks that lack events this version needs.

## Phone (October phone app)

Lantern can act as an October host computer, so the October phone app pairs with it and reaches Lantern's agents through October's relay. The engine implements the same protocol as October Desktop:
- signed control calls to October's Supabase `mobile-*` functions
- relay tickets and the relay WebSocket (`wss://relay.afteroctober.xyz/v1/host/{hostId}`) with acknowledged outer frames
- a Noise_XX_25519_ChaChaPoly_BLAKE2b responder, with the phone's key pinned to October's records
- inner frames, and pairing with a 6-digit code

See `engine/src/mobile/`.

App → engine:
```json
{"type":"phone.token","accessToken":"<October Supabase access token>"}
{"type":"phone.pair"}
{"type":"phone.cancelPair"}
{"type":"phone.decide","allow":true}
{"type":"phone.revoke","bind":"<device bind uuid>"}
{"type":"phone.stop"}
```

`phone.token` carries the user's current October access token: the engine starts hosting for the phone app if it isn't already, or hands the running host the refreshed token. The app sends it after sign-in, on every token refresh, and again whenever a new engine process says `hello`, so an engine restart brings hosting back by itself. `phone.stop` (sign-out) ends hosting.

Engine → app: the whole phone state, sent whenever it changes.
```json
{"type":"phone","status":"offline|connecting|connected|plan-required|signed-out|error","message":null,
 "hostId":"…","devices":[{"bind":"…","label":"Harsh's iPhone","platform":"ios","pairedAt":1790000000000}],
 "pairing":{"qr":"https://october.dev/pair#…","expiresAt":1790000300000,"code":"123456","label":"Harsh's iPhone","finishing":false}}
```
`error` (with `hostId` null) means the host couldn't start, for example because `phone/host.json` is corrupt; `message` says why.

On the phone, Lantern appears as one canvas whose nodes are Lantern's agents. Lantern supports these requests:
- `core.handshake` (with the HMAC proof)
- `core.status`
- `bus.query` `listCanvases` / `currentSnapshot`
- `facts.query` `listNotifications` / `listNodeWorkflows` / `listPrObservations`
- `ui.list`, `terminal.list`, `agent.list`, `devServer.list`, `chat.history` (empty lists)
- `remote.push.register` (`{"token":"ExponentPushToken[…]"}` → `{"registered":true}`; anything that isn't an Expo push token gets `INVALID_ARGUMENT`), `remote.push.unregister` (`{}` → `{"unregistered":true}`) and `remote.push.mute` (`{"muted":bool}` → `{"muted":bool}`), which store the asking phone's push settings (a phone that isn't paired gets `UNAUTHENTICATED`)
- `bus.mutate userSend`, which types the reply into the agent's terminal through the same path as a reply from the app. It answers:
  - `{"accepted":true,"delivery":"delivered"}` only after the reply was typed;
  - `{"accepted":false,"reason":"…"}` when nothing was typed (including a reply that couldn't start in time, which is canceled and never typed later);
  - `{"accepted":true,"delivery":"queued","reason":"not-confirmed-check-the-terminal"}` when typing started but Lantern can't confirm it finished. `accepted:false` would invite a retry that could type it twice.

Checks on `userSend`:
- A request's `deadlineAt` (epoch ms; `0` or absent means none) must be a number. One that has passed is refused with `DEADLINE_EXCEEDED`.
- The optional fourth argument `expiresInMs` must be a number. The sooner of the two is the deadline for typing to start (at most 20 seconds).
- The node must be `{"id":…,"kind":"terminal"}` and the canvas Lantern's own, or the request is refused (`INVALID_ARGUMENT` / `NOT_FOUND`).

The answer to a request with an `idempotencyKey` is remembered per phone (the last 256, for as long as the engine runs). A retry of the same request gets the same answer, re-addressed to the retry's `requestId`, without typing again. The same key on a different request is refused with `INVALID_ARGUMENT`.

It emits the `bus.changed`, `facts.changed`, `core.lifecycle`, `cursor.reset` and `cursor.heartbeat` events. Anything else returns `PERMISSION_DENIED`.

`core.handshake` and `core.status` include `"capabilities":{"push":true,"chatHistory":false,"historyRoles":false,"permissionReplies":false}`. A Lantern without `capabilities.push` doesn't send notifications, so the phone keeps them off for it.

Push notifications: when an agent starts waiting for the person (finished its turn, or asking a question) and stays that way for 2 seconds, the engine posts one notification per phone that registered a token and isn't muted to Expo's push service (`https://exp.host/--/api/v2/push/send`), whether or not the relay is connected. It's sent once per attention episode: not again until the agent has been seen doing something else (a question after a finished turn is new attention), and never for attention that began before the engine started. The title names only the agent's handle (`@claude-1 needs you` / `@claude-1 finished`) with generic body text; no agent output is sent. `data` is `{"hostId","canvasId","nodeId","kind":"needs_input"|"done","notificationId"}`, which the phone opens as that agent; `notificationId` is the same id `listNotifications` gives that episode. A token Expo reports as `DeviceNotRegistered` is forgotten; revoking a phone forgets its token.

Identity and paired phones are stored in `~/Library/Application Support/October Lantern/phone/`, with the folder at 0700 and the files at 0600:
- `host.json` holds the hostId, the Ed25519 seed, the X25519 static key and the canvas id. A file with invalid keys is refused rather than replaced (October's servers pin the keys per host).
- `devices.json` holds only a SHA-256 hash of each phone's credential, plus the phone's Expo push token and mute switch once it turns notifications on.

Phone deliveries are polled on the host loop rather than waited on there (maximum 16 pending deliveries). Requests for other phones, pings and local stop/revoke commands remain available during a slow delivery. Cancel frames cancel the matching request on the current phone connection if typing has not begun. Disconnect/revoke/stop cancel outstanding unstarted operations. Retries sharing an in-flight idempotency key share the original operation and its eventual result, with up to eight response waiters; changing the request under that key is refused.

## Linux client and paths

`desktop/` is a native Rust UI that starts `lantern-engine serve` as a sidecar;
it uses protocol version 3 unchanged. Keep the UI and engine executable together,
or set `LANTERN_ENGINE` to an explicit engine path. On Linux, support files (events,
phone identity, toolkit, launch artifacts) live in
`${XDG_DATA_HOME:-~/.local/share}/october-lantern` rather than the macOS Library.
Process identity, controlling tty (`pts/N`), foreground process group and open
session files come from procfs. Direct replies use tmux with the existing safety
checks. Terminal launch/attach uses a supported Linux terminal emulator.

## Windows

The same `desktop/` and engine build for Windows. Support files live in
`%LOCALAPPDATA%\October Lantern`. Processes come from a Toolhelp snapshot; a process's
`tty` is `con:<pid>` of its console host (`conhost.exe`, or `OpenConsole.exe` behind a
Windows Terminal or VS Code tab). Windows has no foreground process group, so for an
agent in an ordinary terminal the delivery check can't confirm the agent is in front:
`reply` and `keys` are refused, `route` is `{"via":"none"}`, and `focus` brings the
`host` app's window to the front.

`launch` starts the agent in one of Lantern's own sessions instead of tmux: a detached
`lantern-engine pty-host <name>` runs it in a pseudo console (ConPTY) and records itself
in `pty/<name>.json` in the support folder. The agent runs directly, not through a shell
or `cmd.exe` (an npm `.cmd` shim is unwrapped to Node and its script), so the prompt is
passed as one argument, and the session ends when the agent exits. Its agents have
`route` `{"via":"pty","session":"<name>"}` and no `host`. Before typing, the engine asks
the host about its agent: the target must be the process the session started (or run
under it), with the same start time and program, alive, and on the session's console.
`focus`/`attach` bring a window showing the session forward or open a new one
(`lantern-engine attach <name>`, in Windows Terminal when installed). `background: false`
also opens that window when the session starts.

Sessions, their windows and October Bus run from copies in the support folder's `bin`
(`lantern-engine.exe`, the hooks' copy, and `october-bus-<commit>.exe`, copied from the
bundled `october-bus.exe`), so updating or reinstalling the app never replaces a running
program. `launch` with `bus: true` passes the Bus's arguments, environment and wrapper to the
agent as separate arguments, with no shell quoting.
