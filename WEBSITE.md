# Notes for lantern.october.dev

What the website needs for the beta. Product facts come from [product guide](docs/PRODUCT.md), the source of truth. Don't claim anything its "What's true today" table marks "not yet".

## Must change

1. **Download button.** The site only has an early-access mailto today. Link to:
   `https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern.dmg`
   This link always serves the newest release (0.3.0 is 10.5 MB). Show next to it: "macOS 14 or later · Apple silicon and Intel · Free to download".
   Windows (preview, 0.6+): `https://github.com/harshsaver/october-lantern-releases/releases/latest/download/October-Lantern-windows-x86_64-setup.exe`, "Windows 10 and 11 · 64-bit · preview". Until the installer is code-signed, say that Windows asks for **More info › Run anyway** the first time.
2. **Privacy page** (`/privacy`). Use the product guide's Privacy section, which covers:
   - everything stays on the Mac unless you choose to sign in to October; no account is needed
   - without an October account, the only network requests are the daily update check (a public file on GitHub) and anonymous usage counts (feature counts, never content), which are on by default and can be turned off in Settings; the product guide lists every event
   - dictation is on-device only (Lantern refuses rather than sending audio to Apple)
   - there's no screen recording; a screenshot is taken only if you turn on "include a screenshot" for new sessions, and it stays on the Mac (the agent gets its path)
   - session files are only read, never changed
   - Automation permission is used only to type replies into Terminal and iTerm2
   - the optional hooks are backed up and restored
   - problem and crash reports are only ever sent by the user, as an email they see first
   - your own AI key (optional) stays in the system's credential store (Keychain, Credential Manager, Secret Service); each question sends it to October's server only to pass that question on to your provider, and it's never stored, logged or counted
3. **Install and uninstall help.** Install: open the DMG, drag to Applications. Uninstall: Settings › About › Uninstall October Lantern (it restores the agents' settings). From the product guide section "Installing, updating and uninstalling".

## Pricing (from the product guide's "Plans" section)

- **Free:** the whole app, plus 5 questions a day on October's AI (Ask, Point & Ask, Show me) with an October account.
- **Pro trial:** 3 days free with a card up front, then $9.99 for the first month, then $20 a month. During the trial you get what Lantern Lifetime gets. Show the exact charge date at checkout; cancelling is one click in the billing portal.
- **Lantern Lifetime:** $149, then $199, then $249 (100 seats per tier; show only the live price and seats from `/lifetime-status`, never a made-up number). Includes your own AI key with no Lantern daily limit, up to 20 questions a day on October's AI, the iPhone app (beta), Show me, and every Lantern app update. Not October Cloud or monthly October credits. Full refund on request within 14 days.
- **Your own AI key:** OpenAI, Anthropic or OpenRouter, set in Settings › October. Say plainly: "Your provider bills you for these questions."
- The app's **Start free trial** and **Buy Lifetime** open `lantern.october.dev/buy?offer=trial|lifetime&ref=app` (plus `&email=` with the signed-in October email), which goes straight to Stripe checkout; the site's own links use their own `ref`.

## Should add

- **Which agents and terminals work**, from the product guide tables "Supported agents" and "Supported terminals". Be exact:
  - Full support: Claude Code, Codex, OpenCode, Pi, October harness, Gemini CLI and Muse (Meta).
  - Replying from Lantern: cmux, Terminal, iTerm2 and tmux.
  - Ghostty, VS Code and Warp: copy and paste for now.
- **Permissions**, so nobody is surprised during setup: Automation (Terminal / iTerm2, asked on the first reply; browsers, only to read the page address for Point & Ask), Notifications (optional), Microphone and Speech Recognition (only for dictation), Screen Recording (only for Point & Ask and new-session screenshots), Accessibility (only if you let Lantern see which window or document is open, or the selected text). No Full Disk Access.
- **FAQ entries** from the product guide FAQ, especially "Why did macOS ask whether Lantern can control Terminal?" and "How do I uninstall it?".
- **Support contact:** hey@october.dev. The app's Report a Problem uses the same address.
- **Point & Ask** (0.4+): press ⌃⌥P or the pill's dotted-box button, drag a box around anything on screen, then **Ask** (October's AI answers in a card, needs October sign-in; Pro and Max use the plan's Assistant credit, the free plan gets 5 questions a day, Lantern Lifetime and the Pro trial up to 20 a day or your own AI key) or **Send to @agent**. Exact wording and privacy: the product guide's Point & Ask entries.
- **Ask Lantern** (0.5+): click the lantern; the box at the bottom of Waiting. Ask anything and October's AI answers, knowing what your agents are doing ("which agent needs me first?"). Ask it to do something and it starts an agent on it after a 3-second Starting… Cancel card.

## Don't claim

- Automatic routing ("say it once and it picks the agent") doesn't exist yet. Screenshots go with new sessions and Point & Ask, not with ordinary replies.
- Don't advertise October Bus, multiplayer, the phone app, October Desktop pairing, the toolkit list or app Tasks in launch copy. Lead with Claude Code and Codex: see who's waiting, read, reply; Point & Ask and Ask Lantern as the extras.
- October connections: describe them exactly as in the product guide's "What's true today" table. Full October Desktop support needs October 1.0.52+. The phone connection is built but has not been tested against the real phone app: don't advertise it until a live pairing and reply have been confirmed.
- Linux. Windows only as a preview, with the Mac's features except: replies reach agents in their own console (Windows Terminal, VS Code, console windows) and sessions Lantern starts; Open brings the window forward but not the exact tab.
