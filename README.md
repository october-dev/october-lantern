<p align="center">
  <a href="https://lantern.october.dev">
    <img src="docs/assets/banner.png" alt="October Lantern — Know when your AI agents need you." width="1280" />
  </a>
</p>

<h1 align="center">Your agents work. You get your day back.</h1>

<p align="center">
  A little light that glows when an AI coding agent needs you.<br />
  Read what it said, send a reply, and get back to what you were doing.
</p>

<p align="center">
  <a href="https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern.dmg">
    <img src="docs/assets/download-macos.svg" alt="Download Lantern for Mac — Apple silicon and Intel" width="260" height="64" />
  </a>
  <a href="https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern-windows-x86_64-setup.exe">
    <img src="docs/assets/download-windows.svg" alt="Download Lantern for Windows — Windows 10 and 11, 64-bit" width="290" height="64" />
  </a>
</p>

<p align="center">
  macOS 14+ · Windows 10 &amp; 11 (preview) · Free forever · No account needed
</p>

<p align="center">
  <a href="https://lantern.october.dev">Website &amp; live demo</a> ·
  <a href="https://github.com/harshsaver/october-lantern-releases/releases">Release notes</a> ·
  <a href="mailto:hey@october.dev">Get help</a>
</p>

---

## Less checking. More getting things done.

Claude is working in one window. Codex is waiting in another. October finished ten minutes ago.

Lantern watches tools like Claude Code, Codex, October, and Cursor, and brings them together in one quiet place. Keep your favorite agents and terminals, and start them exactly as you do today. Lantern finds supported sessions automatically.

| When this happens…                 | Lantern helps you…                                                                       |
| ---------------------------------- | ---------------------------------------------------------------------------------------- |
| An agent finishes or needs a reply | See the lantern glow, with a count of new things waiting for you.                        |
| You want to know what happened     | Open the conversation, including messages and summaries of tool activity.                |
| You have an answer                 | Type or dictate it, then send it to a supported terminal without hunting for the window. |
| Claude asks for permission         | Inspect the request and choose Allow or Deny with optional hooks in supported terminals. |
| You want to start something new    | Pick an installed agent and a folder, then open a terminal or background session.        |
| Something on screen puzzles you    | Point & Ask: drag a box around it, ask, and get an answer, or send it to an agent.       |
| You have a quick question          | Click the lantern to ask, like "which agent needs me first?", or describe a task to start. |
| A task needs an app or a website   | Agents Lantern starts can use your apps and signed-in browser in the background, with their own cursor ([Cua Driver](https://github.com/trycua/cua)). |

Collapsed, it's one small lantern. It stays beside you across desktop Spaces and full-screen apps. Optional notifications let you step away from the bar, too.

## Up and running in a minute

1. **[Download Lantern for Mac](https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern.dmg)** and open the disk image.
2. **Drag October Lantern into Applications** and open it.
3. **Keep working with your agents.** Lantern finds supported sessions already running on your Mac. The welcome guide walks you through optional notifications and hooks.

Prefer Terminal? One command downloads, installs, and opens the latest release:

```sh
curl -fsSL https://lantern.october.dev/install.sh | sh
```

Press **Control + Option + Space** whenever you want to write a reply, and **Control + Option + P** for Point & Ask. Use the mic button to dictate instead. Both shortcuts are customizable in Settings.

## Your tools. One place to check.

### Agents

| Agents                                                                                               | What you can see                                                                                                               |
| ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| **Claude Code, Codex, [October Harness](https://harness.october.dev), OpenCode, Pi, Gemini CLI, Muse (Meta)** | Working / your-turn status, the last message, and conversation history.                                                        |
| **Grok, Cursor Agent, Qwen Code, Goose, Aider, Amp, GitHub Copilot CLI, Kimi, Droid, Crush, Auggie** | Running sessions, with replies and navigation where the terminal supports them. Detailed status and chat aren't available yet. |

Optional hooks add prompt completion updates for Codex and permission requests for Claude Code. When Lantern can't confidently match a conversation to a session, it says so.

### Terminals

| Where your agent runs                                   | Reply from Lantern                                                          |
| ------------------------------------------------------- | --------------------------------------------------------------------------- |
| **cmux, Terminal, iTerm2, tmux**                        | Send directly to the agent's tab or pane.                                   |
| **Background sessions started by Lantern**              | Send directly; these sessions use tmux.                                     |
| **Ghostty, VS Code, Cursor, Warp, and other terminals** | Copy your reply and bring the terminal app forward, ready for you to paste. |

**Allow / Deny** works with Claude Code's optional hooks in cmux, iTerm2, and tmux. Terminal supports text replies, but not these permission buttons. See the [full compatibility guide](docs/PRODUCT.md#supported-terminals) for details.

## Private by default

Your code and conversations stay on your Mac when you use Lantern on its own. No account is needed. On its own, Lantern connects to the internet for two things: its update check, and anonymous usage counts, which you can turn off in Settings.

- **On-device dictation.** Audio stays on your Mac. If your language doesn't support on-device recognition, Lantern tells you.
- **No screen recording.** Lantern reads supported agents' local session files. It captures your screen only when you ask it to: the area you select in Point & Ask, or a screenshot for a new session when you turn that on.
- **Asking October's AI is optional.** Point & Ask's **Ask** and the assistant's **Ask** send your question (and, for Point & Ask, the selected area) to October only when you press Ask, and need an October sign-in. Nothing is stored there; the [privacy guide](docs/PRODUCT.md#privacy) lists exactly what's sent.
- **You're in control.** Replies are sent when you choose. Optional hooks can be removed in Settings.
- **Anonymous usage counts.** How often Lantern and its features are used (e.g. "a reply was sent through tmux"), never content. The [privacy guide](docs/PRODUCT.md#privacy) lists exactly what's sent. Off with one switch in Settings; builds from source send nothing.
- **Optional October connections.** Signing in and connecting other devices introduces network connections; the [privacy guide](docs/PRODUCT.md#privacy) explains each one.

## A little Lantern. A bigger October.

Lantern is useful on its own. When your work grows into teams of agents, people, repositories, or computers, [October Desktop](https://www.october.dev/) gives them a shared workspace.

Connecting Lantern to October Desktop is optional. Phone pairing is experimental and still awaiting end-to-end verification; see [current capabilities](docs/PRODUCT.md#whats-true-today).

## A few quick answers

<details>
<summary><strong>Do I have to change how I start my agents?</strong></summary>

No. Keep opening your agents in your usual terminals. Lantern discovers supported interactive sessions on your Mac. You can also start a session from Lantern.

</details>

<details>
<summary><strong>Is Lantern free? Does it work on Windows or Linux?</strong></summary>

The core Mac app is free, and it stays free forever. The source code is licensed under Apache 2.0. If you have a paid October plan, sign in and your plan's extras, like cloud agents, are available in Lantern automatically. Lantern runs on macOS 14 or later (Apple silicon and Intel) and, as a preview, on Windows 10 and 11 (64-bit): the same pill and panel as the Mac app, written in Rust. The Windows installer isn't code-signed yet, so Windows shows "Windows protected your PC" the first time: click **More info**, then **Run anyway**. Linux builds from source.

</details>

<details>
<summary><strong>Why does macOS ask to control my terminal?</strong></summary>

Lantern uses macOS Automation to send your reply to the right Terminal or iTerm2 tab. Permission is requested the first time you reply through that app. Lantern doesn't require Full Disk Access. Screen Recording is asked for only if you turn on screenshots, and Accessibility only if you let a Task see which document you have open.

</details>

<details>
<summary><strong>How do I update or uninstall it?</strong></summary>

Lantern checks for updates automatically and asks before installing. You can also use **Check for Updates** in the menu bar.

To uninstall, open **Settings → About → Uninstall October Lantern**. This removes Lantern's optional hooks and support files. Your agents keep running.

</details>

## Build and contribute

Want to improve Lantern or add support for your favorite agent? The app uses Swift / AppKit / SwiftUI, with a Rust engine for agent discovery and terminal integration.

### Linux and Windows (Rust)

`desktop/` is the Linux and Windows app: the Mac app's design and behaviour in Rust (egui).
Lantern is a small pill at the edge of the screen; hover it for its actions, click the
lantern for Ask Lantern, and its panel opens beside it with Waiting, Your agents, chats,
New Session and October. Point & Ask, notifications, the tray and global shortcuts
(Ctrl+Alt+Space messages an agent, Ctrl+Alt+P starts Point & Ask) work as on the Mac.
`--new-session`, `--point-ask`, `--ask`, `--connections`, `--settings` and `--welcome`
open those directly (a second launch hands them to the running Lantern), and
`--fixture FILE.json --out FILE.png --scale 2` renders any surface from a saved model.

Where a platform can't do what the Mac does, Lantern does the closest thing and says so:
GNOME has no backdrop blur, so the glass is a tinted fill; on Wayland the pill runs through
XWayland (a Wayland app can't place its own window), and Point & Ask can't pick another
app's window, so a click takes the whole screen; without XWayland at all there's no edge
pill, and Lantern opens as a normal window. The lantern's right-click menu has Quit, so
Lantern can be closed without a tray host (GNOME needs the AppIndicator extension). The shortcut choices
are the Mac's, except ⌥Space: Alt+Space opens the window menu on GNOME and Windows.

#### Linux

Install a current stable Rust toolchain from [rustup](https://rustup.rs), a C compiler,
Mesa/OpenGL, and your distribution's Wayland/X11 runtime libraries. Install `tmux`
for direct replies and background sessions. Database history uses bundled SQLite.
On Fedora, these can be installed with:

```sh
sudo dnf install gcc tmux mesa-libGL libxkbcommon wayland-libs
```

Build and run from the repository root:

```sh
scripts/build-app.sh --debug
./build/linux/october-lantern
```

Omit `--debug` for an optimized build. Both `october-lantern` and `lantern-engine`
are placed in `build/linux/`; keep them together. The desktop starts the engine as
its sidecar using the same JSON-lines protocol as the Mac app. `LANTERN_ENGINE`
can override its path during development.

Replies go through the engine's safety checks. Agents in ordinary terminals are
discovered too; sessions started In Lantern run in `tmux` and take direct replies, and
agents in your own terminals do too when they run inside `tmux`. Clipboard replies work for other terminals. Supported terminal
launchers include Ptyxis, GNOME Terminal, Konsole, Kitty, Alacritty, Ghostty, Foot,
WezTerm, Xfce Terminal, and xterm. Set `LANTERN_TERMINAL` to a supported executable
name or absolute path to choose one.

Linux data lives under `${XDG_DATA_HOME:-~/.local/share}/october-lantern`.
Preferences live in `${XDG_CONFIG_HOME:-~/.config}/october-lantern/lantern-state.json`
(the terminal and dictation paths in `settings.json` beside it). Settings has the
Mac's sections: open at login, the pill's edge, shortcuts, updates, new-session
defaults, agent hooks, notifications and which agents count in Waiting. Hooks back up and
update supported agents' configuration. Screenshots and the folder picker use XDG
desktop portals; shortcuts need approval in the desktop's dialog once. Notifications and
credential refresh continue while the desktop is locked. Repeated
snapshots are coalesced so they do not build up in the background.
Screenshots are private files, pruned after a week; audio recordings are removed
after dictation. Lantern never sends dictation audio to a cloud service.

To install local dictation (C++ compiler and CMake, or `uv`, required):

```sh
scripts/setup-dictation.sh
```

This installs whisper.cpp and a checksum-verified multilingual base model, and
configures Lantern. Restart the app afterward. A `whisper-cli` from your package manager
works too: with one on `PATH` but no model yet, pressing the mic asks first, then on
**Download** fetches the model (148 MB, once, checked against its SHA-256) into Lantern's
data folder and starts listening. PipeWire's `pw-record` records audio.
On Fedora the optional runtime packages are `pipewire-utils libsecret`.

To bundle October Bus from the pinned source (Go 1.27+ required):

```sh
scripts/build-bus-linux.sh
python3 scripts/smoke-linux.py --bus
```

The native Bus executable lives beside the engine. New sessions can enable Bus
and share the local toolkit; no October account is needed. `LANTERN_BUS_BIN` can
override the bundled binary for development.

The October panel has October account sign-in and October Desktop pairing. Saved
sign-in uses the Linux Secret Service through `secret-tool`; without it, sign-in works
for the current session. Ask Lantern and Point & Ask stream October AI answers with
follow-up questions; October receives a question (and Point & Ask's screenshot) only
when you ask. October Harness, the default agent when Claude Code and Codex aren't
installed, installs itself with its own checksum-verified Node.js.

The app can check Linux updates daily when enabled, and install a checksum-verified
native bundle on request. `scripts/package-linux.sh` creates the bundle and SHA-256
file; the Linux release workflow builds them as CI artifacts, and
`scripts/publish-desktop.sh` publishes them next to the Mac's DMG (see
[docs/RELEASING.md](docs/RELEASING.md)). Updates come from
`harshsaver/october-lantern-releases`; a fork can set `LANTERN_RELEASES_REPO=owner/repo`
at build time to use its own. It never uses Mac release assets. Database history is read directly with bundled
SQLite, so the app does not need a system `sqlite3` executable.

#### Windows (preview)

The same app and engine build for Windows 10 and 11. Install stable Rust
from [rustup](https://rustup.rs) with the MSVC toolchain (Visual Studio Build Tools),
then from PowerShell in the repository root:

```powershell
pwsh scripts/build-windows.ps1 -Debug
.\build\windows\october-lantern.exe
```

Omit `-Debug` for an optimized build without a console window. `scripts/check.ps1` is
the Windows check, and `python scripts/smoke-windows.py` tests discovery, sessions and
replies against a harmless agent.

To bundle October Bus (Go 1.27+) and dictation's speech runtime (whisper.cpp's
`whisper-cli.exe`, built from pinned source with git, CMake and the Visual Studio C++ tools),
and build the release zips (the app, and the speech runtime) and installer
([Inno Setup](https://jrsoftware.org/isinfo.php)):

```powershell
pwsh scripts/build-bus-windows.ps1
pwsh scripts/build-whisper-windows.ps1
pwsh scripts/package-windows.ps1
```

The installer (`October-Lantern-windows-x86_64-setup.exe`) installs for the current user
without administrator rights, adds a Start menu shortcut and optionally starts Lantern at
sign-in; uninstalling also removes Lantern's agent hooks. `package-windows.ps1` signs the
programs and the installer with Azure Artifact Signing or a `.pfx` certificate when one is
configured; the Windows release workflow reads them from repository secrets (see
[docs/RELEASING.md](docs/RELEASING.md)).

What works: Lantern finds Claude Code, Codex and the other supported agents running
in Windows Terminal, VS Code, Cursor, WezTerm and plain console windows, including
npm installs. It shows their state, last message and full conversation. Open brings
the agent's terminal window to the front (the window, not the exact tab). You also
get the tray menu, notifications, the global shortcuts, start at sign-in, saved sign-in in
Windows Credential Manager, and agent hooks.

Replies go to sessions Lantern starts (New Session). Lantern runs the agent in a
session of its own (a ConPTY pseudo console, in place of tmux) that keeps running when
you close Lantern. Open terminal shows it in a Windows Terminal tab, or a console window
when Windows Terminal isn't installed, and closing that window leaves the agent running.
Lantern runs the agent itself, not through a shell or `cmd.exe` (npm's `.cmd` shims are
unwrapped to Node), and the session ends when the agent exits, so a reply can't land in a
shell. Before typing, it checks that the agent is still the process the session started,
on that session's console. Agents you start in your own terminal are followed but never
typed into: Windows has no safe way to type into another app's terminal.

Sessions can join October Bus like on Linux. Screenshots (Point & Ask and New Session)
capture the display under the pointer with Lantern's own windows left out. Dictation
records your default microphone and transcribes on this computer with whisper.cpp, whose
`whisper-cli.exe` the installer puts next to Lantern. The first time you press the mic, Lantern
asks before downloading anything ("Turn on dictation?"). On **Download** it fetches the speech
model (148 MB, once, from Hugging Face, checked against its SHA-256) into
`%LOCALAPPDATA%\October Lantern\speech`, showing its progress, then starts listening; **Not
now** (or Esc) downloads nothing, and the next press asks again. A copy updated in the app
(whose update zip carries no speech runtime, so older versions can still install it) also
downloads `whisper-cli.exe` there first, from its own version's release
(`October-Lantern-whisper-windows-x86_64.zip`, 1 MB, checked like an update); the question
says so. If
Windows keeps desktop apps from the microphone, Lantern says so and opens the privacy
setting. (`scripts/setup-dictation.ps1` still sets dictation up for a build from source.) The app installs updates itself on request: it downloads
the release zip, checks its SHA-256 and that every program is a native Windows build,
unpacks it into its own folder, checks the new app can talk to its engine, then restarts
and points the Start menu shortcut there. Agent sessions, their windows and the Bus run
from Lantern's own copies in its data folder, so updating or reinstalling never stops
them. Data lives in `%LOCALAPPDATA%\October Lantern`, preferences in
`%APPDATA%\October Lantern`. The
preview is built and tested in CI (`scripts/check-windows-types.sh` also type-checks the
Windows code from Linux). It hasn't been tried by hand on a Windows machine yet, so please
report what you find.

### macOS

With **Xcode 26** and a current **stable Rust toolchain** installed, run these commands from the repository root:

```sh
scripts/build-app.sh
open "build/October Lantern.app"
```

Run `scripts/check.sh` before submitting changes. It checks the engine, builds and tests the UI for the current OS, and validates the shell scripts.

See the [product and development guide](docs/PRODUCT.md), [engine protocol](protocol/README.md), and [known limitations](docs/PRODUCT.md#whats-true-today). For help or feedback, write to [hey@october.dev](mailto:hey@october.dev).

## License

Licensed under the [Apache License 2.0](LICENSE). Third-party components and assets retain their own licenses and notices; see [NOTICE](NOTICE).

---

<p align="center">Made by <a href="https://www.october.dev/">October</a>, for people with a few agents on the go.</p>
