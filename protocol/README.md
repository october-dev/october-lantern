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
| `kind` | string | `claude`, `codex`, `opencode`, `pi`, `gemini`, `grok`, `cursor`, `qwen`, `goose`, `aider`, `amp`, `copilot`, `kimi`, `droid`, `crush`, `auggie`. Clients should accept unknown values. |
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
| `canReply` | bool | `true` when the engine can type a reply into the agent directly (tmux) |
| `stateSource` | `"hook" \| "transcript" \| "none"` | Where the state came from |

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

`reply` types the text into the agent's tmux pane, then presses Enter. For agents outside tmux it returns `not_reachable`, and the app falls back to copying the text and focusing the host app.

## Hook events (written by `lantern-engine hook <source>`)

Hooks write one JSON file per session to `~/Library/Application Support/October Lantern/events/<source>-<session>.json`, replacing it atomically:

```json
{"source":"claude","event":"Notification","sessionId":"...","cwd":"...","message":"...","transcriptPath":"...","ancestors":[1234,1200],"at":1790181234567}
```

The engine matches a hook file to a running agent by `sessionId`, falling back to `ancestors`, which contains the process ids above the hook command.
