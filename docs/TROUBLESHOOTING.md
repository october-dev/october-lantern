# Troubleshooting

## Lantern shows no agents

**What you see:** The Agents list is empty, or an agent you installed isn't available in New Session.

**Why it happens:** The Agents list shows supported agents that are running interactively. Headless runs are ignored. New Session lists agents Lantern can start, not every agent that may be running.

**What to do:** Start a supported agent in a terminal to see its running session in Lantern. To start an agent from New Session, make sure it is installed. See [supported agents](PRODUCT.md#supported-agents).

## A reply didn't reach the agent

**What you see:** You replied in Lantern, but the agent didn't respond.

**Why it happens:** How Lantern sends replies depends on the terminal and platform. It can type directly only in supported terminals. In other cases it copies the reply for you to paste, or asks you to answer in the agent's terminal. Lantern also refuses to type if the agent or terminal has changed; an uncertain result means it couldn't confirm whether the message went in.

**What to do:**

- **Mac:** Direct replies work in cmux, Terminal, iTerm2 and tmux. In other terminals, paste the copied reply. If macOS asks for Automation permission for Terminal or iTerm2, allow it to type there.
- **Linux:** Direct replies work in tmux. In other terminals, paste the copied reply.
- **Windows:** Direct replies work for sessions started by Lantern. For agents started in other terminals, reply in that terminal.
- If Lantern says it is unsure whether a reply went in, check the agent's terminal before trying again.

See [supported terminals](PRODUCT.md#supported-terminals) for details.

## Point & Ask or Show me asks for Screen Recording

**What you see:** On a Mac, Point & Ask or Show me says Lantern needs Screen Recording permission.

**Why it happens:** macOS requires this permission for Lantern to capture the screen for these features. Screen Recording is a Mac permission; Linux and Windows do not use this permission prompt.

**What to do:** In macOS System Settings, allow October Lantern under Screen Recording. Reopen Lantern if Point & Ask asks you to, then try again. For Show me, allow access and try again.

See [permissions](PRODUCT.md#requirements-and-permissions).

## Linux: tmux is missing

**What you see:** Lantern cannot start an agent in the background, or cannot send a reply directly to an agent in another terminal.

**Why it happens:** On Linux, background sessions and direct replies to other terminals use tmux. Without tmux, Lantern can start some agents in a visible terminal window, but cannot run them in the background. Some agents also need tmux to receive a first message at launch.

**What to do:** Install tmux using your Linux distribution's package manager, then try again. See the [Linux platform notes](BUILDING.md#linux).

## Linux: the tray icon isn't showing

**What you see:** The lantern is open, but its icon or menu is missing from the GNOME system tray.

**Why it happens:** GNOME needs an AppIndicator host to show Lantern's tray menu.

**What to do:** Use a GNOME panel or tray host that provides AppIndicator support. This project doesn't document which host to install or how to set one up for each distribution. You can still right-click the lantern to find Quit.

See the [Linux desktop notes](BUILDING.md#desktop-and-terminal-integration).

## Windows: “Windows protected your PC” appears during installation

**What you see:** Windows shows “Windows protected your PC” when you open the installer.

**Why it happens:** October Lantern's release documentation says unsigned Windows builds trigger this warning. The warning alone does not tell you whether a particular installer is unsigned.

**What to do:** Get the installer from the [official release page](https://github.com/harshsaver/october-lantern-releases/releases). If Windows still shows the warning, check the release information or contact [October support](mailto:hey@october.dev) before continuing. Do not bypass the warning based only on this guide.

See [Windows release signing](RELEASING.md#windows-code-signing-azure-artifact-signing).

## Send logs to October support

**What you see:** Support asks for Lantern logs, or Lantern reports a problem.

**Why it happens:** The report combines diagnostic information with recent log tails to help the October team investigate Lantern problems.

**What to do:** In Lantern, open **Settings → About → Report a Problem**. If the report is sent, Lantern copies a `LAN-…` report code; send it to October support with a short description of what happened. If sending fails, Lantern saves the report to your Desktop. On Mac, it opens an email with the file attached; on Windows and Linux, it shows where to find the file so you can attach it.

The app-provided details omit conversation messages and project folders, as described in the [privacy guide](PRODUCT.md#privacy). Lantern redacts recognized secret patterns and replaces your home-directory path with `~`. This is best-effort redaction; log tails may contain other diagnostic text.
