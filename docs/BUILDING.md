# Building Lantern

Run commands from the repository root. The Mac app is Swift / AppKit / SwiftUI; Windows and Linux use Rust / egui. Each starts the shared `lantern-engine` sidecar over the [JSON-lines protocol](../protocol/README.md).

- [macOS](#macos)
- [Linux](#linux)
- [Windows](#windows)
- [Development and checks](#development-and-checks)

## macOS

Install **Xcode 26** and a current **stable Rust toolchain** from [rustup](https://rustup.rs). The app runs on macOS 14+, but building uses newer SDK APIs.

```sh
scripts/build-app.sh --debug
open "build/October Lantern.app"
```

Omit `--debug` for an optimized build. Set `UNIVERSAL=1` to build for Apple silicon and Intel; install the corresponding Rust targets first. Local builds are ad-hoc signed unless `SIGN_IDENTITY` is configured. For distribution signing, notarization and publishing, see [Releasing](RELEASING.md).

## Linux

Install stable Rust, a C compiler, Mesa/OpenGL and your distribution's Wayland/X11 runtime libraries. Install `tmux` for direct replies and background sessions. SQLite is bundled.

On Fedora:

```sh
sudo dnf install gcc tmux mesa-libGL libxkbcommon wayland-libs
```

Build and run:

```sh
scripts/build-app.sh --debug
./build/linux/october-lantern
```

Omit `--debug` for an optimized build. Keep `october-lantern` and `lantern-engine` together in `build/linux/`. During development, `LANTERN_ENGINE` can override the sidecar path.

### Desktop and terminal integration

- Sessions started in Lantern run in tmux and accept direct replies. Agents in other terminals also accept direct replies when running inside tmux; otherwise Lantern uses the clipboard.
- Supported launchers include Ptyxis, GNOME Terminal, Konsole, Kitty, Alacritty, Ghostty, Foot, WezTerm, Xfce Terminal and xterm. Set `LANTERN_TERMINAL` to a supported executable name or absolute path to choose one.
- GNOME uses a tinted fill instead of backdrop blur. On Wayland, the edge pill uses XWayland; without XWayland, Lantern opens as a normal window. Point & Ask supports area selection or the whole screen, but cannot pick another app's window there.
- Screenshots and folder selection use XDG desktop portals. Global shortcuts need desktop approval. GNOME's tray menu needs an AppIndicator host; Quit is also available by right-clicking the lantern.
- Saved October sign-in uses Secret Service through `secret-tool`; without it, sign-in lasts for the current session.

### Local dictation

Install whisper.cpp and its model with the setup script. It requires a C++ compiler and CMake; if CMake is missing, `uv` can install it:

```sh
scripts/setup-dictation.sh
```

Restart Lantern afterward. A package-manager `whisper-cli` on `PATH` also works: when the model is missing, the mic offers a checksum-verified download before listening. Audio is transcribed locally. PipeWire's `pw-record` records the microphone; Fedora's optional runtime packages are `pipewire-utils` and `libsecret`.

### October Bus and packaging

To bundle the pinned native Bus, install Go 1.27+ and run:

```sh
scripts/build-bus-linux.sh
python3 scripts/smoke-linux.py --bus
```

The Bus binary sits beside the engine. `LANTERN_BUS_BIN` can override it during development. Local Bus sessions do not need an October account.

`scripts/package-linux.sh` creates the native bundle and SHA-256 file. See [Releasing](RELEASING.md) for distribution. Updates use `harshsaver/october-lantern-releases`; a fork can set `LANTERN_RELEASES_REPO=owner/repo` at build time.

## Windows

Install **PowerShell 7**, stable Rust with the **MSVC toolchain**, and **Visual Studio Build Tools** with C++/Windows SDK support. From PowerShell:

```powershell
pwsh scripts/build-windows.ps1 -Debug
.\build\windows\october-lantern.exe
```

Omit `-Debug` for an optimized build without a console window. Keep the app and engine together in `build\windows\`.

### Sessions and desktop integration

- Lantern discovers supported agents in Windows Terminal, VS Code, Cursor, WezTerm and console windows. Open brings the terminal window forward, rather than selecting an exact tab.
- Direct replies go to sessions Lantern starts in its own ConPTY host. Their terminal windows and Lantern can close while the agent keeps running. Agents started in other terminals are monitored, not typed into.
- Screenshots use the display under the pointer and leave out Lantern's own windows. Sign-in uses Windows Credential Manager. Dictation uses local whisper.cpp; the mic asks before downloading a missing speech model/runtime.
- Windows is a preview. Use the [release notes](https://github.com/harshsaver/october-lantern-releases/releases) for current installer and signing information; packaging supports signing but a configured build is not evidence that a published installer is signed.

### Bus, dictation and packaging

Install Go 1.27+ for Bus; git, CMake and the Visual Studio C++ tools for whisper.cpp; and [Inno Setup](https://jrsoftware.org/isinfo.php) for the installer:

```powershell
pwsh scripts/build-bus-windows.ps1
pwsh scripts/build-whisper-windows.ps1
pwsh scripts/package-windows.ps1
```

For local dictation setup on a source build, use `scripts/setup-dictation.ps1`. The installer installs for the current user without administrator rights. `package-windows.ps1` can sign the app and installer through Azure Artifact Signing or a configured `.pfx` certificate; see [Releasing](RELEASING.md).

## Development and checks

| Host | Check |
|---|---|
| macOS / Linux | `scripts/check.sh` |
| Windows | `pwsh scripts/check.ps1` |

These run formatting, linting, engine tests and the native app's build/tests for the host platform. For terminal discovery, session and reply smoke tests, use `python3 scripts/smoke-linux.py` or `python scripts/smoke-windows.py` on the matching platform. `scripts/check-windows-types.sh` checks Windows-specific Rust types from Linux; it does not replace native testing.

### Launch and fixture tools

The Rust desktop accepts `--new-session`, `--point-ask`, `--ask`, `--connections`, `--settings` and `--welcome`. A second launch forwards the requested surface to the running app.

To render a Rust desktop fixture:

```sh
./build/linux/october-lantern --fixture desktop/fixtures/panel-chat.json --out /tmp/lantern-chat.png --scale 2
```

Mac launch/snapshot flags and engine commands are listed in the [product guide](PRODUCT.md#building-and-releasing).

### Data locations

| Platform | Data | Preferences |
|---|---|---|
| macOS | `~/Library/Application Support/October Lantern/` | macOS app defaults |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/october-lantern/` | `${XDG_CONFIG_HOME:-~/.config}/october-lantern/` |
| Windows | `%LOCALAPPDATA%\October Lantern\` | `%APPDATA%\October Lantern\` |

Linux's `lantern-state.json` holds app preferences; `settings.json` beside it holds terminal and dictation paths. For network requests, screenshots, saved chats and diagnostics, see [Privacy](PRODUCT.md#privacy).
