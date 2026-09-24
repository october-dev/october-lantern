# Notes for lantern.october.dev

What the website needs for the beta. Product facts come from [product guide](docs/PRODUCT.md), the source of truth. Don't claim anything its "What's true today" table marks "not yet".

## Must change

1. **Download button.** The site only has an early-access mailto today. Link to:
   `https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern.dmg`
   This link always serves the newest release (0.3.0 is 10.5 MB). Show next to it: "macOS 14 or later · Apple silicon and Intel · Free during the beta".
2. **Privacy page** (`/privacy`). Use the product guide's Privacy section, which covers:
   - everything stays on the Mac unless you choose to sign in to October; no account is needed
   - without an October account, the only network request is the daily update check (a public file on GitHub)
   - dictation is on-device only (Lantern refuses rather than sending audio to Apple)
   - there's no screen recording
   - session files are only read, never changed
   - Automation permission is used only to type replies into Terminal and iTerm2
   - the optional hooks are backed up and restored
   - problem and crash reports are only ever sent by the user, as an email they see first
3. **Install and uninstall help.** Install: open the DMG, drag to Applications. Uninstall: Settings › About › Uninstall October Lantern (it restores the agents' settings). From the product guide section "Installing, updating and uninstalling".

## Should add

- **Which agents and terminals work**, from the product guide tables "Supported agents" and "Supported terminals". Be exact:
  - Full support: Claude Code, Codex, OpenCode, Pi, October harness and Gemini CLI.
  - Replying from Lantern: cmux, Terminal, iTerm2 and tmux.
  - Ghostty, VS Code and Warp: copy and paste for now.
- **Permissions**, so nobody is surprised during setup: Automation (Terminal / iTerm2, asked on the first reply), Notifications (optional), Microphone and Speech Recognition (only for dictation). No Accessibility, Screen Recording or Full Disk Access.
- **FAQ entries** from the product guide FAQ, especially "Why did macOS ask whether Lantern can control Terminal?" and "How do I uninstall it?".
- **Support contact:** hey@october.dev. The app's Report a Problem uses the same address.

## Don't claim

- Automatic routing ("say it once and it picks the agent") or screen/screenshot context: neither exists yet.
- October connections: describe them exactly as in the product guide's "What's true today" table. Full October Desktop support needs October 1.0.52+. The phone connection is built but has not been tested against the real phone app: don't advertise it until a live pairing and reply have been confirmed.
- Windows or Linux.
