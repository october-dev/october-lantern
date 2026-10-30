# Lantern product and development guide

**A small glowing lantern on the edge of your screen that watches every AI coding agent on your Mac and lights up when one of them needs you.**

**Platform scope:** the features below are the Mac app's (Swift). Linux and Windows run the same
design and behaviour in Rust (`desktop/`, egui): the edge pill and its panel, Waiting and Your
agents, chats and replies, New Session, Ask Lantern, Point & Ask, the October panel, Settings,
Welcome, the tray menu, global shortcuts, notifications and local dictation, with the shared Rust
engine. Where a platform can't do what the Mac does it does the closest thing and says so (no
backdrop blur on GNOME; the pill runs through XWayland on Wayland; Point & Ask selects an area or
the whole screen there). Windows is a preview. See the [build instructions](BUILDING.md).

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
- [Plans: free, the Pro trial and Lantern Lifetime](#plans-free-the-pro-trial-and-lantern-lifetime)
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
3. **One place to look.** The lantern glows amber with a count when agents finish or ask you something. The stack button shows **Waiting**: each agent, its project, how long it's been waiting and its last message.
4. **Read the whole conversation.** Click any agent to open its chat: what you asked, what it answered, and a one-line summary of each command or tool it ran.
5. **Answer without switching windows.** Type or dictate a reply and Lantern types it into the agent's own terminal tab: in **cmux**, **Terminal**, **iTerm2** and **tmux**. Right before typing, it checks that the terminal still belongs to the agent: the same process, still running the agent (not a shell it left behind), on that terminal, in the foreground. When it can't tell, it doesn't type. Questions are answered right from Lantern, on a card with the terminal's own numbered options (click one, or select Use number keys before pressing its number): a permission prompt (**Yes**, Claude Code's "don't ask again" option when it offers one, **No**), Claude Code's own multiple-choice questions (each option with its description; pick-many questions get tick boxes and **Send answers**; **Don't answer** closes the question) and its plan (**Yes, start on it** or **No, keep planning**). The card says **Sent** once it's answered. Lantern only presses keys if that question is still the one on screen.
6. **Notifies you.** An optional macOS notification when an agent finishes or needs you, even when the lantern is out of sight. Clicking it opens that agent's conversation.
7. **Starts new sessions.** Pick any installed agent, a folder and an optional first message, and Lantern runs it in Lantern (in the background, with its chat in the panel) or opens it in a new Terminal window.
8. **Answers questions and starts work.** Click the lantern for **Ask Lantern**: ask anything ("which agent needs me first?") and October's AI answers, knowing what your agents are doing. Ask it to do something ("add a dark mode toggle in october-lantern") and it answers in a line, then starts an agent on it right away, in the conversation.
9. **Stays out of your way.** Collapsed, Lantern is one small lantern on the edge of the screen. It never takes keyboard focus from the app you're using, and it appears on every desktop Space and over full-screen apps.

Lantern does **not** replace your terminals. Your agents keep running exactly where you started them, and you can keep using them there as usual. Lantern is an extra view and remote control on top.

## What it looks like

**The lantern (collapsed).** A small glass circle on the right (or left) edge of the screen containing the Lantern logo, a little orange lantern with eyes.
- **Dim:** nothing needs you.
- **Soft glow:** agents are working.
- **Amber glow with a number:** that many agents have finished or are asking you something since you last looked.
- **Amber glow, no number:** agents are still waiting, but you've already seen them.
- **Agent colours** everywhere in Lantern: **amber** needs you, **blue** working, **green** done (its turn is over), grey idle.

**Clicking the lantern** opens **Ask Lantern**: ask anything or tell it what to do (clicking it again closes the panel). Hovering shows five buttons, top to bottom: Ask Lantern (sparkles), Point & Ask, Start a new agent (+), your agents' cards (Waiting, then all agents), and **Connections** (computer and phone), which opens October account, phone and October Desktop setup. Right-click the lantern for everything else: Ask Lantern, Point & Ask, Start New Agent, Your Agents, Dictate, October, Settings, Quit (also in the menu bar icon, which a full menu bar can hide; on Windows and Linux, in the tray menu).

**Expanded.** Hover over the lantern and it grows downward into a glass capsule with five buttons: **Ask Lantern**, **Point & Ask**, **Start a new agent (+)**, **Your agents** and **Connections**. Connections opens the existing October panel; its button stays highlighted while that panel is open. The pill tucks back in when the mouse leaves. Drag the lantern to move it; it snaps to the nearest screen edge. On the Mac one Lantern runs at a time: opening it again (from Applications or Spotlight) brings up Ask Lantern in the one that's running. If a panel Lantern thinks is open has gone from the screen, clicking its button shows it again rather than closing it.

**The panel** opens beside the lantern:
- **Ask Lantern** (click the lantern): questions get an answer in the panel (follow-ups work; "‹ New question" starts over). Requests for work get a one-line answer and the agent starts right away, with your request as its first message, in your own words (a follow-up like "yes, do that" sends October's self-contained version). It runs in the background (through tmux) and **stays in the conversation**: a card shows the agent, its folder and state, the question card when it asks, and its replies and tool steps as they come; Open shows its terminal. If no agent can start, the conversation says why instead (and **Try again** starts it again after a failure). The first time October is used, the card says it's being set up. A Claude Code or Codex that has never been signed in comes after the others; when it's used anyway, it opens in a Terminal window, where it asks you to sign in. **Images:** paste one (⌘V, or Ctrl+V on Windows and Linux: a copied screenshot, a picture copied in a browser, or image files copied in Finder, Explorer or Files; copied text stays text), drop it on the box, or pick it with the paperclip (on a Linux desktop without X11, the paperclip and dropping only); up to 6 on a message, shown as previews you can remove. October's answer receives up to six pictures, with this message's pictures first and recent conversation pictures filling the remaining places. An unreadable picture stops the question instead of being omitted; an agent the conversation starts gets them all (their paths in its first message, attached for Codex), and a reply to a running agent gives it their paths. They're saved as private files in Lantern's screenshots folder (cleared after a week). When you're out of free questions or October AI credit, the answer says so (with the server's own daily number) and offers **Start free trial** and **Buy Lifetime**, which open Stripe checkout through `lantern.october.dev/buy?offer=trial&ref=app` and `…?offer=lifetime&ref=app`, with your October email filled in (in Point & Ask and Show me too). With your own AI key saved (Settings › October), your provider answers instead and a small **via OpenAI** (or Anthropic, OpenRouter) shows by the answer; if your key is refused the answer offers **Edit key**, if the model is wrong **Open Settings**, and if the provider is rate-limited, out of credit or down (before any answer came back) **Ask October's AI instead**, which asks the same question once more with October's AI. Lantern never switches on its own. Work for an agent that's already running goes to it instead of a new one, when you name it ("tell claude-1 to add tests") or the request clearly continues what it's doing: the answer says "Sending this to @claude-1", and that agent's card joins the conversation with what it says from then on. An agent Lantern started on October Bus gets it through the Bus, from Lantern's own Bus identity ("You (from Lantern)", linked to it for the message), and a one-line nudge in its terminal to read its inbox, since an agent waiting for you never checks on its own; any other agent has it typed in, like a reply. Lantern types it only when the Bus confirms nothing was accepted. An uncertain Bus outcome stays uncertain; retrying uses the same message identity to avoid duplication. While it's running, the box replies to it ("Reply to @claude-1…"); **Ask Lantern instead** sends the next message to Lantern. The Mac app brings its own tmux (used only when the Mac has none: macOS doesn't come with it). Without tmux (Linux) it opens in a Terminal window. Conversations are saved on this Mac (the most recent 50, in `ask-lantern.json` in Lantern's support folder; a conversation that can't be read is left out rather than losing the rest, and the file is first kept as `ask-lantern-unreadable-….json`): Ask Lantern's home lists the latest under **Recent** (Show all; right-click to delete), and opening one brings back its thread, with its agent live again if it's still running. **New chat** starts a fresh one. The agent is the one you named, else an agent signed in to a plan you already pay for (Claude Code on a Claude plan, Codex with a ChatGPT plan, Grok with an x.ai account, Gemini CLI with a Google account, in that order), so your own subscription is used before October's; else October Harness, then Claude Code, then Codex. Lantern tells which ones are signed in from each agent's saved sign-in (`~/.claude.json`, `~/.codex/auth.json`, `~/.grok/auth.json`, `~/.gemini/oauth_creds.json`), reading no secrets and running nothing; the folder is the project you named (matched against recent folders and agents' folders), else your home folder. Each Ask request carries the question, relevant conversation history, running-agent context and any included pictures. Show me has its additional screen/context data path described below. Needs October sign-in (the same allowance as Point & Ask). Signed out, Ask Lantern shows the sign-in buttons (Google, GitHub, Apple, or email and password) in place of the box; after the browser step, the browser shows "You're signed in to October Lantern", and Lantern comes to the front on Ask Lantern with "Signed in as …" and your question still in the box. On Linux and Windows the task runs in Lantern's own session (tmux on Linux, ConPTY on Windows) and conversations are saved in `ask-lantern.json` in Lantern's data folder (`~/.local/share/october-lantern`, `%LOCALAPPDATA%\October Lantern`).
- **Cards** (Claude Code, Codex and October that Lantern starts): the agent can ask you a question (its options, or answer in your own words), hand you a step only you can do (**Your turn: Sign in to …**, with Lantern's own line saying why it's yours, the agent's reason as its own, up to three steps, **Open <site>** which never opens by itself, then **I've done it** or **I can't right now** and an optional note), keep you posted (a status line and a checklist of steps), and finish with a result (what it says it did, marked **Says it's done**, **Partly done** or **Didn't work** as reported by the agent, with its files: documents **Open**, everything **Show**s in Finder, Explorer or Files, and apps and scripts are never run from there). A waiting card puts its agent at the top of Waiting, can't be dismissed, and notifies once per card. The card says **Sending…**, **Sent** once the agent's call took the answer, **Not sure the agent got it** if it stopped waiting first, or why it closed (nobody answered in 25 minutes, the agent stopped waiting, or it cancelled). Cards show on the agent's Waiting card, at the top of its conversation, and in Ask Lantern's task; after the agent exits, its questions and result stay in its task and under **Finished** in Your Agents (bounded recent history; live launches and unresolved questions/answer receipts stay visible beyond those limits).
- **Waiting:** a card for each agent that finished or needs you since you last looked, showing its logo, handle (e.g. `@claude-2`), project, the terminal it runs in, how long ago, the session title and its last message. Cards have **Reply**, **Open** (jumps to the agent's own tab) and **Dismiss** buttons. An agent's question gets the question card: a permission prompt ("Permission to run Bash · npm test", with **Show full request** for every line of the command), Claude Code's own multiple-choice question, or its plan. Select **Use number keys** on the visible card to focus its exact prompt/revision. Text editors, repeated presses and replacement prompts cannot answer it. Below the cards, an **Earlier** list holds agents that are still waiting but that you've seen, with **Clear all**.
- **Agents:** every running agent, the ones that need you first, with logo, handle, folder, terminal and status (*Needs you*, *Your turn*, *Working*, *Idle*, *Running*).
- **Chat:** click any agent to slide into its conversation. Your messages appear on the right in amber bubbles, the agent's replies on the left in glass bubbles, and each command it ran as a one-line entry ("Run · npm test"). It updates live.
- **Composer** at the bottom: "To @claude-2", a message field, a mic button and a send button.
- **Start a new agent:** an optional first message, **Choose your agent** (the agents installed on this Mac, with "…" for the rest and links to get the ones Lantern works with), a **Model** menu (Default, the models the agent offers, a search for long lists such as October's, or any model id you type; the last choice per agent is remembered), **Folder it works in** (recent folders, folders agents are running in, or "Choose Folder…"), **Context** (include a screenshot of your screen, with a preview and Retake), and **Start**. It opens in Lantern; the small *Opens in Lantern* menu under Start switches to a new Terminal window instead (remembered). Without tmux it opens in a Terminal window. In Lantern, the panel shows the session's chat straight away: your message, then "Starting…" until the agent is up, when its real conversation takes over. If it can't start, the chat says why and offers **Try in Terminal**.
- **Task:** the same form, when you click **+** while you're in DaVinci Resolve, Preview, Keynote or any app other than a terminal: the panel shows that app (and, with Accessibility allowed, its window and open document), asks "What should it do in DaVinci Resolve?", starts in the document's folder (or where the last task for that app ran), and turns the screenshot on. The agent's first message says which app, window and document you're in, and includes the toolkit list. ✕ on the app's card turns it back into a plain session.
- **Point & Ask:** press ⌃⌥P (Settings › General can change it), or the pill's selection button (a dotted box with a pointer), or Point & Ask in the menu bar menu. The screen dims and you drag a box around what you mean, like taking a screenshot of an area; moving without dragging outlines the window under the pointer and a click picks it, Return picks the whole screen, Esc cancels. A card beside the selection shows the screenshot (Lantern's own windows left out), the selected window's app and title when it can identify one. A selection spanning windows has no app caption. Windows and Linux send the selected picture without attaching foreground-app metadata. Say or type your question: holding the shortcut listens while you select, a quick tap keeps listening until you press it again, and the card has its own mic. Reselect picks a different area. Then either:
  - **Ask** (Enter when signed in to October): October's AI answers right in the card, streaming, and you can ask follow-ups. It's one model call through October, not an agent session: Pro and Max use the plan's Assistant credit, the free plan gets 5 questions a day (the card says when few are left), and Lantern Lifetime and the Pro trial get up to 20 a day, or your own AI key with no Lantern daily limit. Copy takes the answer. Ask needs October sign-in; signed out, the card offers Sign in to Ask. When you ask it to **do** something with what you selected ("put these images in a folder", "uninstall this app", "fill in this form from my resume"), it answers "Starting an agent to …", the card closes and **Ask Lantern** opens with that conversation and the usual **Starting <agent>… Cancel / Start** card; the agent gets the screenshot and October's description of what it shows. The app in front only says where you were: it never changes what's done.
  - **Send to @agent**: hands your words, the screenshot and the context to a coding agent, like a reply (with the card's last answer, if you asked first). On Mac it chooses an agent whose project is named in the selected window, or one hosted by that window's app, then the agent you last messaged. Windows and Linux use the last agent you messaged, then the first live one. The arrow picks another. The screenshot is saved in the agent's project under `.lantern/shots/`, which Lantern adds to the repository's local exclude list (`.git/info/exclude`, never `.gitignore`), so it isn't committed; the agent gets its path with your words.
- **Using your apps (Cua Driver, on by default):** agents Lantern starts can see and work the computer's apps (click, type, scroll, menus, window screenshots, your signed-in browser) in the background, with their own cursor, without taking over your mouse or bringing the app forward. It uses the open-source [Cua Driver](https://github.com/trycua/cua) (MIT). Lantern downloads a pinned release the first time it's on (about 52 MB on Mac, 35 MB on Windows, checked against its SHA-256), keeps only its programs in `cua-driver/<version>/` in Lantern's support folder, and runs one driver of its own on a private socket (`cua-driver.sock`; a per-user named pipe on Windows) while Lantern runs. One copy of Lantern on a computer runs the driver at a time: a second copy (for example a second install) says another copy is using it rather than taking it over, and takes over within a few seconds when the first quits, as during an update. On macOS it runs embedded in Lantern, so it uses Lantern's Accessibility and Screen Recording permissions and never asks for its own. Claude Code, Codex, OpenCode, Gemini CLI, Qwen Code, GitHub Copilot and Goose get its tools per launch (the same way as October Bus, without touching your config files); October and other agents are told the `cua-driver call` command in their first message. Its telemetry is turned off. Settings › General can turn it off.
- **October Bus:** agents Lantern starts (Claude Code, Codex, OpenCode, Gemini CLI, Qwen Code, GitHub Copilot, Goose and October) are connected to the public [October Bus](https://github.com/october-dev/october-bus) and linked with each other, so they can see their peers, message them and share tasks. Their first message says so. Lantern downloads the Bus the first time it's needed (about 6 MB, a signed build checked against a pinned checksum), starts it when needed, and keeps its agents in one scope, `lantern`; nothing is sent off the Mac. The daemon's run file (its address) is kept in `bus/run` in Lantern's support folder, not the temporary folder macOS clears after a few days; a daemon of Lantern's that can no longer be reached is stopped and replaced. Settings › General can turn it off.
- **App sessions:** Claude Desktop (its Code tab), Cowork and the Codex app's sessions are listed too, labeled with their app. Running ones show live; ones from the last three days show as **Recent**, and **Open** opens the app. They can't be typed into from Lantern.
- **October:** three cards. **Connect to October** signs in (Google, GitHub, Apple or email) and then shows the account and plan, **Lantern Lifetime** or **Pro trial · ends <date>**, and the daily questions on October's AI; **Refresh** reads the plan again (after buying or starting the trial in the browser), and a free account gets **Start free trial** and **Buy Lifetime**. **Your own AI key** (in Settings › October) takes a provider (OpenAI, Anthropic or OpenRouter), the API key and an optional model, with **Remove key**. **Connect to October Desktop** shows whether October is running, and once you click **Connect** and allow Lantern in October (matching a 6-digit code), it's fully connected. **Phone → Pair** walks through installing the October iPhone beta, signing in with the same account, scanning a Camera-compatible QR and comparing the six-digit code with **Approve** / **Deny**. **Connect your phone** also opens it from an empty agent list (signed-out users see sign-in first). Paired phones show last-seen activity and **Remove**; failed server removal blocks access locally and offers **Retry removal**. Keep this computer awake with Lantern open for phone access and agent alerts.

**Welcome.** The first launch shows a short, four-page welcome (Back on every page after the first). It remembers its page, so quitting or reopening Lantern partway through (macOS asks to reopen it after Screen Recording is allowed) picks up there, in front of other windows:
1. What the lantern's states mean, and the agents Lantern found.
2. **Let Lantern see and use your screen** (Mac): a page of its own for the two permissions Lantern needs, each a card saying what it's for and what happens next. **Screen Recording** lets Point & Ask see the part of the screen you select and lets agents see the apps they work in, only when you or an agent asks: **Allow…** (macOS's prompt, else System Settings), then **Reopen Lantern** (macOS applies it after a reopen; you come back to this page). **Accessibility** lets agents use your apps for you (click, type, read windows, in the background with their own cursor) and lets Lantern see which window you're in: **Allow…**, then **Open Settings…**, and Lantern restarts its driver as soon as it's allowed. Continue stays off until both are allowed; **Skip for now** goes on without them. Until both are allowed, the page comes back by itself on each launch (also for Macs set up before it existed), unless you chose Skip for now; Settings keeps the controls. On Windows and Linux nothing needs granting, so this page is **Let agents use your apps**: what it does, how it works (in the background with their own cursor, only for something you asked, Cua Driver downloaded the first time, off anytime in Settings) and its switch (on).
3. Setup, all optional: open at login, notifications, exact status (hooks) and the shortcut.
4. Where to find everything.

**Manual agent starts can work without stopping to ask.** With Ask before agents act off, Lantern uses supported harness auto-approve modes (Claude Code bypasses its permission prompts, Codex runs without approvals or sandbox, Gemini and Qwen with `--yolo`, Copilot with all tools and paths allowed, Cursor with `--force`, Grok with `--always-approve`, Goose in auto mode), and Claude Code's "Do you trust this folder?" is answered for the folder it starts in. Tasks often run in the background, where nobody would see a question. **Ask before agents act** (Settings › General) omits those auto-approve flags; it is not an enforcement boundary or a guarantee that every action asks. Local routines also omit them: Claude keeps native permissions and Codex one-shot runs use the workspace-write sandbox. Agents don't run as administrator: when a step needs it, they run just that step through the system's own prompt (Windows' "Do you want to allow…", the Mac's password dialog) and say to look for it.

**Settings** (from the menu bar lantern) has four tabs:
- **General:** open at login, which screen edge, the two shortcuts and automatic updates; new sessions and tasks (screenshot, using your apps, ask before agents act, October Bus, the toolkit list); exact status (the hooks); notifications (on or off, and whether finished turns notify too).
- **Agents:** which agents count toward Waiting and notifications.
- **October:** the October account (plan, Lantern Lifetime or the Pro trial, Refresh), **Your own AI key** (provider, key, optional model, Remove key: "Your provider bills you for these questions. Lantern sends your key to October's server only to pass each question on; it's never stored.") and October Desktop.
- **About:** version, check for updates, usage counts (in builds that send them; linked to your October account when signed in), report a problem, send logs, the welcome guide, and uninstall.

**Menu bar.** A small lantern icon: Waiting, Agents, New Session or Task, Message an Agent, Show/Hide Lantern, Settings, Check for Updates, Report a Problem, Send Logs, Welcome Guide, Quit.

**Look and feel.** Dark, translucent glass (Apple's Liquid Glass on macOS 26, a frosted blur on earlier versions) with a light rim. Amber means "needs you", green means "working" and grey means "idle". Each agent is shown with its official logo.

## Local tasks, guidance and routines

See [Local task features](LOCAL-FEATURES.md) for cards, deliberate keyboard answers, app-access controls, local schedules, and Show me, including their privacy and control boundaries. These describe this source branch; installer release notes identify shipped versions. Show me draws a click-through orange ring, arrow and step label over the selected display on Mac, Windows and supported X11 desktops. Clicking the ring advances the guide after a moment; window changes hide it and trigger a fresh look. Reduce Motion keeps the cue still. Wayland and desktops without the required overlay/input capabilities show the marked screenshot instead, with an explanation in the card.

## A walkthrough

Say you have six sessions running in cmux: two Claude Code, two Codex, one OpenCode and one Pi.

1. **Open Lantern.** All six appear within a couple of seconds, each with its logo and a handle: `@claude-1`, `@claude-2`, `@codex-1`, `@codex-2`, `@opencode-1`, `@pi-1`. You don't register or restart anything.
2. **Keep working.** While they run, the lantern glows softly.
3. **An agent finishes.** When `@codex-1` finishes, the lantern turns amber with a **1**, and a notification says "@codex-1 finished · Tests pass. Want me to commit?".
4. **Read and reply.** Open **Waiting**, then the card, to see the whole conversation. Type or dictate "yes, commit it" and press Enter. Lantern types it into `@codex-1`'s own cmux tab, just as if you'd typed it there.
5. **A permission prompt** (with hooks on). `@claude-2` asks to run a command. Its card shows the command with **1 Yes**, **2 Always allow …** and **3 No**. Press 1 and it carries on. When Claude asks you something itself ("Which layout? Three tiers / One plan / A table"), the card shows those options; pick one and it goes on with your answer.
6. **Jump there.** **Open** brings `@opencode-1`'s own tab to the front if you'd rather work in the terminal.
7. **Start another one.** Click **+**, pick Codex and a folder, type "write tests for the parser", and keep *In Lantern*. Its chat opens in the panel straight away, and the agent shows up in Your agents.

## Supported agents

| Agent | Detected | Working / your turn | Last message and chat | Notes |
|---|---|---|---|---|
| Claude Code | ✅ | ✅ | ✅ | With hooks: instant updates, exact session matching, and permission prompts (with the full command) answered from Lantern. Its own questions and plans are answered from Lantern with or without hooks |
| Codex CLI | ✅ | ✅ | ✅ | With hooks: instant "finished" updates |
| OpenCode | ✅ | ✅ | ✅ | Reads OpenCode's local database |
| Pi | ✅ | ✅ | ✅ | |
| October harness (`october`) | ✅ | ✅ | ✅ | October's own agent, built on Pi. Built in: installed on first use when it isn't on your PATH, and the default agent when no other agent is signed in to your own plan |
| Gemini CLI | ✅ | ✅ | ✅ | |
| Muse (Meta) | ✅ | ✅ | ✅ | Exact session matching. When Muse asks to approve a tool, the card shows what it wants; answer in its terminal |
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

Lantern is in **beta (v0.6.16)**.

| Capability | Status |
|---|---|
| macOS app (macOS 14+, Apple silicon and Intel), signed and notarized | ✅ |
| Automatic updates | ✅ |
| Finds agents in any terminal | ✅ |
| Working / your-turn status, last message, chat view | ✅ Claude Code, Codex, OpenCode, Pi, October harness, Gemini CLI, Muse |
| Reply from Lantern | ✅ cmux, Terminal, iTerm2, tmux. Other terminals: copy and paste. |
| Allow / Deny permission prompts | ✅ Claude Code with hooks on, in cmux, iTerm2 and tmux. The card shows the tool and the full command being approved. The key is only pressed if that prompt is still the one on screen. |
| Cards: an agent Lantern started asks you a question or hands you a step, shows its progress and its result | ✅ Claude Code, Codex and October started from Lantern (with or without October Bus and Cua Driver). An answer goes only to a question the agent is still waiting on (up to 25 minutes); otherwise the card says it closed, and nothing is typed in later. Proven end to end with Claude Code and Codex on macOS; October's own model turn and Windows/Linux not yet. |
| Jump to the agent's exact tab | ✅ cmux, Terminal, iTerm2, tmux |
| Notifications | ✅ Optional |
| Start new sessions (Terminal window or background) | ✅ Background runs in tmux: yours, or the one Lantern brings |
| October Harness built in | ✅ The default agent for assistant tasks when no agent is signed in to a plan you already pay for (Claude Code, Codex, Grok or Gemini go first; falls back to Claude Code, then Codex, when October can't start). Your own `october` is used when it's on your PATH; otherwise Lantern installs the pinned version on first use (not in the DMG; a few minutes and about 370 MB on disk, less when October Desktop's Node.js can be reused). Signs in with your October account when you're signed in to Lantern. |
| Dictation, on-device only | ✅ When the Mac supports on-device recognition for your language; otherwise Lantern says so instead of sending audio to Apple |
| Global shortcut (default ⌃⌥Space, changeable) | ✅ |
| Welcome guide, Settings, Report a Problem, Send Logs, Uninstall | ✅ |
| Automatic routing ("send this to whichever agent it's for") | ❌ Not yet. You choose the agent. |
| Screenshot as context for a new session | ✅ Optional (New Session › Context, or Settings › General). Needs Screen Recording permission. |
| Point & Ask (drag a box, then Ask October's AI or send to an agent) | ✅ Ask needs October sign-in (free plan: 5 questions a day; Pro and Max: the plan's Assistant credit; Lantern Lifetime and the Pro trial: up to 20 a day, or your own AI key). Needs Screen Recording permission. |
| Ask Lantern (answers, and starts agents for requests) | ✅ Needs October sign-in, same allowance as Point & Ask. Tasks start right away. Images can be pasted, dropped or picked. |
| Persistent agent memory | Not part of this implementation; saved chats and preferences are separate local features. |
| Sign in with an October account (Google, GitHub, Apple, email) | ✅ |
| Connect to October Desktop | ✅ Read-only with any October version that runs october-core (lists October's terminals). Full connection (October's agent names and questions, replies through October's safe delivery, Open on the canvas) needs October 1.0.52 or later, after the user clicks Connect and allows Lantern in October. |
| October phone app | October → Phone → Pair immediately shows a pairing QR on Mac, Windows and Linux, with a TestFlight install link alongside it. Includes same-account guidance, live QR expiry, link copying, comparison, plan/service recovery and removal. Needs mobile access on the account. TestFlight build 37+ is the phone contract; Camera universal-link delivery, the website install fallback, physical iPhone pairing/reply and failure-path acceptance must be recorded before release. |
| Linux | ✅ The Mac app's pill and panel in Rust (egui) with the Rust engine: Waiting/Agents, chats, New Session (In Lantern via tmux), Ask Lantern, Point & Ask, October account and Desktop, Settings, tray menu (with an AppIndicator host), Ctrl+Alt+Space / Ctrl+Alt+P, notifications, local dictation (with an installed whisper.cpp; the model downloads on first use, after you agree), October Harness, updates. GNOME/Wayland via XWayland; no backdrop blur. |
| Windows | ⚠️ Preview with a downloadable per-user installer, with the same pill and panel as Linux. Finds agents in Windows Terminal, VS Code and console windows, shows state and conversations, brings the terminal window to the front; tray menu, notifications, Ctrl+Alt+Space / Ctrl+Alt+P, hooks, Credential Manager sign-in. Starts sessions in Lantern's own ConPTY sessions (shown in Windows Terminal or a console window) and replies and answers questions there; agents started elsewhere are followed, not typed into. October Bus, screenshots of the display under the pointer, local dictation (whisper.cpp shipped with the app; its model downloads on first use, after you agree), in-app updates, and a per-user installer. Built and tested in CI; not yet tried by hand on Windows. |
| Pricing | The core app is free, with no account needed for local monitoring/replies. Ask Lantern and Point & Ask’s AI answers need October sign-in and use the account’s allowance or credits. |

## Plans: free, the Pro trial and Lantern Lifetime

- **Free:** everything that runs on your computer, plus Ask, Point & Ask and Show me with October sign-in, 5 questions a day on October's AI.
- **Pro trial:** 3 days free with a card up front, then $9.99 for the first month, then $20 a month (October Pro). During the trial you get what Lantern Lifetime gets; cancel in one click in the billing portal before the charge date shown at checkout.
- **Lantern Lifetime:** one payment, in tiers of 100 seats at $149, then $199, then $249. It includes Ask, Point & Ask and Show me with your own AI key and no Lantern daily limit (your provider bills you), up to 20 questions a day on October's AI, the iPhone app (beta), and every Lantern app update. It doesn't include October Cloud or monthly October credits. Full refund on request within 14 days (hey@october.dev).
- **Start free trial** and **Buy Lifetime** open Stripe checkout directly (through `lantern.october.dev/buy`, with `ref=app`), with the signed-in October account's email filled in so the purchase lands on that account; afterwards **Refresh** on the account card shows it without restarting.

## Privacy

- **Local monitoring and replies do not require an October account.** Session discovery and history reading use local files. Network use also includes update checks, enabled usage analytics and component/model downloads (October Harness/Node.js, Cua Driver, Bus and speech models, as applicable). Agents can contact their own model providers and websites, including when computer access is enabled. Ask and optional October connections have the additional data paths below.
- **Cards** are kept on this computer only, in `cards/cards.db` in Lantern's support folder (private to you), with each session's key in `cards/keys`: up to 200 completed sessions, plus live sessions and unresolved cards/receipts, with their questions, your answers, steps and results. Card answers go to the harness that asked, which can send them to its model provider. The UI shows bounded recent completed history and keeps live work and unresolved cards visible.
- **Usage analytics (on by default, off in Settings › About or on the welcome screen).** Sent to PostHog (US), a few batched requests a day, so we can count daily and monthly users and see which features are used. Usage events carry their name, an install or signed-in account identifier, the app version, the system (macOS version, or Windows or Linux) and the chip type, plus these details. Sign-in identification also sends the account email, as described below:
  - `lantern_active`, once a day: how many agents of each kind are running (e.g. 3 Claude Code, 1 Codex).
  - `reply_sent`, `reply_failed`, `reply_uncertain`, `reply_copied`: the agent's kind and how the reply was typed (tmux, cmux, Terminal…).
  - `permission_answered`: allow or deny, and the route.
  - `session_started`, `session_start_failed`: the agent's kind, the model id chosen (or "default"), whether it started in the background, whether a screenshot was included, and for a Task the app's bundle id (e.g. `com.blackmagic-design.DaVinciResolve`), never the document or window title.
  - `chat_opened`, `agent_opened`, `dictation_started`, `hooks_changed` (on or off).
  - `card_answered`: whether it was a question or a hand-off (never what it said or your answer).
  - `october_signed_in`, `october_signed_out`, `october_desktop_connected`, `phone_paired`, `engine_failed`.
  - `assistant_asked`, `point_ask_asked` (whether it was a follow-up), `assistant_task` (the agent's kind), `upgrade_opened` (from Ask Lantern, Point & Ask, Show me, the account card or the own-key card, and which offer: trial or lifetime).
  - `own_key_saved` (the provider: OpenAI, Anthropic or OpenRouter; never the key or model), `own_key_removed`, `own_key_fallback` (Ask October's AI instead was chosen).

  The Windows and Linux builds send `lantern_active`, `assistant_asked`, `assistant_task`, `point_ask_asked`, `session_started` and `session_start_failed` (whether it was an Ask Lantern task), `upgrade_opened`, `own_key_saved`, `own_key_removed`, `own_key_fallback`, `october_signed_in` and `october_signed_out`.

  Never sent: messages, prompts, commands, folder or project names, file paths, agent titles, or anything typed or dictated. PostHog derives an approximate location (country and city) from the IP address the counts arrive from, to show where Lantern is used; it isn't precise and nothing else is used for it. If you sign in to October, your usage is linked to your October account (its id and email). Turning the switch off stops sending and drops anything not yet sent. Builds without a configured PostHog key send no analytics; this is the default for source builds.
- **October account (optional).** Signing in talks to October's sign-in service (Supabase) and reads your plan from october.dev. The session is kept in the macOS Keychain (on Windows, Credential Manager; on Linux, the Secret Service).
- **Your own AI key (optional).** The provider, key and optional model are kept only in the system's credential store, in their own entry, never with the October sign-in: the macOS Keychain (service `dev.october.lantern.ai-key`, account `own-key`), Windows Credential Manager (`dev.october.lantern/ai-key`) or the Linux Secret Service (`application dev.october.lantern`, `credential ai-key`). If that store is locked or missing, Lantern says so and doesn't use or save the key; it never writes it to a file. Each Ask, Point & Ask or Show me question sends the key to October's server (`www.october.dev/api/lantern/ask`, over HTTPS, in request headers), which uses it to pass that one question to your provider and doesn't store, log or echo it; your provider then receives the same question, pictures and context October's AI would. The key is never in Lantern's settings, logs, Send Logs, problem or crash reports, or usage counts. Signing out keeps it; **Remove key** and Uninstall delete it.
- **Unix agent startup.** Lantern reads the selected zsh, bash or fish login environment once per engine lifetime, with no input or controlling terminal, a three-second deadline and a 256 KiB output cap. Agent discovery, model/tool lists and launches share that PATH; executable files count as installations, aliases and shell functions do not. Slow, broken or unsupported shells use the sanitized inherited environment plus normal install folders and leave a diagnostic in the engine log. Restart Lantern to reload changed shell exports. Each launch passes separate arguments to tmux, bypassing its default shell. Without tmux, a clean POSIX reader runs in a visible terminal. Windows keeps its direct ConPTY launch path.
- **October Harness (downloaded on first use).** On a Mac without Claude Code or Codex (or with one that has never been signed in), Lantern installs it in the background right away (about 170 MB, plus Node.js if the Mac has none) and makes it the default agent; Macs with either one download nothing until you pick October. When you start an October session and `october` isn't installed, Lantern downloads it the first time: Node.js from nodejs.org (checked against a pinned SHA-256; skipped when October Desktop already has a compatible one) and the pinned `@october-dev/october` package from the npm registry, installed without running install scripts into `~/Library/Application Support/October Lantern/` (`runtime/` and `harness/`). If you're signed in to October, October sessions get your current October access token (not the refresh token) so the harness uses your October account; the engine keeps it in memory only, and it's passed to the agent's process environment like October Desktop does. Signed out, the harness asks you to sign in itself. To keep a long session signed in, the harness asks `lantern-engine october-token` for a fresh token (`OCTOBER_TOKEN_COMMAND`); Lantern keeps the current access token for that in `~/Library/Application Support/October Lantern/october-token.json` (only you can read it; access token only, never the refresh token; deleted when you sign out).
- **October Desktop (optional).** Lantern talks to October only on your Mac (127.0.0.1), and only after you allow it in October. You can disconnect it from either app.
- **October phone app (optional).** Traffic goes through October's relay and is end-to-end encrypted (Noise), so the relay can't read it. Phones are paired by QR and can be revoked from Lantern. If the phone turns on notifications, Lantern sends a push through Expo's push service (exp.host, then Apple or Google) when an agent starts waiting for you: it names only the agent's handle (e.g. "@claude-1 needs you") and which agent to open, never what the agent said, code or file paths.
- **Dictation is on-device only.** Lantern uses Apple's on-device speech recognizer and never sends audio to Apple's servers: if on-device recognition isn't available for your language on this Mac, dictation says so rather than falling back. Lantern listens only while the mic button is on. On Windows and Linux, whisper.cpp transcribes on the computer; the first time you press the mic, Lantern asks before downloading anything ("Turn on dictation?"): **Download** fetches its speech model (148 MB, once) from Hugging Face, checked against a pinned SHA-256 (and, on a Windows copy updated in the app, its 1 MB speech program from Lantern's releases); **Not now** downloads nothing, and the next press asks again. No audio is sent anywhere.
- **The toolkit list.** Lantern keeps a short list of what this Mac has in `~/Library/Application Support/October Lantern/toolkit.md`: the chip and memory, notable command-line tools, local AI models (Ollama, whisper, Hugging Face and LM Studio caches), and apps agents can script. It's rebuilt in the background when it's a day old (or from Settings), never sent anywhere by Lantern, and given to agents you start in a Task (and in new sessions if you turn that on). Anything you write under its "Your notes" heading is kept.
- **Tasks.** A Task tells the agent which app you're in, its front window's title and its open document's path. Reading the window and document uses macOS's Accessibility permission, asked only when you click Allow on the Task panel.
- **Point & Ask** captures only after you start it: the area you select and, on Mac when one window contains it without another window overlapping, that window's app and title. It does not attach another app's selected text, document path or browser URL. Windows and Linux use the selected picture without automatic app metadata. **Send to @agent** saves the screenshot in the agent's project (`.lantern/shots/`, excluded from git) and gives it to that agent. **Ask** sends your question, the screenshot (as a JPEG, at most 1600 pixels wide) and that same context to October (`www.october.dev/api/lantern/ask`), signed in with your October account; October picks the model (on Pro and Max a stronger one, paid from the plan's monthly Assistant credit; on the free plan a lighter one, up to 5 questions a day) and the model provider answers it. Nothing is sent until you press Ask, and Lantern keeps no copy of the answer after the card closes.
- **The assistant's Ask** sends your question, earlier questions and answers in that chat, and a short summary of your running agents (handle, agent, project, state, session title, the question it's asking and the start of its last message) to the same October endpoint. Nothing is sent until you submit the question. Ask conversations are saved locally, including text and image references, and can be reopened after quitting Lantern. A task it starts runs on your computer like any New Session; the selected agent can send its input to its own model provider.
- **Agents using your apps.** With "Let agents use this Mac's apps" on (the default), agents Lantern starts can look at and operate your apps' windows, including your signed-in browser, through Cua Driver running on this computer. Screenshots and what agents read stay on the computer except what the agent itself sends to its own AI provider, as with anything else it reads. Cua Driver's own anonymous usage reporting is turned off (`cua-driver telemetry disable`, and `DO_NOT_TRACK=1` for Lantern's driver). Turn it off in Settings › General.
- **New-session screenshots.** The optional "Include a screenshot of my screen" setting adds a screenshot to a new session. Point & Ask and agents with computer access have the separate capture paths described above. It captures the display you're on when you open New Session, leaving out Lantern's own windows, and shows you a preview. When you click Start, it's saved on your Mac (`~/Library/Application Support/October Lantern/screenshots`, only readable by you, deleted after a week), and the new agent is given its path with the first message: Codex gets it attached (`--image`), and Claude Code and Gemini CLI are allowed to read that folder. Lantern itself never uploads it; the agent sends it to its model provider like any image you give it. It's never recorded continuously.
- **Read-only by default.** Lantern reads the process list and the agents' own session files (`~/.claude/projects`, `~/.codex/sessions`, OpenCode's database, `~/.pi` / `~/.october` sessions, `~/.gemini`), and never modifies them.
- **Typing replies.** When you send a reply, Lantern types it into that agent's terminal: through cmux's command-line tool, tmux, or macOS Automation for Terminal and iTerm2. macOS asks you once per app before Automation is allowed.
- **Optional hooks.** If you turn on exact status, Lantern adds itself to Claude Code's hooks (`~/.claude/settings.json`: PermissionRequest, Notification, Stop, UserPromptSubmit, PostToolUse, PostToolUseFailure) and Codex's `notify` setting (`~/.codex/config.toml`). It reads and checks both files before changing either, backs both up (never overwriting an earlier backup), replaces each atomically, puts the first file back if the second can't be written, keeps any existing Codex notify program working, and restores everything when you turn hooks off or uninstall. Settings shows Claude Code and Codex separately, and offers an update when an older Lantern's Claude hooks are installed. If Lantern is deleted without uninstalling, the hooks quietly do nothing. The hook events Lantern stores (`~/Library/Application Support/October Lantern/events`) hold the agent's last message and the command it asked permission for.
- **Problem and crash reports are only ever sent by you**, as an email you see before sending. They contain versions and a summary of which agents and terminals are running, never conversations, folders or code.
- **Send Logs** (Settings › About, and the menu) sends a report only when you choose it: Lantern's version, the system's, what Lantern shows (engine state, agent counts by kind, each agent's id, state and session), Ask Lantern's last five tasks (which agent, when, its session, when Lantern first saw it and what the conversation shows; never the message or the folder), what's there right now (the agents Lantern can start and whether they're signed in, Lantern's own tmux sessions with how long they've run and what they're running, and each agent's latest hook state) and the tails of Lantern's own logs (the app's, its engine, October Bus and Cua Driver; on the Mac, Lantern's crash reports from the last week, and how many saved chats loaded). The app log notes each Ask Lantern start, how it went, when its agent appeared, and, if it hasn't a minute on, which sessions Lantern could see; the engine log notes each launch and its result and agents appearing and going. Passwords, keys and tokens are taken out, your home folder's path becomes `~` and your user name `user`. It goes to the October team's private store (the same one October Desktop's reports use), where only the team can read it, and you get a code (LAN-…) to quote. If it can't be sent, it's saved to your Desktop instead, to send however you like.

## Requirements and permissions

- macOS 14 Sonoma or later (the Liquid Glass look needs macOS 26 Tahoe)
- At least one supported agent

| Permission | Why | When it's asked |
|---|---|---|
| Automation (Terminal, iTerm2) | Typing replies into the agent's tab | The first time you reply to an agent in that app |
| Notifications | Telling you when an agent finishes or needs you | In the welcome, or when you turn them on |
| Microphone and Speech Recognition | Dictating a reply | The first time you press the mic button, after Lantern asks "Turn on dictation?" (Turn On / Not Now; Not Now asks again next time). Holding the Point & Ask shortcut doesn't ask: it listens only once dictation is on. |

| Screen Recording | Point & Ask, including a screenshot when starting a session, and agents seeing your apps' windows (Cua Driver) | In the welcome (required there unless you skip), or the first time you use Point & Ask |
| Accessibility | Letting agents use your apps (Cua Driver); telling a Task's agent which window and document you have open | In the welcome (required there unless you skip; asked once more on Macs that finished the welcome earlier), on the Task panel, or Allow… in Settings › General |

Lantern doesn't need Full Disk Access.

## Installing, updating and uninstalling

- **Install:** open the DMG and drag **October Lantern** to Applications. Open it; the welcome guide starts.
- **Update:** Lantern checks for updates once a day and asks before installing. *Check for Updates…* is in the menu bar lantern and in Settings › About. On Windows and Linux, the daily check installs a newer version on its own and restarts into it the next time Lantern is idle (no panel open, not dictating or selecting); agents running in Lantern keep running. Turn off *Check for updates automatically* in Settings to update only by hand.
- **Uninstall:** Settings › About › **Uninstall October Lantern…**. It removes your own AI key, the hooks (restoring your agents' settings), Lantern's support files in `~/Library/Application Support/October Lantern`, the login item and its settings, then moves the app to the Trash. Your agents and any sessions Lantern started keep running.

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
No. Everything runs locally. Without an October account, the only network requests are the update check and usage counts (feature counts, never content; off in Settings). If you connect a phone through October, replies and agent status travel end-to-end encrypted through October's relay. With the phone's notifications on, a short "@agent needs you" notification goes through Expo's push service (no agent output).

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
Yes. The core app is free and local monitoring/replies need no account. Ask Lantern and Point & Ask’s AI answers require October sign-in and use the account’s allowance or credits.

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
  4. reads each agent's session to decide *working* vs *your turn* and get the last message and title. It uses Claude Code transcripts (each entry's `stop_reason` and timestamp), Codex rollout files (found through the process's open files; turn events carry their own times), OpenCode's SQLite database (read-only), Pi and October harness JSONL, and Gemini CLI JSONL (these three matched by folder and start time), and Muse's session event log (found through the session lock the process holds open; a run's start and end mark the turn, and a tool approval with no decision is a question for you);
  5. merges hook events (from `lantern-engine hook ...`), using whichever is newer;
  6. sends the app a snapshot whenever something changed.

  It also serves conversation history, presses the keys that answer agents' questions, focuses tabs, and starts new sessions on Lantern's own tmux server (`tmux -L lantern`) with its captured exported environment, without running interactive shell startup in the agent pane. Replies, keys and focus run on one worker per terminal with a deadline, off the scan loop. Right before typing it re-checks the target (same pid and start time, same program, same tty, in the foreground, the route's pane or tab still on that tty) and refuses when it can't tell. A send that times out before it starts is canceled; one that started but can't be confirmed is reported as uncertain. It has no UI, so a Windows front end could reuse it.
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
- `--open inbox|agents|new|october|assistant|routines|chat|welcome:N|settings:N` opens a panel or window at launch.
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
| `desktop/` | The Linux and Windows app (Rust, egui): the Mac app's pill and panel; uses the shared engine over stdio. |
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
