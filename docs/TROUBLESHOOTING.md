# Troubleshooting

## Lantern shows no agents

**What you see:** The Agents list is empty, or an agent you installed isn't offered under **Start a new agent** (the **+** button).

**Why it happens:** Installing a terminal agent doesn't start it. Lantern ignores one-off runs such as `claude -p` and `codex exec`. **Start a new agent** needs an installed program Lantern can find; a shell alias or function is not enough.

**What to do:** Start a supported agent in a terminal and wait a few seconds. If an installed agent is missing under **Start a new agent**, check **… (More agents)** first. If it isn't there, check that its command is installed on your `PATH`, then quit and reopen Lantern. See [supported agents](PRODUCT.md#supported-agents).

## A reply didn't reach the agent

**What you see:** You replied in Lantern, but the agent didn't respond.

**Why it happens:** How Lantern sends replies depends on the terminal and platform. It can type directly only in supported terminals. In other cases it copies the reply for you to paste, or asks you to answer in the agent's terminal. Lantern also refuses to type if the agent or terminal has changed; an uncertain result means it couldn't confirm whether the message went in.

**What to do:**

- **Mac:** Direct replies work in cmux, Terminal, iTerm2, tmux and kitty. In other terminals, paste the copied reply. If macOS asks for Automation permission for Terminal or iTerm2, allow it; if you declined it earlier, turn it on in System Settings › Privacy & Security › Automation. For kitty, direct replies need remote control on in `kitty.conf` (`allow_remote_control socket-only` and `listen_on unix:/tmp/kitty`), then restart kitty; without that, Lantern copies the reply and you paste it.
- **Linux:** Direct replies work in tmux. In other terminals, paste the copied reply.
- **Windows:** Direct replies work for sessions started by Lantern and for agents running in their own console (Windows Terminal, VS Code or a console window). If Lantern says something else is running in the agent's terminal, wait for it to finish or answer in that terminal.
- If Lantern says it is unsure whether a reply went in, check the agent's terminal before trying again.

See [supported terminals](PRODUCT.md#supported-terminals) for details.

## Point & Ask or Show me asks for Screen Recording

**What you see:** On a Mac, Point & Ask or Show me says Lantern needs Screen Recording permission.

**Why it happens:** macOS requires this permission for Lantern to capture the screen for these features. Screen Recording is a Mac permission; Linux and Windows do not use this permission prompt.

**What to do:** Open System Settings › Privacy & Security › Screen Recording (named Screen & System Audio Recording on newer macOS) and turn on October Lantern. macOS applies the change when Lantern reopens: choose **Quit & Reopen** if macOS offers it, or quit and reopen Lantern yourself. Then try again.

See [permissions](PRODUCT.md#requirements-and-permissions).

## Linux: tmux is missing

**What you see:** Lantern cannot start an agent in the background, or cannot send a reply directly to an agent in another terminal.

**Why it happens:** On Linux, background sessions and direct replies to other terminals use tmux. Without tmux, Lantern can start some agents in a visible terminal window, but cannot run them in the background. Some agents also need tmux to receive a first message at launch.

**What to do:** Install tmux with your Linux distribution's package manager, then quit and reopen Lantern so it checks for tmux again. To receive direct replies, an agent you start yourself must run inside tmux. See the [Linux platform notes](BUILDING.md#linux).

## Linux: the tray icon isn't showing

**What you see:** The lantern is open, but its tray icon and menu are missing on GNOME.

**Why it happens:** GNOME needs an extension to display Lantern's tray icon and menu.

**What to do:** Install and turn on [AppIndicator and KStatusNotifierItem Support](https://extensions.gnome.org/extension/615/appindicator-support/). If GNOME doesn't list the extension after installation, log out and back in, then turn it on. Once it is enabled, allow about 10 seconds for Lantern's icon to appear. Until then, right-click the lantern for Settings and Quit.

See the [Linux desktop notes](BUILDING.md#desktop-and-terminal-integration).

## Windows: “Windows protected your PC” appears during installation

**What you see:** Windows shows “Windows protected your PC” when you open the installer.

**Why it happens:** Windows doesn't recognize the installer. Unsigned builds can trigger this warning, and newly signed builds can show it too. Check the [release notes](https://github.com/harshsaver/october-lantern-releases/releases) for the installer's signing status.

**What to do:** Make sure the installer came from the [official release page](https://github.com/harshsaver/october-lantern-releases/releases). For that installer, click **More info**, then **Run anyway**, as described in the release notes. If it came from elsewhere, download it again from the official page. If **Run anyway** is unavailable, contact your device administrator or [October support](mailto:hey@october.dev).

## Send logs to October support

**What you see:** Something isn't working and you want to tell October, or support asked for Lantern's logs.

**Why it helps:** The report gives the October team details about Lantern and its recent activity to help find the problem.

**What to do:** Open **Settings → About → Report a Problem**. Choosing it starts sending the report immediately. On success, Lantern copies its `LAN-…` code; send that code to [October support](mailto:hey@october.dev) with a line about what happened. If sending fails, Lantern tries to save the report to your Desktop, or Downloads if Desktop is missing, or your home folder if both are missing. On Mac, it opens an email with the file attached when supported; otherwise it shows the file for you to attach manually. On Windows and Linux, it opens the folder and an email naming the file to attach.

The report includes Lantern's version, system details, an agent summary and recent logs. Session names can include project folder names. Lantern replaces your home folder's path with `~` and tries to remove passwords, keys and tokens. This is best effort; other personal or diagnostic details may remain. See the [privacy guide](PRODUCT.md#privacy) for more about reports.
