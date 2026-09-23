# October Lantern

**A small glowing lantern on the edge of your screen that watches every AI coding agent on your Mac and lights up when one of them needs you.**

People now run several coding agents at once (Claude Code in one terminal, Codex in another, Pi or OpenCode in a third), and each one keeps stopping to say "done" or to ask something. The only way to find out is to click through every window. October Lantern sits on the edge of your screen, finds all of those agents by itself, shows which ones are waiting on you and what they said, and lets you answer without hunting for the right window.

Lantern is part of the **October** family. October Desktop is the flagship workspace for running teams of agents; `october-harness` and `october-bus` are its agent and messaging layers. Lantern is the small, free-standing entry point: a light download that's useful on its own, and gets more capable when October Desktop is installed (see [Roadmap](#roadmap)).

> **For anyone writing about Lantern (website, launch posts, docs):** this README is the source of truth. The [What's true today](#whats-true-today) table says exactly what works now and what doesn't. Please don't claim anything marked "not yet".

---

## Contents

- [The problem](#the-problem)
- [What Lantern does](#what-lantern-does)
- [What it looks like](#what-it-looks-like)
- [A walkthrough](#a-walkthrough)
- [Supported agents](#supported-agents)
- [What's true today](#whats-true-today)
- [Privacy](#privacy)
- [Requirements and permissions](#requirements-and-permissions)
- [Roadmap](#roadmap)
- [Brand and voice](#brand-and-voice)
- [FAQ](#faq)
- [How it works (technical)](#how-it-works-technical)
- [Building from source](#building-from-source)
- [Repository layout](#repository-layout)
- [Glossary](#glossary)

---

## The problem

AI coding agents are good enough now that developers run several of them in parallel. That creates a new chore: **babysitting**.

- Agents finish or get stuck at unpredictable times, and nothing tells you which one.
- Each agent lives in its own terminal window, tab or tmux pane, so checking on them means clicking through all of them.
- Every check pulls you out of what you were doing.

The result is either wasted agent time (an agent sat idle for 20 minutes waiting for a "yes") or wasted human attention (you keep checking agents that were still busy).

## What Lantern does

1. **Finds every agent on your Mac, automatically.** No setup and no special way of launching them. If Claude Code, Codex, Pi, OpenCode or another supported agent is running in any terminal (Terminal, iTerm2, Ghostty, cmux, VS Code's terminal, tmux...), Lantern sees it.
2. **Knows which ones are waiting on you.** For Claude Code and Codex, Lantern reads each agent's own session log to tell whether it's still working or has finished its turn and is waiting for you. It shows what the agent last said.
3. **One place to look.** The lantern glows amber and shows a count when agents are waiting. Click it to see a "Waiting" list: each agent, its project, how long it's been waiting, and its last message.
4. **Answer without switching windows.** Type or dictate a reply in Lantern. For agents running inside tmux, Lantern types the reply straight into the agent's session. For agents in other terminals, Lantern copies your reply and brings the right app to the front so you can paste it (see [What's true today](#whats-true-today)).
5. **Stays out of your way.** Collapsed, Lantern is a single small lantern icon on the edge of the screen. It never takes keyboard focus from the app you're using, and it appears on every desktop Space and over full-screen apps.

Lantern does **not** replace your terminals. Your agents keep running exactly where you started them, and you can keep using them there as usual. Lantern is an extra view and remote control on top.

## What it looks like

**The lantern (collapsed).** A small glass circle on the right (or left) edge of the screen containing the Lantern logo, a little orange lantern with eyes.
- **Dim**: no agent needs anything.
- **Soft glow**: agents are working.
- **Bright amber glow with a number badge**: that many agents are waiting on you.

**Expanded.** Hover over the lantern and it grows downward into a glass capsule with five buttons: **Waiting** (inbox), **Agents** (everyone running), **Message** (compose), **Dictate** (microphone) and **More** (settings). Move the mouse away and it tucks back into the single lantern. Drag the lantern to move it; it snaps to the nearest screen edge.

**The panel.** Clicking Waiting or Agents opens a frosted-glass panel beside the lantern:
- **Waiting**: one card per agent that's waiting on you, with the agent's logo, its handle (e.g. `@claude-2`), its project folder, the app it's running in, how long it's been waiting, the session title, and its last message. Each card has **Reply**, **Open** (bring its terminal to the front) and **Done** (clear it from the list) buttons.
- **Agents**: every running agent, sorted with the ones that need you first, each with its logo, handle, folder, app, and a status of *Your turn*, *Working* or *Running*.
- **Composer** at the bottom: "To @claude-2", a message field, a mic button and a send button.

**Menu bar.** A small lantern icon in the menu bar with Waiting, Agents, Message an Agent, Show/Hide Lantern, Open at Login, agent hooks, and Quit.

**Look and feel.** Dark, translucent glass (Apple's Liquid Glass on macOS 26, frosted blur on earlier versions) with a light rim along the edges. Amber means "needs you", green means "working" and grey means "idle". Each agent is shown with its official logo.

## A walkthrough

Say you have six sessions running: two Claude Code, two Codex, one Pi and one OpenCode, spread across a couple of terminal apps.

1. **Open Lantern.** All six appear within a couple of seconds, each with its logo and a handle: `@claude-1`, `@claude-2`, `@codex-1`, `@codex-2`, `@pi-1`, `@opencode-1`. You didn't have to register or restart anything.
2. **Keep working.** While the agents run, the lantern glows softly. Hover over it and open **Agents** to see each one's folder and status.
3. **An agent finishes.** When `@codex-1` finishes its task, the lantern turns amber with a **1**. Click it: the Waiting card shows what Codex said, e.g. *"Tests pass. Want me to commit?"*
4. **Reply.** Click **Reply**, type or dictate "yes, commit it", and press Enter.
   - If `@codex-1` runs inside **tmux**, the reply is typed straight into its session, just as if you'd typed it in the terminal.
   - Otherwise, Lantern copies the reply and brings that terminal app to the front, and you paste with ⌘V.
5. **Or reply the old way.** You can always switch to the terminal and answer there. Lantern updates by itself either way.
6. **Pi and OpenCode.** These show up in Agents as **Running**, but Lantern can't yet tell whether they're waiting on you, so they don't appear in Waiting.

## Supported agents

| Agent | Detected | Knows "working" vs "your turn" | Shows last message | Notes |
|---|---|---|---|---|
| Claude Code | ✅ | ✅ | ✅ | Optional hooks add "needs permission" alerts |
| Codex CLI | ✅ | ✅ | ✅ | Optional `notify` hook for instant updates |
| OpenCode | ✅ | — | — | Shows as Running |
| Pi | ✅ | — | — | Shows as Running |
| Gemini CLI, Grok, Cursor Agent, Qwen Code, Goose, Aider, Amp, GitHub Copilot CLI, Kimi, Droid, Crush, Auggie | ✅ | — | — | Shows as Running |

"Detected" means Lantern finds the running process and shows it with its logo, folder and host app. Agents running headless (for example `claude -p` or `codex exec` in a script) are deliberately ignored, because there's no one to answer them.

## What's true today

Lantern is in **early development (v0.1)**. Nothing has been publicly released yet.

| Capability | Status |
|---|---|
| macOS app (macOS 14+, Apple silicon) | ✅ Works. Intel builds aren't set up yet. |
| Finds agents automatically in any terminal app | ✅ Works |
| Working / your-turn status and last message for Claude Code and Codex | ✅ Works |
| Status for OpenCode, Pi and the others | ❌ Not yet (shown as "Running") |
| "Needs permission" alerts | ✅ With the optional hooks (Claude Code) |
| Reply to agents running in tmux | ✅ Types the reply into the session |
| Reply to agents in other terminals (Terminal, iTerm2, Ghostty, cmux...) | ⚠️ Copies the reply and focuses the app; you paste. Direct typing is planned. |
| Jump to the exact terminal tab | ❌ Not yet. "Open" brings the right app forward, not the exact tab. |
| Dictation (speech to text), on-device | ✅ Works (Apple Speech) |
| Global shortcut ⌃⌥Space to open the composer | ✅ Works |
| Open at login | ✅ Works (menu bar menu) |
| "Say it once and Lantern picks the right agent" (automatic routing) | ❌ Not yet. You choose the agent. |
| Screenshot or screen context | ❌ Not yet |
| Phone / remote control | ❌ Not yet |
| Connects to October Desktop | ❌ Not yet (planned) |
| Windows or Linux | ❌ Not yet |
| Pricing | Not decided. There is no account, sign-in or payment today. |

## Privacy

- **Everything stays on your Mac.** Lantern has no server and no account, and sends nothing over the network.
- **Dictation is on-device** where the Mac supports it (Apple's speech recognizer). Lantern only listens while the mic button is on.
- **No screen recording.** Lantern doesn't take screenshots or read other apps' windows.
- **Read-only by default.** Lantern reads the process list and the agents' own session files (`~/.claude/projects`, `~/.codex/sessions`). It never modifies them.
- **One opt-in exception: hooks.** If you turn on "Exact Status with Agent Hooks", Lantern adds itself to Claude Code's hooks (`~/.claude/settings.json`) and Codex's `notify` setting (`~/.codex/config.toml`). It backs up both files first, keeps any existing Codex notify program working, and can remove itself again from the same menu.

## Requirements and permissions

- macOS 14 Sonoma or later (Liquid Glass look on macOS 26 Tahoe)
- At least one supported agent

| Permission | Why | When it's asked |
|---|---|---|
| Microphone | Dictating a reply | The first time you press the mic button |
| Speech Recognition | Turning speech into text, on-device | Same |

Lantern needs no Accessibility, Screen Recording or Full Disk Access permission today.

## Roadmap

In rough order:

1. **Direct replies everywhere.** Type into agents in any terminal, not only tmux (through each terminal's own tools where they exist, or Accessibility).
2. **Status for more agents.** Session-file readers or hooks for OpenCode, Pi, Gemini and others, so they show "your turn" too.
3. **Jump to the exact tab or pane** of an agent.
4. **Connect to October Desktop.** When October is running, Lantern will connect to it (after a one-time approval in October) and gain October's full agent list (30+ agents), reliable message delivery, and "Open on canvas". Lantern keeps working on its own without October.
5. **Automatic routing.** Say "tell the backend agent to use Postgres" and Lantern picks the agent.
6. **Windows.**

## Brand and voice

- **Name:** October Lantern. "Lantern" for short. The app's menu bar item and window titles say "October Lantern".
- **Logo:** `logo.png`, a glossy orange paper-lantern character with a handle and two eyes, glowing warm from inside. The app icon puts it on a dark rounded tile (`macos/Resources/AppIcon.icns`).
- **Idea behind the name:** a lantern is a small light you keep nearby that shows you where to look. It lights up when something needs you.
- **Colours used in the app:** amber `#FAB845` means needs you or your turn; green `#5CCC82` means working; red `#ED5C5C` means recording; the surfaces are dark translucent glass with white text.
- **Tone:** calm, plain and honest. Lantern exists to *reduce* noise, so the product and its copy should never feel shouty.
- **One-line descriptions** (suggestions):
  - "Every coding agent on your Mac, in one small light."
  - "Know when your agents need you, and answer without switching windows."
  - "A lantern for your agents: it lights up when one needs you."

## FAQ

**Do I have to launch my agents in a special way?**
No. Lantern finds agents however you started them, in whatever terminal.

**Does it replace my terminal or October Desktop?**
No. Your agents keep running where they are, and you can keep using them there. Lantern is a lightweight view and remote on top. October Desktop is the full workspace for building with teams of agents.

**Does it send my code or conversations anywhere?**
No. Everything runs locally and nothing is uploaded.

**Can it answer permission prompts ("Allow this command?")?**
It can *tell* you about them (with hooks on, for Claude Code). Answering them from Lantern works for agents in tmux, where your reply is typed into the session. Elsewhere, click **Open** and answer in the terminal.

**Why does an agent say "Running" instead of "Your turn"?**
Lantern can only read the status of Claude Code and Codex so far. The others will follow.

**Is it free?**
There's no pricing yet. Today it's a local app with no account.

---

## How it works (technical)

```
┌──────────────────────────┐   JSON lines on stdin/stdout   ┌───────────────────────────┐
│  macos/  OctoberLantern  │ ◄────────────────────────────► │  engine/  lantern-engine  │
│  Swift/AppKit/SwiftUI:   │  snapshots of agents, replies  │  Rust: find agents, read  │
│  lantern, panels, voice, │                                │  session files and hooks, │
│  hotkey, menu bar        │                                │  deliver replies (tmux)   │
└──────────────────────────┘                                └───────────────────────────┘
```

- **Engine (`engine/`, Rust).** A small helper the app starts and talks to over stdin/stdout ([protocol](protocol/README.md)). About every 1.5 seconds it:
  1. reads the process table (`ps` plus `sysinfo`) and picks out agent processes by their executable name (including `node`/`bun`/`python` wrappers), keeping only interactive ones with a terminal;
  2. walks up each process's parents to find the app it runs in (e.g. cmux, Terminal), and matches its terminal against every running tmux server's panes;
  3. reads the tail of each agent's session file (a Claude Code transcript found by session id or folder; a Codex rollout file found through the process's open files) to decide *working* vs *your turn* and extract the last message and session title;
  4. merges any hook events (written by `lantern-engine hook ...`) and uses whichever is newer;
  5. sends the app a snapshot whenever something changed.

  Replies to tmux agents are delivered with `tmux send-keys`. The engine has no UI, so a Windows front end can reuse it unchanged.
- **App (`macos/`, Swift).** The lantern and the panel are borderless, non-activating `NSPanel`s, so clicking them never steals focus. They float above other windows, and appear on every Space and over full-screen apps. Surfaces use `NSGlassEffectView` (macOS 26) or `NSVisualEffectView`. Hover expansion works by polling the mouse position. Dictation uses `SFSpeechRecognizer` with on-device recognition. The ⌃⌥Space shortcut uses Carbon `RegisterEventHotKey`, which needs no Accessibility permission.

## Building from source

Requires Xcode 16+ (Xcode 26 for the Liquid Glass look) and Rust (`brew install rustup && rustup default stable`).

```sh
scripts/build-app.sh          # builds engine + app → build/October Lantern.app
open "build/October Lantern.app"
```

- `scripts/build-app.sh --debug` makes a debug build.
- `swift scripts/make-icon.swift` regenerates the app icon from `logo.png`.
- `"build/October Lantern.app/Contents/MacOS/OctoberLantern" --open inbox` (or `agents`) launches with the panel open (for development).

To run the engine on its own:

```sh
cd engine
cargo run -- agents           # print the agents Lantern can see, as JSON
cargo run -- serve            # the stdio protocol the app uses
cargo run -- hooks status     # show whether the optional hooks are installed
cargo run -- hooks install    # add the hooks (backs up config files first)
cargo run -- hooks uninstall  # remove them and restore previous settings
```

## Repository layout

| Path | What |
|---|---|
| `engine/` | Rust engine: agent detection, session-file readers, hooks, tmux delivery |
| `macos/` | Swift package for the macOS app. `Resources/` holds the app icon and harness logos. |
| `protocol/` | The message protocol between the app and the engine |
| `scripts/` | Build, packaging and icon scripts |
| `logo.png` | The Lantern logo (source for the app, menu bar and app icons) |

Harness logos come from October Desktop. Some come from Agent Orchestrator under Apache-2.0; see `macos/Resources/harness/NOTICE.md`. All names and logos are trademarks of their owners.

## Glossary

| Term | Meaning |
|---|---|
| **Agent** | An AI coding tool running in a terminal: Claude Code, Codex, Pi, OpenCode and so on |
| **Harness** | The October family's word for an agent's program (Claude Code is a harness) |
| **Session** | One running agent conversation in one terminal |
| **Handle** | Lantern's short name for a session, e.g. `@claude-2`. It stays the same while the session runs. |
| **Your turn / waiting** | The agent finished and is waiting for you |
| **Needs you** | The agent is blocked on a question or permission prompt (known with hooks) |
| **Hooks** | An optional setting that makes Claude Code and Codex notify Lantern directly |
| **tmux** | A terminal multiplexer; agents inside it can receive replies directly from Lantern |
