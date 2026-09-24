# Lantern product and development guide

**A small glowing lantern on the edge of your screen that watches every AI coding agent on your Mac and lights up when one of them needs you.**

People now run several coding agents at once (Claude Code in one terminal, Codex in another, Pi or OpenCode in a third), and each one keeps stopping to say "done" or to ask something. The only way to find out is to click through every window. October Lantern sits on the edge of your screen, finds all of those agents by itself, shows which ones are waiting on you and what they said, and lets you answer them without hunting for the right window.

Lantern is part of the **October** family. October Desktop is the flagship workspace for running teams of agents; `october-harness` and `october-bus` are its agent and messaging layers. Lantern is the small, free-standing entry point: a light download that's useful on its own, and will get more capable when October Desktop is installed (see [Roadmap](#roadmap)).

- **Website:** <https://lantern.october.dev>
- **Download (always the latest):** <https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern.dmg>
- **Contact:** hey@october.dev

> **For anyone writing about Lantern (website, launch posts, docs):** this guide is the detailed source of truth. The [What's true today](#whats-true-today) table says exactly what works now and what doesn't. Please don't claim anything marked "not yet".

---

## Contents

- [The problem](#the-problem)
- [What Lantern does](#what-lantern-does)
- [What it looks like](#what-it-looks-like)
- [A walkthrough](#a-walkthrough)
- [Supported agents](#supported-agents)
- [Supported terminals](#supported-terminals)
- [What's true today](#whats-true-today)
- [Privacy](#privacy)
- [Requirements and permissions](#requirements-and-permissions)
- [Installing, updating and uninstalling](#installing-updating-and-uninstalling)
- [Roadmap](#roadmap)
- [Brand and voice](#brand-and-voice)
- [FAQ](#faq)
- [How it works (technical)](#how-it-works-technical)
- [Building and releasing](#building-and-releasing)
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

1. **Finds every agent on your Mac, automatically.** You don't set anything up or start agents differently. If Claude Code, Codex, OpenCode, Pi, Gemini CLI or another supported agent is running in any terminal, Lantern sees it.
2. **Knows which ones are waiting on you.** Lantern reads each agent's own session log to tell whether it's still working or has finished and is waiting for you, and shows what it last said. With the optional hooks, it also knows the moment Claude Code asks for permission, and which command it's asking about.
3. **One place to look.** The lantern glows amber with a count when agents finish or ask you something. Click it to see **Waiting**: each agent, its project, how long it's been waiting and its last message.
4. **Read the whole conversation.** Click any agent to open its chat: what you asked, what it answered, and a one-line summary of each command or tool it ran.
5. **Answer without switching windows.** Type or dictate a reply and Lantern types it into the agent's own terminal tab: in **cmux**, **Terminal**, **iTerm2** and **tmux**. Right before typing, it checks that the terminal still belongs to the agent: the same process, still running the agent (not a shell it left behind), on that terminal, in the foreground. When it can't tell, it doesn't type. For permission prompts, **Allow** and **Deny** buttons answer right from Lantern; they only press a key if that prompt is still the one on screen.
6. **Notifies you.** An optional macOS notification when an agent finishes or needs you, even when the lantern is out of sight. Clicking it opens that agent's conversation.
7. **Starts new sessions.** Pick any installed agent, a folder and an optional first message, and Lantern opens it in a new Terminal window or runs it in the background.
8. **Stays out of your way.** Collapsed, Lantern is one small lantern on the edge of the screen. It never takes keyboard focus from the app you're using, and it appears on every desktop Space and over full-screen apps.

Lantern does **not** replace your terminals. Your agents keep running exactly where you started them, and you can keep using them there as usual. Lantern is an extra view and remote control on top.

## What it looks like

**The lantern (collapsed).** A small glass circle on the right (or left) edge of the screen containing the Lantern logo, a little orange lantern with eyes.
- **Dim:** nothing needs you.
- **Soft glow:** agents are working.
- **Amber glow with a number:** that many agents have finished or are asking you something since you last looked.
- **Amber glow, no number:** agents are still waiting, but you've already seen them.

**Expanded.** Hover over the lantern and it grows downward into a glass capsule with buttons for **Waiting**, **Agents**, **New session (+)**, **Dictate** and **More**, then the **October** logo. It tucks back in when the mouse leaves. Drag the lantern to move it; it snaps to the nearest screen edge.

**The panel** opens beside the lantern:
- **Waiting:** a card for each agent that finished or needs you since you last looked, showing its logo, handle (e.g. `@claude-2`), project, the terminal it runs in, how long ago, the session title and its last message. Cards have **Reply**, **Open** (jumps to the agent's own tab) and **Dismiss** buttons. A permission prompt ("Permission to run Bash · npm test") gets **Allow** and **Deny** buttons, and **Show full request** shows every line of the command. Below the cards, an **Earlier** list holds agents that are still waiting but that you've seen, with **Clear all**.
- **Agents:** every running agent, the ones that need you first, with logo, handle, folder, terminal and status (*Needs you*, *Your turn*, *Working*, *Idle*, *Running*).
- **Chat:** click any agent to slide into its conversation. Your messages appear on the right in amber bubbles, the agent's replies on the left in glass bubbles, and each command it ran as a one-line entry ("Run · npm test"). It updates live.
- **Composer** at the bottom: "To @claude-2", a message field, a mic button and a send button.
- **New session:** a grid of the agents installed on this Mac (with "more agents Lantern works with…" showing the rest, dimmed, each linking to where to get it), a **Model** menu (Default, the models the agent offers, a search for long lists such as October's, or any model id you type; the last choice per agent is remembered), a folder picker (recent folders, folders agents are running in, or "Choose Folder…"), an optional first message, **Context** (include a screenshot of your screen, with a preview and Retake), **Open in** *Terminal window* or *Background*, and **Start**.
- **Task:** the same form, when you click **+** while you're in DaVinci Resolve, Preview, Keynote or any app other than a terminal: the panel shows that app (and, with Accessibility allowed, its window and open document), asks "What should it do in DaVinci Resolve?", starts in the document's folder (or where the last task for that app ran), and turns the screenshot on. The agent's first message says which app, window and document you're in, and includes the toolkit list. ✕ on the app's card turns it back into a plain session.
- **October Bus:** agents Lantern starts (Claude Code, Codex, OpenCode, October) are connected to the public [October Bus](https://github.com/october-dev/october-bus) and linked with each other, so they can see their peers, message them and share tasks. Their first message says so. Lantern downloads the Bus the first time it's needed (about 6 MB, a signed build checked against a pinned checksum), starts it when needed, and keeps its agents in one scope, `lantern`; nothing is sent off the Mac. Settings › General can turn it off.
- **App sessions:** Claude Desktop (its Code tab), Cowork and the Codex app's sessions are listed too, labeled with their app. Running ones show live; ones from the last three days show as **Recent**, and **Open** opens the app. They can't be typed into from Lantern.
- **October:** three cards. **Connect to October** signs in (Google, GitHub, Apple or email) and then shows the account and plan. **Connect to October Desktop** shows whether October is running, and once you click **Connect** and allow Lantern in October (matching a 6-digit code), it's fully connected. **October phone app** pairs a phone by QR after you sign in, then lists paired phones with Revoke.

**Welcome.** The first launch shows a short, four-page welcome:
1. What the lantern's states mean.
2. The agents Lantern found.
3. Optional setup: open at login, notifications, exact status (hooks) and the shortcut.
4. Where to find everything.

**Settings** (from the menu bar lantern) has four tabs:
- **General:** open at login, which screen edge, the shortcut, automatic updates and the hooks.
- **Notifications:** on or off, and whether finished turns notify too.
- **Agents:** which agents count toward Waiting and notifications.
- **About:** version, check for updates, report a problem, the welcome guide, and uninstall.

**Menu bar.** A small lantern icon: Waiting, Agents, New Session or Task, Message an Agent, Show/Hide Lantern, Settings, Check for Updates, Report a Problem, Welcome Guide, Quit.

**Look and feel.** Dark, translucent glass (Apple's Liquid Glass on macOS 26, a frosted blur on earlier versions) with a light rim. Amber means "needs you", green means "working" and grey means "idle". Each agent is shown with its official logo.

## A walkthrough

Say you have six sessions running in cmux: two Claude Code, two Codex, one OpenCode and one Pi.

1. **Open Lantern.** All six appear within a couple of seconds, each with its logo and a handle: `@claude-1`, `@claude-2`, `@codex-1`, `@codex-2`, `@opencode-1`, `@pi-1`. You don't register or restart anything.
2. **Keep working.** While they run, the lantern glows softly.
3. **An agent finishes.** When `@codex-1` finishes, the lantern turns amber with a **1**, and a notification says "@codex-1 finished · Tests pass. Want me to commit?".
4. **Read and reply.** Click the lantern, then the card, to see the whole conversation. Type or dictate "yes, commit it" and press Enter. Lantern types it into `@codex-1`'s own cmux tab, just as if you'd typed it there.
5. **A permission prompt** (with hooks on). `@claude-2` asks to run a command. Its card says "Claude needs your permission to use Bash" with **Allow** and **Deny**. Click **Allow** and it carries on.
6. **Jump there.** **Open** brings `@opencode-1`'s own tab to the front if you'd rather work in the terminal.
7. **Start another one.** Click **+**, pick Codex and a folder, type "write tests for the parser", and choose *Background*. It starts out of sight and shows up in Agents straight away.

## Supported agents

| Agent | Detected | Working / your turn | Last message and chat | Notes |
|---|---|---|---|---|
| Claude Code | ✅ | ✅ | ✅ | With hooks: instant updates, exact session matching, and permission prompts (with the full command) with Allow/Deny |
| Codex CLI | ✅ | ✅ | ✅ | With hooks: instant "finished" updates |
| OpenCode | ✅ | ✅ | ✅ | Reads OpenCode's local database |
| Pi | ✅ | ✅ | ✅ | |
| October harness (`october`) | ✅ | ✅ | ✅ | October's own agent, built on Pi |
| Gemini CLI | ✅ | ✅ | ✅ | |
| Grok, Cursor Agent, Qwen Code, Goose, Aider, Amp, GitHub Copilot CLI, Kimi, Droid, Crush, Auggie | ✅ | — | — | Shown as *Running*; you can still reply and jump to them |

Headless runs (for example `claude -p` or `codex exec` in a script) are ignored on purpose, because there's no one to answer them.

## Supported terminals

| Where the agent runs | Reply from Lantern | Allow / Deny | Open jumps to |
|---|---|---|---|
| cmux | ✅ Types into the agent's tab | ✅ | Its workspace |
| Terminal | ✅ Types into the agent's tab (asks once for Automation permission) | — | Its tab |
| iTerm2 | ✅ Types into the agent's session (asks once for Automation permission) | ✅ | Its session |
| tmux (inside any terminal) | ✅ | ✅ | Its pane |
| Sessions Lantern started | ✅ (they run in tmux) | ✅ | A Terminal window attached to it |
| Ghostty, VS Code / Cursor terminal, Warp, others | Copies your reply and brings the app forward; you paste with ⌘V | — | The app |

## What's true today

Lantern is in **beta (v0.3.8)**.

| Capability | Status |
|---|---|
| macOS app (macOS 14+, Apple silicon and Intel), signed and notarized | ✅ |
| Automatic updates | ✅ |
| Finds agents in any terminal | ✅ |
| Working / your-turn status, last message, chat view | ✅ Claude Code, Codex, OpenCode, Pi, October harness, Gemini CLI |
| Reply from Lantern | ✅ cmux, Terminal, iTerm2, tmux. Other terminals: copy and paste. |
| Allow / Deny permission prompts | ✅ Claude Code with hooks on, in cmux, iTerm2 and tmux. The card shows the tool and the full command being approved. The key is only pressed if that prompt is still the one on screen. |
| Jump to the agent's exact tab | ✅ cmux, Terminal, iTerm2, tmux |
| Notifications | ✅ Optional |
| Start new sessions (Terminal window or background) | ✅ Background needs tmux |
| Dictation, on-device only | ✅ When the Mac supports on-device recognition for your language; otherwise Lantern says so instead of sending audio to Apple |
| Global shortcut (default ⌃⌥Space, changeable) | ✅ |
| Welcome guide, Settings, Report a Problem, Uninstall | ✅ |
| Automatic routing ("send this to whichever agent it's for") | ❌ Not yet. You choose the agent. |
| Screenshot as context for a new session | ✅ Optional (New Session › Context, or Settings › General). Needs Screen Recording permission. |
| Sign in with an October account (Google, GitHub, Apple, email) | ✅ |
| Connect to October Desktop | ✅ Read-only with any October version that runs october-core (lists October's terminals). Full connection (October's agent names and questions, replies through October's safe delivery, Open on the canvas) needs October 1.0.52 or later, after the user clicks Connect and allows Lantern in October. |
| October phone app | ⚠️ Built, not yet tested against the real phone app and relay: sign in, then pair the phone by QR. Needs a plan that includes mobile. Lantern appears as its own computer on the October account. Treat as experimental until a live pairing and reply have been confirmed. |
| Windows or Linux | ❌ Not yet |
| Pricing | The core app is free forever. No account needed; signing in to October is optional. A paid October plan adds its extras, like cloud agents and multiplayer, automatically after sign-in. |

## Privacy

- **Your code and conversations stay on your Mac unless you connect to October.** Without an October account, Lantern makes two kinds of network request: the daily update check (a public list of versions on GitHub), and anonymous usage counts (below).
- **Anonymous usage counts (on by default, off in Settings › General or on the welcome screen).** Sent to PostHog (US), a few batched requests a day, so we can count daily and monthly users and see which features are used. Each event carries only its name, a random install id, the app version, the macOS version and the chip type, plus these details:
  - `lantern_active`, once a day: how many agents of each kind are running (e.g. 3 Claude Code, 1 Codex).
  - `reply_sent`, `reply_failed`, `reply_uncertain`, `reply_copied`: the agent's kind and how the reply was typed (tmux, cmux, Terminal…).
  - `permission_answered`: allow or deny, and the route.
  - `session_started`, `session_start_failed`: the agent's kind, the model id chosen (or "default"), whether it started in the background, whether a screenshot was included, and for a Task the app's bundle id (e.g. `com.blackmagic-design.DaVinciResolve`), never the document or window title.
  - `chat_opened`, `agent_opened`, `dictation_started`, `hooks_changed` (on or off).
  - `october_signed_in`, `october_signed_out`, `october_desktop_connected`, `phone_paired`, `engine_failed`.

  Never sent: messages, prompts, commands, folder or project names, file paths, agent titles, or anything typed or dictated. No location is looked up from your IP address. If you sign in to October, your usage is linked to your October account (its id and email). Turning the switch off stops sending and drops anything not yet sent. Builds from source have no PostHog key and send nothing.
- **October account (optional).** Signing in talks to October's sign-in service (Supabase) and reads your plan from october.dev. The session is kept in the macOS Keychain.
- **October Desktop (optional).** Lantern talks to October only on your Mac (127.0.0.1), and only after you allow it in October. You can disconnect it from either app.
- **October phone app (optional).** Traffic goes through October's relay and is end-to-end encrypted (Noise), so the relay can't read it. Phones are paired by QR and can be revoked from Lantern.
- **Dictation is on-device only.** Lantern uses Apple's on-device speech recognizer and never sends audio to Apple's servers: if on-device recognition isn't available for your language on this Mac, dictation says so rather than falling back. Lantern listens only while the mic button is on.
- **The toolkit list.** Lantern keeps a short list of what this Mac has in `~/Library/Application Support/October Lantern/toolkit.md`: the chip and memory, notable command-line tools, local AI models (Ollama, whisper, Hugging Face and LM Studio caches), and apps agents can script. It's rebuilt in the background when it's a day old (or from Settings), never sent anywhere by Lantern, and given to agents you start in a Task (and in new sessions if you turn that on). Anything you write under its "Your notes" heading is kept.
- **Tasks.** A Task tells the agent which app you're in, its front window's title and its open document's path. Reading the window and document uses macOS's Accessibility permission, asked only when you click Allow on the Task panel.
- **Screenshots only when you ask.** Lantern takes a screenshot only if you turn on "Include a screenshot of my screen" for new sessions. It captures the display you're on when you open New Session, leaving out Lantern's own windows, and shows you a preview. When you click Start, it's saved on your Mac (`~/Library/Application Support/October Lantern/screenshots`, only readable by you, deleted after a week), and the new agent is given its path with the first message: Codex gets it attached (`--image`), and Claude Code and Gemini CLI are allowed to read that folder. Lantern itself never uploads it; the agent sends it to its model provider like any image you give it. It's never recorded continuously.
- **Read-only by default.** Lantern reads the process list and the agents' own session files (`~/.claude/projects`, `~/.codex/sessions`, OpenCode's database, `~/.pi` / `~/.october` sessions, `~/.gemini`), and never modifies them.
- **Typing replies.** When you send a reply, Lantern types it into that agent's terminal: through cmux's command-line tool, tmux, or macOS Automation for Terminal and iTerm2. macOS asks you once per app before Automation is allowed.
- **Optional hooks.** If you turn on exact status, Lantern adds itself to Claude Code's hooks (`~/.claude/settings.json`: PermissionRequest, Notification, Stop, UserPromptSubmit, PostToolUse, PostToolUseFailure) and Codex's `notify` setting (`~/.codex/config.toml`). It reads and checks both files before changing either, backs both up (never overwriting an earlier backup), replaces each atomically, puts the first file back if the second can't be written, keeps any existing Codex notify program working, and restores everything when you turn hooks off or uninstall. Settings shows Claude Code and Codex separately, and offers an update when an older Lantern's Claude hooks are installed. If Lantern is deleted without uninstalling, the hooks quietly do nothing. The hook events Lantern stores (`~/Library/Application Support/October Lantern/events`) hold the agent's last message and the command it asked permission for.
- **Problem and crash reports are only ever sent by you**, as an email you see before sending. They contain versions and a summary of which agents and terminals are running, never conversations, folders or code.

## Requirements and permissions

- macOS 14 Sonoma or later (the Liquid Glass look needs macOS 26 Tahoe)
- At least one supported agent

| Permission | Why | When it's asked |
|---|---|---|
| Automation (Terminal, iTerm2) | Typing replies into the agent's tab | The first time you reply to an agent in that app |
| Notifications | Telling you when an agent finishes or needs you | In the welcome, or when you turn them on |
| Microphone and Speech Recognition | Dictating a reply | The first time you press the mic button |

| Screen Recording | Including a screenshot when starting a session | When you turn that option on |
| Accessibility | Telling a Task's agent which window and document you have open | When you click Allow on the Task panel |

Lantern doesn't need Full Disk Access.

## Installing, updating and uninstalling

- **Install:** open the DMG and drag **October Lantern** to Applications. Open it; the welcome guide starts.
- **Update:** Lantern checks for updates once a day and asks before installing. *Check for Updates…* is in the menu bar lantern and in Settings › About.
- **Uninstall:** Settings › About › **Uninstall October Lantern…**. It removes the hooks (restoring your agents' settings), Lantern's support files in `~/Library/Application Support/October Lantern`, the login item and its settings, then moves the app to the Trash. Your agents and any sessions Lantern started keep running.

## Roadmap

1. **Prove the October connections live:** a real phone pairing, reply and revoke, and a real October Desktop pairing.
2. **Replies in more terminals:** Ghostty, VS Code and Warp.
3. **Status for more agents:** Grok, Cursor, Qwen, Goose and others.
4. **Automatic routing.** Say "tell the backend agent to use Postgres" and Lantern picks the agent.
5. **Windows.**

## Brand and voice

- **Name:** October Lantern, or "Lantern" for short.
- **Logo:** `logo.png`, a glossy orange paper-lantern character with a handle and two eyes, glowing warm from inside. The app icon puts it on a dark rounded tile (`macos/Resources/AppIcon.icns`).
- **The idea:** a lantern is a small light you keep nearby that shows you where to look. It lights up when something needs you.
- **Colours in the app:**
  - amber `#FAB845`: needs you, or your turn
  - green `#5CCC82`: working
  - red `#ED5C5C`: recording
  - surfaces: dark translucent glass with white text
- **Tone:** calm, plain and honest. Lantern exists to *reduce* noise.
- **One-liners** (suggestions):
  - "Every coding agent on your Mac, in one small light."
  - "Know when your agents need you, and answer without switching windows."
  - "A lantern for your agents: it lights up when one needs you."

## FAQ

**Do I have to launch my agents in a special way?**
No. Lantern finds agents however you started them, in whatever terminal.

**Does it replace my terminal or October Desktop?**
No. Your agents keep running where they are. Lantern is a lightweight view and remote on top. October Desktop is the full workspace for building with teams of agents.

**Does it send my code or conversations anywhere?**
No. Everything runs locally. Without an October account, the only network requests are the update check and anonymous usage counts (feature counts, never content; off in Settings). If you connect a phone through October, replies and agent status travel end-to-end encrypted through October's relay.

**Can it answer permission prompts ("Allow this command?")?**
Yes, for Claude Code with hooks turned on, when the agent runs in cmux, iTerm2 or tmux. Elsewhere, click **Open** to jump to it. Lantern presses the key only if the prompt is still the one it showed you; if it was answered or replaced in the meantime, it says so and presses nothing.

**Why does an agent show "Running" and no conversation, when another one in the same folder works?**
Two agents of the same kind in one folder, and Lantern could only guess which session file is whose. Rather than show you the wrong conversation, it shows neither. Turn on hooks (Claude Code), or start the agent with `--resume <id>`, for an exact match. Replies still go to the right terminal.

**Why does an agent say "Running" instead of "Your turn"?**
Lantern can't read that agent's session files yet (see [Supported agents](#supported-agents)).

**Why did macOS ask whether Lantern can control Terminal?**
That's how Lantern types your reply into the right Terminal or iTerm2 tab. It asks once per app, and you can change it in System Settings › Privacy & Security › Automation.

**How do I uninstall it?**
Settings › About › Uninstall October Lantern. See [Installing, updating and uninstalling](#installing-updating-and-uninstalling).

**Is it free?**
Yes. The core app is free forever, and no account is needed. A paid October plan adds extras like cloud agents and multiplayer after you sign in.

---

## How it works (technical)

```
┌──────────────────────────┐   JSON lines on stdin/stdout   ┌───────────────────────────┐
│  macos/  OctoberLantern  │ ◄────────────────────────────► │  engine/  lantern-engine  │
│  Swift/AppKit/SwiftUI:   │  snapshots, history, replies   │  Rust: find agents, read  │
│  lantern, panels, voice, │                                │  sessions and hooks, type │
│  notifications, updates  │                                │  into terminals, launch   │
└──────────────────────────┘                                └───────────────────────────┘
```

- **Engine (`engine/`, Rust).** A small helper the app starts and talks to over stdin/stdout ([protocol](../protocol/README.md)). About every 1.5 seconds it:
  1. reads the process table and picks out interactive agent processes by executable name (including `node`/`bun`/`python` wrappers);
  2. finds the app each one runs in by walking up its parent processes, and matches its terminal against tmux panes;
  3. works out how to type into it: tmux, cmux (from the `CMUX_WORKSPACE_ID` / `CMUX_SURFACE_ID` in the agent's environment), or Terminal / iTerm2 (AppleScript, matching the tab's tty);
  4. reads each agent's session to decide *working* vs *your turn* and get the last message and title. It uses Claude Code transcripts (each entry's `stop_reason` and timestamp), Codex rollout files (found through the process's open files; turn events carry their own times), OpenCode's SQLite database (read-only), Pi and October harness JSONL, and Gemini CLI JSONL; the last three are matched by folder and start time;
  5. merges hook events (from `lantern-engine hook ...`), using whichever is newer;
  6. sends the app a snapshot whenever something changed.

  It also serves conversation history, sends single keys for Allow/Deny, focuses tabs, and starts new sessions on Lantern's own tmux server (`tmux -L lantern`) through the user's login shell. Replies, keys and focus run on one worker per terminal with a deadline, off the scan loop. Right before typing it re-checks the target (same pid and start time, same program, same tty, in the foreground, the route's pane or tab still on that tty) and refuses when it can't tell. A send that times out before it starts is canceled; one that started but can't be confirmed is reported as uncertain. It has no UI, so a Windows front end could reuse it.
- **App (`macos/`, Swift).**
  - **Windows:** the lantern and the panel are borderless, non-activating `NSPanel`s inside an `NSGlassEffectView` (macOS 26) or `NSVisualEffectView` container.
  - **Hover:** expansion works by polling the mouse position.
  - **Dictation:** `SFSpeechRecognizer` with `requiresOnDeviceRecognition`; refuses when on-device recognition isn't supported.
  - **Shortcut:** Carbon `RegisterEventHotKey`, which needs no Accessibility permission.
  - **Notifications:** `UserNotifications`.
  - **Updates:** Sparkle 2, with EdDSA-signed updates from an appcast in the releases repo.

## Building and releasing

Run the following commands from the repository root.

Requires Xcode 26 (the app references `NSGlassEffectView`, which older SDKs don't have) and Rust 1.85 or later, edition 2024 (`brew install rustup && rustup default stable && rustup target add x86_64-apple-darwin`).

```sh
scripts/build-app.sh          # engine + app → build/October Lantern.app (ad-hoc signed)
open "build/October Lantern.app"
(cd engine && cargo fmt --all -- --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked)   # the gate release.sh runs
```

Development flags:
- `--open inbox|agents|new|october|chat|welcome:N|settings:N` opens a panel or window at launch.
- `--snapshot <dir>` draws the welcome pages and settings tabs offscreen into PNGs, then quits.

Engine commands:
- `cargo run -- agents`
- `cargo run -- history <agent-id>`
- `cargo run -- probe opencode|pi|october|gemini <folder>`
- `cargo run -- hooks status|install|uninstall|remove-all`

**Releasing** (from the Mac that holds the signing keys):

```sh
# once per machine: the notarization password, kept in the Keychain
xcrun notarytool store-credentials lantern-notary --apple-id <apple-id> --team-id 75D25SJRM5 --password <app-specific-password>

# 1. bump `version` in engine/Cargo.toml (the app's version comes from it)
# 2. run the checks, then build, sign, notarize and staple the app and DMG, sign the update and write the appcast
scripts/release.sh
# 3. publish: uploads the DMG to a GitHub release and pushes the new appcast.xml (safe to rerun if it fails part-way)
scripts/publish.sh "Release notes in one paragraph"
```

- **Keys:** the Developer ID certificate, the `lantern-notary` Keychain profile and the Sparkle private key (`generate_keys --account october-lantern`) all live in the release Mac's Keychain. Back up the Sparkle key (`generate_keys --account october-lantern -x <file>`): without it, existing installs can't receive updates.
- **Where downloads live:** [harshsaver/october-lantern-releases](https://github.com/harshsaver/october-lantern-releases), a public repo holding only release files and `appcast.xml`. Always upload the DMG as `October-Lantern.dmg` so the `latest/download` link keeps working.

## Repository layout

| Path | What |
|---|---|
| `engine/` | Rust engine: agent detection, session readers, hooks, delivery, launching |
| `macos/` | Swift package for the macOS app. `Resources/` holds the app icon and logos. |
| `protocol/` | The message protocol between the app and the engine |
| `scripts/` | Build, release, publish and icon scripts |
| `logo.png` | The Lantern logo |

Harness logos come from October Desktop. Some come from Agent Orchestrator under Apache-2.0; see `macos/Resources/harness/NOTICE.md`. All names and logos are trademarks of their owners.

## Glossary

| Term | Meaning |
|---|---|
| **Agent** | An AI coding tool running in a terminal: Claude Code, Codex, OpenCode, Pi and so on |
| **Harness** | The October family's word for an agent's program (Claude Code is a harness) |
| **Session** | One running agent conversation in one terminal |
| **Handle** | Lantern's short name for a session, e.g. `@claude-2`. It stays the same while the session runs. |
| **Your turn / waiting** | The agent finished and is waiting for you |
| **Needs you** | The agent is blocked on a question or permission prompt (known with hooks) |
| **Hooks** | An optional setting that makes Claude Code and Codex notify Lantern directly |
| **Waiting / Earlier** | New turns since you last looked / turns still waiting that you've already seen |
