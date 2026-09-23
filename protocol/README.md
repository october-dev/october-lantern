# Lantern engine protocol (v1)

The app starts `lantern-engine serve`. The two sides exchange one JSON object per line. The engine writes to stdout and reads from stdin. Anything the engine writes to stderr is a log.

## Engine → app

### `hello`
Sent once at startup.
```json
{"type":"hello","protocol":1,"version":"0.1.0"}
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
| `id` | string | Stable while the process lives: `"<kind>:<pid>"` |
| `kind` | string | `claude`, `codex`, `opencode`, `pi`, `october` (the October harness), `gemini`, `grok`, `cursor`, `qwen`, `goose`, `aider`, `amp`, `copilot`, `kimi`, `droid`, `crush`, `auggie`. Clients should accept unknown values. |
| `handle` | string | Display handle, e.g. `"claude-2"`. Numbered per kind. A number stays with its agent for the agent's whole life, and a new agent takes the lowest free number. |
| `pid` | number | |
| `tty` | string? | e.g. `"ttys004"` |
| `cwd` | string? | Current working directory |
| `project` | string? | Last path component of `cwd` |
| `title` | string? | Session title (Claude's `ai-title`, or the first prompt in a Codex session) |
| `sessionId` | string? | |
| `state` | `"working" \| "waiting" \| "needs_input" \| "idle" \| "unknown"` | `waiting` means the agent finished its turn and it's your turn. `needs_input` means it's blocked on a question or permission (known only from hooks). |
| `stateSince` | number? | Epoch ms of the last state change, if known |
| `lastMessage` | string? | The agent's last message to you (truncated to 2,000 characters) |
| `question` | string? | For `needs_input`: what it's asking |
| `host` | `{ "app": string, "pid": number, "bundlePath": string }?` | The GUI app the agent runs inside (Terminal, iTerm2, Ghostty, cmux, ...) |
| `tmux` | `{ "socket": string?, "target": string, "paneId": string }?` | Present when the agent runs inside a tmux pane |
| `canReply` | bool | `true` when the engine can type a reply into the agent (`route.via` isn't `none`) |
| `route` | object | How replies reach the agent. `{"via":"tmux"}`, `{"via":"cmux","workspace":"…","surface":"…"}`, `{"via":"terminal","tty":"/dev/ttys004"}`, `{"via":"iterm","tty":"…"}` or `{"via":"none"}`. Single keys (`keys`) work for tmux, cmux and iterm. |
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
{"type":"historyResult","requestId":"h1","agentId":"codex:4242","supported":true,
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

## App → engine

```json
{"type":"refresh"}
{"type":"reply","requestId":"r1","agentId":"claude:83630","text":"Use Postgres"}
```

```json
{"type":"launch","requestId":"l1","kind":"codex","cwd":"/Users/me/proj","prompt":"write tests","background":false}
{"type":"attach","requestId":"a1","agentId":"codex:4242"}
{"type":"history","requestId":"h1","agentId":"codex:4242"}
```

```json
{"type":"keys","requestId":"k1","agentId":"claude:4242","keys":["1"]}
{"type":"focus","requestId":"f1","agentId":"claude:4242"}
```

`keys` presses single keys without Enter (`"1"`, `"Escape"`), e.g. to answer a permission prompt; the result comes back as `replyResult`. `focus` brings the agent's own tab or pane to the front (for a tmux session nobody is attached to, it opens a Terminal window attached to it); the result comes back as `attachResult`.

`launch` starts a new session. With tmux, it runs on Lantern's tmux server (`-L lantern`), and `background: false` also opens a Terminal window attached to it. Without tmux, it runs directly in a new Terminal window, and `background: true` fails. Claude Code and Codex get `prompt` as a command-line argument; other agents have it typed in about 4 seconds after they start. `attach` opens a Terminal window attached to an agent's tmux session.

`reply` types the text into the agent's terminal (see `route`), then presses Enter. For agents with `route.via == "none"` it returns `not_reachable`, and the app falls back to copying the text and bringing the host app forward.

## Hook events (written by `lantern-engine hook <source>`)

Hooks write one JSON file per session to `~/Library/Application Support/October Lantern/events/<source>-<session>.json`, replacing it atomically:

```json
{"source":"claude","event":"Notification","sessionId":"...","cwd":"...","message":"...","transcriptPath":"...","ancestors":[1234,1200],"at":1790181234567}
```

The engine matches a hook file to a running agent by `sessionId`, falling back to `ancestors`, which contains the process ids above the hook command.

## Phone (October phone app)

Lantern can act as an October host computer, so the October phone app pairs with it and reaches Lantern's agents through October's relay. The engine implements the same protocol as October Desktop:
- signed control calls to October's Supabase `mobile-*` functions
- relay tickets and the relay WebSocket (`wss://relay.afteroctober.xyz/v1/host/{hostId}`) with acknowledged outer frames
- a Noise_XX_25519_ChaChaPoly_BLAKE2b responder, with the phone's key pinned to October's records
- inner frames, and pairing with a 6-digit code

See `engine/src/mobile/`.

App → engine:
```json
{"type":"phone.start","accessToken":"<October Supabase access token>"}
{"type":"phone.token","accessToken":"<refreshed token>"}
{"type":"phone.pair"}
{"type":"phone.decide","allow":true}
{"type":"phone.revoke","bind":"<device bind uuid>"}
{"type":"phone.stop"}
```

Engine → app: the whole phone state, sent whenever it changes.
```json
{"type":"phone","status":"offline|connecting|connected|plan-required|signed-out","message":null,
 "hostId":"…","devices":[{"bind":"…","label":"Harsh's iPhone","platform":"ios","pairedAt":1790000000000}],
 "pairing":{"qr":"https://october.dev/pair#…","expiresAt":1790000300000,"code":"123456","label":"Harsh's iPhone","finishing":false}}
```

On the phone, Lantern appears as one canvas whose nodes are Lantern's agents. Lantern supports these requests:
- `core.handshake` (with the HMAC proof)
- `core.status`
- `bus.query` `listCanvases` / `currentSnapshot`
- `facts.query` `listNotifications` / `listNodeWorkflows` / `listPrObservations`
- `ui.list`, `terminal.list`, `agent.list`, `devServer.list`, `chat.history` (empty lists)
- `bus.mutate userSend`, which types the reply into the agent's terminal

It emits the `bus.changed`, `facts.changed`, `core.lifecycle`, `cursor.reset` and `cursor.heartbeat` events. Anything else returns `PERMISSION_DENIED`.

Identity and paired phones are stored in `~/Library/Application Support/October Lantern/phone/`, with the folder at 0700 and the files at 0600:
- `host.json` holds the hostId, the Ed25519 seed, the X25519 static key and the canvas id.
- `devices.json` holds only a SHA-256 hash of each phone's credential.
