# Notes for lantern.october.dev

What the website needs for the beta. Product facts come from [product guide](docs/PRODUCT.md), the source of truth. Don't claim anything its "What's true today" table marks "not yet".

## Must change

1. **Download button.** The site only has an early-access mailto today. Link to:
   `https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern.dmg`
   This link always serves the newest release (0.3.0 is 10.5 MB). Show next to it: "macOS 14 or later · Apple silicon and Intel · Free during the beta".
2. **Privacy page** (`/privacy`). Use the product guide's Privacy section, which covers:
   - everything stays on the Mac unless you choose to sign in to October; no account is needed
   - without an October account, the only network requests are the daily update check (a public file on GitHub) and anonymous usage counts (feature counts, never content), which are on by default and can be turned off in Settings; the product guide lists every event
   - dictation is on-device only (Lantern refuses rather than sending audio to Apple)
   - there's no screen recording; a screenshot is taken only if you turn on "include a screenshot" for new sessions, and it stays on the Mac (the agent gets its path)
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
- **Permissions**, so nobody is surprised during setup: Automation (Terminal / iTerm2, asked on the first reply; browsers, only to read the page address for Point & Ask), Notifications (optional), Microphone and Speech Recognition (only for dictation), Screen Recording (only for Point & Ask and new-session screenshots), Accessibility (only if you let Lantern see which window or document is open, or the selected text). No Full Disk Access.
- **FAQ entries** from the product guide FAQ, especially "Why did macOS ask whether Lantern can control Terminal?" and "How do I uninstall it?".
- **Support contact:** hey@october.dev. The app's Report a Problem uses the same address.
- **Point & Ask** (0.4+): press ⌃⌥P or the pill's dotted-box button, drag a box around anything on screen, then **Ask** (October's AI answers in a card, needs October sign-in; Pro and Max use the plan's Assistant credit, the free plan gets 20 questions a day) or **Send to @agent**. Exact wording and privacy: the product guide's Point & Ask entries.
- **The assistant** (0.4.3+): click the lantern. **Ask** chats with October's AI and knows what your agents are doing ("which agent needs me first?"); **Do** turns a request into a filled-in New Session that you start yourself. Waiting has its own tray button.

## Don't claim

- Automatic routing ("say it once and it picks the agent") doesn't exist yet. Screenshots go with new sessions and Point & Ask, not with ordinary replies.
- The assistant's **Do** doesn't start anything by itself: it fills in New Session, and you press Start.
- October connections: describe them exactly as in the product guide's "What's true today" table. Full October Desktop support needs October 1.0.52+. The phone connection is built but has not been tested against the real phone app: don't advertise it until a live pairing and reply have been confirmed.
- Windows or Linux.
