//! OS adapters shared by the Rust core. Platform-specific UI never discovers or types itself.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The user's home folder.
pub fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn data_dir(app: &str) -> PathBuf {
    if cfg!(target_os = "macos") {
        return home().join("Library/Application Support").join(app);
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home().join("AppData").join("Local"))
            .join(app);
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"))
        .join(if app == "October Lantern" { "october-lantern" } else { app })
}

/// Opens a URL (e.g. `october://…`) with the app registered for it.
#[cfg(unix)]
pub fn open_url(url: &str, limit: Duration) -> anyhow::Result<()> {
    let opener = if cfg!(target_os = "macos") { "/usr/bin/open" } else { "xdg-open" };
    let out = crate::run::output(Command::new(opener).arg(url), limit)?;
    if !out.status.success() {
        anyhow::bail!("couldn't open {url}");
    }
    Ok(())
}

#[cfg(windows)]
pub fn open_url(url: &str, _limit: Duration) -> anyhow::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb = wide("open");
    let target = wide(url);
    // Values above 32 mean success.
    let r =
        unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    if r as usize <= 32 {
        anyhow::bail!("couldn't open {url}");
    }
    Ok(())
}

/// A NUL-terminated UTF-16 string for Win32 calls.
#[cfg(windows)]
pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// Sets Unix permission bits. Windows has none: Lantern's folders there are already private to
/// the user (`%LOCALAPPDATA%`), and executables need no flag.
pub fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(windows)]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// Whether only the owner can read or write the file.
pub fn is_private(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o077 == 0
    }
    #[cfg(windows)]
    {
        let _ = meta;
        true
    }
}

/// A number that changes when a file is replaced rather than edited in place (its inode), or 0
/// where there isn't one.
pub fn file_id(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.ino()
    }
    #[cfg(windows)]
    {
        // Creation time stands in: a replaced file is a new file.
        meta.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64)
    }
}

/// Whether a process with this pid exists.
pub fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            ok && code == STILL_ACTIVE as u32
        }
    }
}

/// Starts `cmd` on its own, so it outlives the engine and isn't stopped along with it.
pub fn detach(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
    }
}

/// Where `name` would run from in a new Command Prompt: the first folder on `PATH` holding it
/// with one of `PATHEXT`'s extensions. `PATH` is brought up to date first (see `refresh_path`).
#[cfg(windows)]
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    refresh_path();
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        exts.split(';')
            .filter(|e| !e.is_empty())
            .map(|ext| dir.join(format!("{name}{}", ext.to_ascii_lowercase())))
            // App execution aliases (e.g. `wt.exe`) are links metadata can't follow.
            .find(|p| p.is_file() || p.symlink_metadata().is_ok_and(|m| !m.is_dir()))
    })
}

/// Windows: makes the engine's `PATH` what a Command Prompt opened now would have, so an agent
/// installed after Lantern started is found and runs, with what it needs (Node.js) on `PATH`
/// too. A process keeps the `PATH` it started with, and Lantern often starts at sign-in, before
/// the install. That's the system's and the user's `PATH` from the registry, then the engine's own
/// (what Lantern added), then folders agent installers use without always adding them to `PATH`
/// (Claude Code's native installer: `~\.local\bin`). At most every 10 seconds.
#[cfg(windows)]
pub fn refresh_path() {
    use std::sync::Mutex;
    use std::time::Instant;
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|at| at.elapsed() < Duration::from_secs(10)) {
            return;
        }
        *last = Some(Instant::now());
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for key in [r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment", r"HKCU\Environment"] {
        if let Some(value) = registry_path(key) {
            dirs.extend(std::env::split_paths(&expand_env(&value)));
        }
    }
    dirs.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let known = [
        Some(home().join(".local").join("bin")),
        var("APPDATA").map(|d| d.join("npm")),
        var("LOCALAPPDATA").map(|d| d.join("Microsoft").join("WinGet").join("Links")),
        Some(home().join(".bun").join("bin")),
        Some(home().join("scoop").join("shims")),
    ];
    dirs.extend(known.into_iter().flatten().filter(|d| d.is_dir()));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.to_string_lossy().trim_end_matches('\\').to_lowercase()));
    if let Ok(path) = std::env::join_paths(dirs) {
        // SAFETY: Windows' environment block is guarded by the system (SetEnvironmentVariableW),
        // so another thread reading PATH meanwhile sees the old or the new value, never a torn one.
        unsafe { std::env::set_var("PATH", path) };
    }
}

/// A `Path` value under `key`, from `reg query`.
#[cfg(windows)]
fn registry_path(key: &str) -> Option<String> {
    let out = crate::run::output(Command::new("reg").args(["query", key, "/v", "Path"]), Duration::from_secs(5)).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.trim_start().to_ascii_lowercase().starts_with("path "))?;
    let at = line.find("REG_EXPAND_SZ").map(|i| i + "REG_EXPAND_SZ".len()).or_else(|| line.find("REG_SZ").map(|i| i + "REG_SZ".len()))?;
    Some(line[at..].trim().to_string())
}

/// `%NAME%` references replaced with the environment's values (unknown ones are left as they are).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn expand_env(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => match std::env::var(&after[..end]) {
                Ok(v) => {
                    out.push_str(&v);
                    rest = &after[end + 1..];
                }
                Err(_) => {
                    out.push_str(&rest[start..start + end + 2]);
                    rest = &after[end + 1..];
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Brings the terminal window of `host` (a `HostApp` pid) to the front: a top-level window of
/// that process or, for a console host, the console window it draws. Windows reports a console
/// window as belonging to a process attached to the console, so those are matched through their
/// console host.
#[cfg(windows)]
pub fn focus_window(host: u32) -> anyhow::Result<()> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GetClassNameW, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE,
        SetForegroundWindow, ShowWindow,
    };

    struct Search {
        host: u32,
        console: String,
        found: HWND,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> i32 {
        let search = unsafe { &mut *(data as *mut Search) };
        if unsafe { IsWindowVisible(hwnd) } == 0 || !unsafe { GetWindow(hwnd, GW_OWNER) }.is_null() {
            return 1;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let mut class = [0u16; 64];
        let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) }.max(0) as usize;
        let is_console = String::from_utf16_lossy(&class[..len]) == "ConsoleWindowClass";
        let matches = pid == search.host || is_console && crate::procs::live(pid).and_then(|l| l.tty).as_deref() == Some(&search.console);
        if matches {
            search.found = hwnd;
            return 0;
        }
        1
    }

    let mut search = Search { host, console: format!("con:{host}"), found: std::ptr::null_mut() };
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    if search.found.is_null() {
        anyhow::bail!("couldn't find the terminal's window");
    }
    unsafe {
        if IsIconic(search.found) != 0 {
            ShowWindow(search.found, SW_RESTORE);
        }
        SetForegroundWindow(search.found);
    }
    Ok(())
}

pub fn open_files(pid: u32) -> Vec<PathBuf> {
    // Only the Mac Codex app is found through its open files.
    #[cfg(windows)]
    {
        let _ = pid;
        Vec::new()
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir(format!("/proc/{pid}/fd"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read_link(e.path()).ok())
            .collect()
    }
    #[cfg(target_os = "macos")]
    {
        let Ok(out) = crate::run::output(
            std::process::Command::new("/usr/sbin/lsof").args(["-p", &pid.to_string(), "-Fn"]),
            std::time::Duration::from_secs(3),
        ) else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.strip_prefix('n')).map(PathBuf::from).collect()
    }
}

#[cfg(target_os = "linux")]
fn terminal_args(name: &str, script: &str) -> Option<Vec<String>> {
    let prefix: &[&str] = match name {
        "ptyxis" => &["--new-window", "--"],
        "gnome-terminal" => &["--"],
        "kgx" => &["--"],
        "konsole" => &["--separate", "-e"],
        "xfce4-terminal" => &["--disable-server", "-x"],
        "foot" => &["--"],
        "x-terminal-emulator" | "xterm" | "alacritty" | "kitty" | "ghostty" | "wezterm" => {
            if name == "wezterm" {
                &["start", "--"]
            } else {
                &["-e"]
            }
        }
        _ => return None,
    };
    Some(prefix.iter().map(|s| s.to_string()).chain(["/bin/sh".into(), "-c".into(), script.into()]).collect())
}

#[cfg(target_os = "linux")]
pub fn open_terminal(script: &str) -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    use std::process::{Command, Stdio};
    // An override is one executable path/name, never a shell command.
    let override_bin = std::env::var("LANTERN_TERMINAL").ok();
    let candidates = [
        "ptyxis",
        "gnome-terminal",
        "kgx",
        "konsole",
        "xfce4-terminal",
        "x-terminal-emulator",
        "kitty",
        "alacritty",
        "ghostty",
        "foot",
        "wezterm",
        "xterm",
    ];
    let bins: Vec<&str> = override_bin.as_deref().map(|b| vec![b]).unwrap_or_else(|| candidates.to_vec());
    for bin in bins {
        let name = crate::procs::basename(bin);
        let args =
            terminal_args(name, script).with_context(|| format!("unsupported terminal {bin}; use a supported terminal executable"))?;
        let available = std::path::Path::new(bin).is_absolute() && std::path::Path::new(bin).is_file()
            || std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()));
        if !available {
            continue;
        }
        let mut child = Command::new(bin)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("opening {bin}"))?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("{bin} failed to open a terminal ({status})");
            }
            return Ok(());
        }
        // Terminal windows often stay alive until closed. Reap without blocking the engine.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }
    bail!("no supported terminal found; install a terminal or set LANTERN_TERMINAL (e.g. ptyxis, gnome-terminal, konsole, kitty, xterm)")
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn terminal_passes_script_as_one_argument() {
        let script = "exec tmux -L lantern attach -t 'project with spaces'";
        let args = terminal_args("ptyxis", script).unwrap();
        assert_eq!(args, ["--new-window", "--", "/bin/sh", "-c", script]);
        assert!(terminal_args("unknown", script).is_none());
    }

    #[test]
    fn open_file_lookup_finds_current_process_file() {
        let path = std::env::temp_dir().join(format!("lantern-procfs-{}", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        assert!(open_files(std::process::id()).contains(&path));
        drop(file);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod path_tests {
    #[test]
    fn registry_paths_expand_their_variables() {
        let home = std::env::var("PATH").unwrap();
        assert_eq!(super::expand_env("%PATH%"), home);
        assert_eq!(super::expand_env(r"%LANTERN_NOT_SET_ANYWHERE%\bin;C:\x"), r"%LANTERN_NOT_SET_ANYWHERE%\bin;C:\x");
        assert_eq!(super::expand_env("100% sure"), "100% sure");
    }
}
