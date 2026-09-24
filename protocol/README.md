# Lantern engine protocol (v2)

The app starts `lantern-engine serve`. The two sides exchange one JSON object per line. The engine writes to stdout and reads from stdin. Anything the engine writes to stderr is a log.

## Engine → app

### `hello`
Sent once at startup. The app only talks to an engine whose `protocol` matches its own; otherwise it stops the engine and asks the user to reinstall.
```json
{"type":"hello","protocol":2,"version":"0.3.0"}
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
| `kind` | string | `claude`, `codex`, `opencode`, `pi`, `october` (the October harness), `gemini`, `grok`, `cursor`, `qwen`, `goose`, `aider`, `amp`, `copilot`, `kimi`, `droid`, `crush`, `auggie`. Clients should accept unknown values. |
| `handle` | string | Display handle, e.g. `"claude-2"`. Numbered per kind. A number stays with its agent for the agent's whole life, and a new agent takes the lowest free number. |
| `pid` | number | |
| `startTime` | number | Seconds since the epoch when the process started. Delivery checks the process still has it. |
| `tty` | string? | e.g. `"ttys004"` |
| `cwd` | string? | Current working directory |
| `project` | string? | Last path component of `cwd` |
| `title` | string? | Session title (Claude's `ai-title`, or the first prompt in a Codex session) |
| `sessionId` | string? | |
| `state` | `"working" \| "waiting" \| "needs_input" \| "idle" \| "unknown"` | `waiting` means the agent finished its turn and it's your turn. `needs_input` means it's blocked on a question or permission (known only from hooks). `unknown` means the session file didn't say (for Codex, a turn longer than the part of the file the engine reads). |
| `stateSince` | number? | Epoch ms of the event that produced `state`: the session file's own timestamp for that event, or the hook event's time. Only falls back to the file's modification time when the file has no timestamps. |
| `lastMessage` | string? | The agent's last message to you (truncated to 2,000 characters) |
| `question` | string? | For `needs_input`: what it's asking |
| `questionKind` | `"permission" \| "other"`? | For `needs_input` from a hook: `permission` is a tool permission prompt (`1` allows once, Escape declines, with the tool and command in `question`); `other` is anything else, to be answered in the terminal. |
| `host` | `{ "app": string, "pid": number, "bundlePath": string }?` | The GUI app the agent runs inside (Terminal, iTerm2, Ghostty, cmux, ...) |
| `tmux` | `{ "socket": string?, "target": string, "paneId": string }?` | Present when the agent runs inside a tmux pane |
| `canReply` | bool | `true` when the engine can type a reply into the agent (`route.via` isn't `none`) |
| `route` | object | How replies reach the agent. `{"via":"tmux"}`, `{"via":"cmux","workspace":"…","surface":"…"}`, `{"via":"terminal","tty":"/dev/ttys004"}`, `{"via":"iterm","tty":"…"}`, `{"via":"october","canvasId":"…","nodeId":"…"}` (through October Desktop, when Lantern is connected to it) or `{"via":"none"}`. Single keys (`keys`) work for tmux, cmux and iterm. |
| `stateSource` | `"hook" \| "transcript" \| "none"` | Where the state came from ("transcript" covers every session reader) |

### `installed`
Sent once, shortly after `hello`. Lists the agents installed on this machine, as found by the user's login shell.
```json
{"type":"installed","installed":{"kinds":["claude","codex","opencode"],"tmux":true}}
```

### `launchResult` / `attachResult`
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
`error` is `unknown_agent`, `not_reachable` or `send_failed`. A reply is only `ok` after the text and Enter reached the terminal; before typing, the engine checks that the agent process is alive with the same start time, still on the same tty, and in the foreground of that tty, so a reply can't land in the shell an exited agent left behind (`send_failed` with the reason).

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
{"type":"launch","requestId":"l1","kind":"codex","cwd":"/Users/me/proj","prompt":"write tests","background":false}
{"type":"attach","requestId":"a1","agentId":"codex:4242:1790180000"}
{"type":"history","requestId":"h1","agentId":"codex:4242:1790180000"}
```

```json
{"type":"keys","requestId":"k1","agentId":"claude:4242:1790180000","keys":["1"]}
{"type":"focus","requestId":"f1","agentId":"claude:4242:1790180000"}
```

```json
{"type":"october.pair"}
{"type":"october.cancelPair"}
{"type":"october.forget"}
```

`keys` presses single keys without Enter (`"1"`, `"Escape"`), e.g. to answer a permission prompt, after the same target check as `reply`; the result comes back as `replyResult`. `focus` brings the agent's own tab or pane to the front (for a tmux session nobody is attached to, it opens a Terminal window attached to it; for an agent inside a connected October Desktop, it shows the agent on the canvas); the result comes back as `attachResult`.

`launch` starts a new session. With tmux, it runs on Lantern's tmux server (`-L lantern`), and `background: false` also opens a Terminal window attached to it. Without tmux, it runs directly in a new Terminal window, and `background: true` fails. Claude Code and Codex get `prompt` as a command-line argument; other agents have it typed in about 4 seconds after they start (see AUDIT.md, F18). `attach` opens a Terminal window attached to an agent's tmux session.

`reply` types the text into the agent's terminal (see `route`), then presses Enter. For agents with `route.via == "none"` it returns `not_reachable`, and the app falls back to copying the text and bringing the host app forward. `october.pair` asks October Desktop to allow Lantern (October shows a code to compare), `october.cancelPair` withdraws that, and `october.forget` drops the credential October issued (Lantern then goes back to listing October's terminals read-only).

## Hook events (written by `lantern-engine hook <source>`)

Hooks write one JSON file per session to `~/Library/Application Support/October Lantern/events/<source>-<session>.json`, replacing it atomically. The hook process reduces the harness's payload to Lantern's vocabulary before writing:

```json
{"source":"claude","state":"needs_input","sessionId":"...","cwd":"...","message":null,
 "question":"Permission to run Bash · rm -rf node_modules","questionKind":"permission",
 "transcriptPath":"...","ancestors":[1234,1200],"at":1790181234567}
```

The engine matches a hook file to a running agent by `ancestors`, the process ids above the hook command (the agent is one of them). Claude Code hooks: `PermissionRequest` (the moment a permission prompt appears, with the tool and command), `Notification` (`permission_prompt`, `idle_prompt`, `elicitation_dialog`, `elicitation_url_dialog`, `agent_needs_input`; other notification types are ignored), `Stop` (with `last_assistant_message`), `UserPromptSubmit` and `PostToolUse`. Codex: the `notify` program (`agent-turn-complete`).

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
- `bus.mutate userSend`, which types the reply into the agent's terminal through the same path as a reply from the app, and answers `{"accepted":true,"delivery":"delivered"}` only after it did; otherwise `{"accepted":false,"reason":"…"}`

A request whose `deadlineAt` has passed is refused with `DEADLINE_EXCEEDED`; a `userSend` for another canvas with `NOT_FOUND`. The answer to a request with an `idempotencyKey` is remembered per phone, so a retry gets the same answer without typing again.

It emits the `bus.changed`, `facts.changed`, `core.lifecycle`, `cursor.reset` and `cursor.heartbeat` events. Anything else returns `PERMISSION_DENIED`.

Identity and paired phones are stored in `~/Library/Application Support/October Lantern/phone/`, with the folder at 0700 and the files at 0600:
- `host.json` holds the hostId, the Ed25519 seed, the X25519 static key and the canvas id. A file with invalid keys is refused rather than replaced (October's servers pin the keys per host).
- `devices.json` holds only a SHA-256 hash of each phone's credential.
