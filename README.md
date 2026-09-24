<p align="center">
  <a href="https://lantern.october.dev">
    <img src="docs/assets/banner.png" alt="October Lantern — Know when your AI agents need you." width="1280" />
  </a>
</p>

<h1 align="center">Your agents work. You get your day back.</h1>

<p align="center">
  A little light at the edge of your Mac that glows when an AI coding agent needs you.<br />
  Read what it said, send a reply, and get back to what you were doing.
</p>

<p align="center">
  <a href="https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern.dmg">
    <img src="docs/assets/download-macos.svg" alt="Download Lantern for Mac — Apple silicon and Intel" width="260" height="64" />
  </a>
</p>

<p align="center">
  macOS 14+ · Apple silicon &amp; Intel · Free forever · No account needed
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

Collapsed, it's one small lantern. It stays beside you across desktop Spaces and full-screen apps. Optional notifications let you step away from the bar, too.

## Up and running in a minute

1. **[Download Lantern for Mac](https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern.dmg)** and open the disk image.
2. **Drag October Lantern into Applications** and open it.
3. **Keep working with your agents.** Lantern finds supported sessions already running on your Mac. The welcome guide walks you through optional notifications and hooks.

Press **Control + Option + Space** whenever you want to write a reply. Use the mic button to dictate instead. The shortcut is customizable in Settings.

## Your tools. One place to check.

### Agents

| Agents                                                                                               | What you can see                                                                                                               |
| ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| **Claude Code, Codex, [October Harness](https://harness.october.dev), OpenCode, Pi, Gemini CLI**     | Working / your-turn status, the last message, and conversation history.                                                        |
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

Your code and conversations stay on your Mac when you use Lantern on its own. No account is needed. The standalone app's only network request is its update check.

- **On-device dictation.** Audio stays on your Mac. If your language doesn't support on-device recognition, Lantern tells you.
- **No screen recording.** Lantern reads supported agents' local session files.
- **You're in control.** Replies are sent when you choose. Optional hooks can be removed in Settings.
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

The core Mac app is free, and it stays free forever. The source code is licensed under Apache 2.0. If you have a paid October plan, sign in and your plan's extras, like cloud agents and multiplayer, are available in Lantern automatically. Lantern currently runs on macOS 14 or later, on Apple silicon and Intel. Windows and Linux apps aren't available yet.

</details>

<details>
<summary><strong>Why does macOS ask to control my terminal?</strong></summary>

Lantern uses macOS Automation to send your reply to the right Terminal or iTerm2 tab. Permission is requested the first time you reply through that app. Lantern doesn't require Accessibility, Screen Recording, or Full Disk Access.

</details>

<details>
<summary><strong>How do I update or uninstall it?</strong></summary>

Lantern checks for updates automatically and asks before installing. You can also use **Check for Updates** in the menu bar.

To uninstall, open **Settings → About → Uninstall October Lantern**. This removes Lantern's optional hooks and support files. Your agents keep running.

</details>

## Build and contribute

Want to improve Lantern or add support for your favorite agent? The app uses Swift / AppKit / SwiftUI, with a Rust engine for agent discovery and terminal integration.

With **Xcode 26** and a current **stable Rust toolchain** installed, run these commands from the repository root:

```sh
scripts/build-app.sh
open "build/October Lantern.app"
```

Run `scripts/check.sh` before submitting changes. It checks the engine, builds and tests the Mac app, and validates the shell scripts.

See the [product and development guide](docs/PRODUCT.md), [engine protocol](protocol/README.md), and [known limitations](docs/PRODUCT.md#whats-true-today). For help or feedback, write to [hey@october.dev](mailto:hey@october.dev).

## License

Licensed under the [Apache License 2.0](LICENSE). Third-party components and assets retain their own licenses and notices; see [NOTICE](NOTICE).

---

<p align="center">Made by <a href="https://www.october.dev/">October</a>, for people with a few agents on the go.</p>
