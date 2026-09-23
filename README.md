# October Lantern

October Lantern is a small floating bar for your desktop. It finds every coding agent running on your computer (Claude Code, Codex, OpenCode, Pi), shows which ones are waiting on you, and lets you answer them by voice or text without hunting for the right terminal window.

It's part of the October family, alongside October Desktop, `october-harness` and `october-bus`. Lantern is the lightweight entry point: a small download that works on its own. It becomes more capable when October Desktop is running.

## Why

People now run several agents at once, in different terminals, apps and tmux panes. Each one stops to ask something or to say it's done, and the only way to find out is to check every window. Lantern puts one light on the edge of the screen:

- **See every agent.** It lists every agent on the machine, whichever terminal or app launched it.
- **Know who's waiting.** The light turns amber and shows a count when agents are waiting on you.
- **Answer in place.** Read what the agent said and reply by typing or speaking, without leaving the app you're in.

## Status

Early development. v1 targets **macOS 14+** and runs standalone, with no connection to October Desktop.

| Area | v1 | Later |
|---|---|---|
| Detect agents: Claude Code, Codex, OpenCode, Pi | ✅ | More harnesses, via October |
| Status from the agents' own session files (working, or waiting on you) | ✅ | |
| Exact status from agent hooks (opt-in) | ✅ | |
| Inbox of agents waiting on you, with their last message | ✅ | |
| Reply to agents running in tmux | ✅ | |
| Reply to agents in other terminals | Copies the reply and focuses the terminal | Typing through Accessibility |
| Push-to-talk dictation (on-device) | ✅ | |
| Global hotkey (⌃⌥Space) | ✅ | |
| Connect to October Desktop (`october-core`) | | ✅ |
| Automatic routing ("send this to whichever agent it's for") | | ✅ |
| Windows | | ✅ |

## How it works

```
┌──────────────────────────┐   JSON lines on stdin/stdout   ┌───────────────────────────┐
│  macos/  OctoberLantern  │ ◄────────────────────────────► │  engine/  lantern-engine  │
│  Swift/AppKit: the pill, │  snapshots of agents, replies  │  Rust: find agents, read  │
│  panels, voice, hotkey   │                                │  session files and hooks, │
└──────────────────────────┘                                │  deliver replies (tmux)   │
                                                            └───────────────────────────┘
```

- **The engine** (`engine/`) is a small Rust program. The app starts it and talks to it over stdin/stdout. Every couple of seconds it scans running processes, reads each agent's session file, and sends the app a snapshot. The protocol is in [`protocol/README.md`](protocol/README.md). The engine has no UI, so a future Windows front end can reuse it as is.
- **The macOS app** (`macos/`) is a native Swift/AppKit app. The pill is a non-activating panel, so clicking it never takes focus away from the app you're in. It stays on every Space and over full-screen apps.
- **How it knows an agent is waiting:** by default Lantern reads the agent's own session files: Claude Code transcripts under `~/.claude/projects` and Codex logs under `~/.codex/sessions`. For exact status, including permission prompts, run `lantern-engine hooks install`. That adds Lantern to Claude Code's hooks and Codex's `notify` setting, after backing up both config files. Lantern only reads the agents' files. It never changes them, apart from that opt-in hook install.
- **Privacy:** everything stays on your Mac. Dictation uses Apple's on-device speech recognition. Nothing is uploaded.

## Repository layout

| Path | What |
|---|---|
| `engine/` | Rust engine: agent detection, session-file readers, hooks, tmux delivery |
| `macos/` | Swift package for the macOS app |
| `protocol/` | Message protocol between the app and the engine |
| `scripts/` | Build and packaging scripts |
| `windows/` | Not started |

## Building and running (macOS)

Requires Xcode 16+ and a Rust toolchain (`brew install rustup && rustup default stable`).

```sh
scripts/build-app.sh          # builds the engine and the app, then assembles build/October Lantern.app
open "build/October Lantern.app"
```

While developing, `"build/October Lantern.app/Contents/MacOS/OctoberLantern" --open inbox` (or `agents`) launches with the panel open.

To use the engine on its own:

```sh
cd engine
cargo run -- agents           # print the agents Lantern can see, as JSON
cargo run -- serve            # the stdio protocol the app uses
cargo run -- hooks status     # show whether the optional hooks are installed
```

## Permissions

| Permission | Why | When it's requested |
|---|---|---|
| Microphone | Push-to-talk dictation | The first time you press the mic button |
| Speech Recognition | Turning speech into text, on-device | Same |
| Accessibility | Typing replies into terminals other than tmux | Not needed in v1 |

## Relationship to October Desktop

Lantern runs on its own. A later version will look for a running `october-core` and connect to it after a one-time approval in October. When connected, October's agent list and message delivery become the source of truth, and Lantern gains "Open on canvas". Until then, Lantern requires no changes to October Desktop.
