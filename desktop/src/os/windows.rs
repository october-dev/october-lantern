//! Windows desktop integration. One "shell" thread owns a hidden window that carries the tray
//! icon, its notifications (shown by Windows as toasts) and the global shortcut; the rest are
//! plain calls. Screenshots capture the display under the pointer with Lantern's own window left
//! out; dictation records through the system's audio mapper and transcribes with whisper.cpp.
use super::Event;
use crate::settings::Settings;
use anyhow::{Context, Result, bail};
use std::cell::RefCell;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, ERROR_SUCCESS, GetLastError, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, POINT, STILL_ACTIVE,
    WPARAM,
};
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_MODIFY, NIM_SETVERSION, NIN_BALLOONTIMEOUT, NIN_BALLOONUSERCLICK,
    NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMessageW, HICON, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, RegisterClassW, RegisterWindowMessageW, SW_SHOWNORMAL,
    SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_HOTKEY, WM_LBUTTONUP,
    WM_NULL, WNDCLASSW,
};

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// Copies `text` into a fixed-size UTF-16 field, cut short to fit with its terminating NUL.
fn fill(field: &mut [u16], text: &str) {
    let units: Vec<u16> = text.encode_utf16().take(field.len() - 1).collect();
    field[..units.len()].copy_from_slice(&units);
    field[units.len()] = 0;
}

// ---------- single instance ----------

/// One pipe per user: a second launch hands its request (e.g. "settings") to the running
/// Lantern. Only the user (and administrators) can write to a pipe the user created.
fn pipe_name() -> Vec<u16> {
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_alphanumeric()).collect();
    wide(format!(r"\\.\pipe\dev.october.Lantern-{user}"))
}

pub fn bridge(tx: Sender<Event>, _snapshot: Arc<RwLock<String>>) {
    std::thread::spawn(move || {
        let name = pipe_name();
        loop {
            let pipe = unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    PIPE_UNLIMITED_INSTANCES,
                    0,
                    4096,
                    0,
                    std::ptr::null(),
                )
            };
            if pipe == INVALID_HANDLE_VALUE {
                let _ = tx.send(Event::Status("Desktop integration: another Lantern is already listening".into()));
                return;
            }
            let connected =
                unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            let mut file = unsafe { std::fs::File::from_raw_handle(pipe as _) };
            if !connected {
                continue;
            }
            let mut request = Vec::new();
            let _ = (&mut file).take(512).read_to_end(&mut request);
            let action = String::from_utf8_lossy(&request).trim().to_string();
            let event = Event::Activate(if action.is_empty() { "show".into() } else { action });
            if tx.send(event).is_err() {
                return;
            }
        }
    });
}

pub fn show_existing(action: &str) -> bool {
    let wide_name = pipe_name();
    let name = String::from_utf16_lossy(&wide_name[..wide_name.len() - 1]);
    for _ in 0..20 {
        match std::fs::OpenOptions::new().write(true).open(&name) {
            Ok(mut pipe) => return pipe.write_all(action.as_bytes()).is_ok(),
            // Every instance is busy with another launch: wait for the next one.
            Err(e) if e.raw_os_error() == Some(231) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
    false
}

/// Lets the engine bring a terminal window to the front (Windows only lets the foreground app
/// pass that on).
pub fn allow_focus(pid: u32) {
    unsafe { AllowSetForegroundWindow(pid) };
}

pub fn open(path: impl AsRef<OsStr>) -> Result<()> {
    let verb = wide("open");
    let target = wide(path.as_ref());
    let result =
        unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    // Values above 32 mean success.
    anyhow::ensure!(result as usize > 32, "Windows couldn't open {}", path.as_ref().to_string_lossy());
    Ok(())
}

// ---------- screenshots ----------

/// The display under the pointer, as a PNG in Lantern's screenshots folder, without a chooser
/// (Windows has none to show). Lantern's own windows draw nothing while it's taken.
pub fn capture_screen() -> Result<PathBuf> {
    let image = capture_display()?;
    let mut png = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    anyhow::ensure!(png.len() <= 20 * 1024 * 1024, "screenshot must be a PNG under 20 MB");
    let dest = crate::settings::data_dir().join("screenshots").join(format!("{}.png", uuid::Uuid::new_v4().simple()));
    crate::settings::private_write(&dest, &png)?;
    Ok(dest)
}

/// The display under the pointer as it is now, with where it is (physical pixels), for Point &
/// Ask's frozen overlay. Taken before the overlay shows, while Lantern's windows draw nothing.
pub fn freeze_screen() -> Result<Frozen> {
    let (image, monitor) = capture_monitor()?;
    Ok(Frozen { image: Arc::new(image), monitor })
}

/// A still of one display (Point & Ask's overlay background and the source of its crop).
pub struct Frozen {
    pub image: Arc<image::RgbaImage>,
    /// The display's bounds on the virtual screen, in physical pixels.
    pub monitor: eframe::egui::Rect,
}

fn capture_display() -> Result<image::RgbaImage> {
    capture_monitor().map(|(image, _)| image)
}

fn capture_monitor() -> Result<(image::RgbaImage, eframe::egui::Rect)> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ReleaseDC, SRCCOPY,
        SelectObject,
    };
    unsafe {
        let mut pointer = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pointer);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        anyhow::ensure!(GetMonitorInfoW(MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST), &mut info) != 0, "no display found");
        let r = info.rcMonitor;
        let (width, height) = (r.right - r.left, r.bottom - r.top);
        anyhow::ensure!(width > 0 && height > 0, "the display has no size");
        let screen = GetDC(std::ptr::null_mut());
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(memory, bitmap);
        let copied = BitBlt(memory, 0, 0, width, height, screen, r.left, r.top, SRCCOPY | CAPTUREBLT) != 0;
        let mut header: BITMAPINFO = std::mem::zeroed();
        header.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: rows top to bottom.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let rows = GetDIBits(memory, bitmap, 0, height as u32, pixels.as_mut_ptr().cast(), &mut header, DIB_RGB_COLORS);
        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(std::ptr::null_mut(), screen);
        anyhow::ensure!(copied && rows == height, "Windows couldn't capture the screen");
        // BGRA, with no meaningful alpha, to RGBA.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        let image = image::RgbaImage::from_raw(width as u32, height as u32, pixels).context("screenshot size mismatch")?;
        let monitor = eframe::egui::Rect::from_min_max(
            eframe::egui::pos2(r.left as f32, r.top as f32),
            eframe::egui::pos2(r.right as f32, r.bottom as f32),
        );
        Ok((image, monitor))
    }
}

pub fn diagnostics() -> Result<serde_json::Value> {
    Ok(
        serde_json::json!({"system": "windows", "tray": true, "globalShortcuts": ["Ctrl+Alt+Space", "Ctrl+Alt+P"], "screenshots": "display under the pointer", "dictation": "waveIn + whisper.cpp", "notifications": notifications_allowed()}),
    )
}

pub fn folder() -> Result<PathBuf> {
    rfd::FileDialog::new().set_title("Choose a project folder").pick_folder().context("no folder selected")
}

/// The login item and the Start menu shortcut. Each failure says what's left.
pub fn remove_integrations() -> Vec<String> {
    let mut failures = Vec::new();
    if let Err(error) = autostart_executable(false, Path::new("")) {
        failures.push(format!("Removing the login item: {error:#}. Turn October Lantern off in Settings › Apps › Startup."));
    }
    let shortcut = crate::updates::start_menu_shortcut();
    if shortcut.exists()
        && let Err(error) = std::fs::remove_file(&shortcut)
    {
        failures.push(format!("Removing the Start menu shortcut: {error}. Delete {} by hand.", shortcut.display()));
    }
    failures
}

/// The installer's entry in Apps (windows/installer.iss `AppId`, per user).
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{6C4C8A0E-5D0B-4F57-9D34-0C2A6F6E7B11}_is1";

fn registry_string(key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
    };
    if status != ERROR_SUCCESS || size < 2 {
        return None;
    }
    let mut buffer = vec![0u16; size as usize / 2];
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buffer).trim_end_matches('\0').to_string())
}

/// Removes the app through its installer's own uninstaller, which runs once Lantern has quit. Only
/// for the installed copy (or an update of it, in Lantern's data folder), never a build elsewhere.
pub fn remove_app(_installed: Option<&Path>) -> std::result::Result<(), String> {
    let folder = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)).unwrap_or_default();
    let (Some(location), Some(uninstaller)) =
        (registry_string(UNINSTALL_KEY, "InstallLocation"), registry_string(UNINSTALL_KEY, "UninstallString"))
    else {
        return Err(format!(
            "This copy of Lantern wasn't installed with its installer, so its folder ({}) was left in place. Delete it after Lantern quits.",
            folder.display()
        ));
    };
    let normal = |path: &Path| path.to_string_lossy().trim_end_matches('\\').to_lowercase();
    let installed = normal(&folder) == normal(Path::new(&location)) || folder.starts_with(crate::settings::data_dir().join("releases"));
    if !installed {
        return Err(format!(
            "This copy of Lantern ({}) isn't the installed one, so the installed app was left in place.",
            folder.display()
        ));
    }
    super::command(uninstaller.trim().trim_matches('"'))
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
        .spawn()
        .map(drop)
        .map_err(|error| format!("Couldn't start the uninstaller: {error}. Remove October Lantern in Settings › Apps."))
}

pub fn process_alive(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        ok && code == STILL_ACTIVE as u32
    }
}

// ---------- start at login ----------

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "October Lantern";

pub fn autostart_enabled() -> bool {
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    status == ERROR_SUCCESS
}

pub fn autostart_executable(enable: bool, executable: &Path) -> Result<()> {
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    if !enable {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
        return Ok(());
    }
    let command = wide(format!("\"{}\"", executable.display()));
    let status = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), REG_SZ, command.as_ptr().cast(), (command.len() * 2) as u32)
    };
    anyhow::ensure!(status == ERROR_SUCCESS, "couldn't add Lantern to the programs that start at sign-in");
    Ok(())
}

// ---------- sign-in, in Credential Manager ----------

const CREDENTIAL: &str = "dev.october.lantern/refresh-token";

pub fn secret_store(token: &str) -> Result<()> {
    let mut target = wide(CREDENTIAL);
    let mut blob = token.as_bytes().to_vec();
    let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
    credential.Type = CRED_TYPE_GENERIC;
    credential.TargetName = target.as_mut_ptr();
    credential.CredentialBlobSize = blob.len() as u32;
    credential.CredentialBlob = blob.as_mut_ptr();
    credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
    anyhow::ensure!(unsafe { CredWriteW(&credential, 0) } != 0, "Credential Manager could not save sign-in");
    Ok(())
}

pub fn secret_lookup() -> Result<String> {
    let target = wide(CREDENTIAL);
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    anyhow::ensure!(unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } != 0, "No saved October sign-in");
    let token = unsafe {
        let c = &*credential;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
        CredFree(credential.cast());
        bytes
    };
    Ok(String::from_utf8(token)?.trim().to_owned())
}

pub fn secret_clear() {
    let target = wide(CREDENTIAL);
    unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
}

// ---------- tray, notifications and the global shortcut ----------

enum ShellCommand {
    Tray,
    Shortcuts { message: String, point_ask: String },
    Notify { agent: String, title: String, body: String },
}

struct Shell {
    commands: Mutex<Sender<ShellCommand>>,
    /// The hidden window, as an integer so it can be shared between threads.
    window: usize,
}

/// Messages the hidden window gets: commands waiting, and tray icon events.
const WM_SHELL_COMMAND: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
/// The two global shortcuts: messaging an agent (Ask Lantern) and Point & Ask, as the preferences
/// choose them (the Mac's defaults are Ctrl+Alt+Space and Ctrl+Alt+P).
const HOTKEY_COMPOSE: i32 = 1;
const HOTKEY_CAPTURE: i32 = 2;

struct ShellState {
    tx: Sender<Event>,
    commands: Receiver<ShellCommand>,
    icon: HICON,
    tray: bool,
    /// The agent the notification on screen is about; clicking it opens that agent.
    notified: Option<String>,
    /// Explorer broadcasts this after it restarts; tray icons must be added again.
    taskbar_created: u32,
}

thread_local! {
    static STATE: RefCell<Option<ShellState>> = const { RefCell::new(None) };
}

static SHELL: OnceLock<Option<Shell>> = OnceLock::new();

fn send(tx: &Sender<Event>, command: ShellCommand) {
    let shell = SHELL.get_or_init(|| start_shell(tx.clone()));
    match shell {
        Some(shell) => {
            let _ = shell.commands.lock().unwrap().send(command);
            unsafe { PostMessageW(shell.window as HWND, WM_SHELL_COMMAND, 0, 0) };
        }
        None => {
            let _ = tx.send(Event::Status("Desktop integration: couldn't create the tray window".into()));
        }
    }
}

fn start_shell(tx: Sender<Event>) -> Option<Shell> {
    let (commands, receiver) = mpsc::channel();
    let (ready, window) = mpsc::channel();
    std::thread::spawn(move || unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("LanternShell");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(shell_proc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let title = wide("October Lantern");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            let _ = ready.send(None);
            return;
        }
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let icon = app_icon();
        STATE.with(|state| {
            *state.borrow_mut() = Some(ShellState { tx, commands: receiver, icon, tray: false, notified: None, taskbar_created })
        });
        let _ = ready.send(Some(hwnd as usize));
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
    let window = window.recv().ok()??;
    Some(Shell { commands: Mutex::new(commands), window })
}

/// Lantern's logo as a 32×32 icon.
fn app_icon() -> HICON {
    let Ok(image) = image::load_from_memory(include_bytes!("../../../logo.png")) else { return std::ptr::null_mut() };
    let image = image::imageops::resize(&image.to_rgba8(), 32, 32, image::imageops::FilterType::Lanczos3);
    // Windows wants BGRA; the AND mask is unused when the colors carry alpha.
    let bgra: Vec<u8> = image.pixels().flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
    let mask = [0u8; 32 * 32 / 8];
    unsafe { CreateIcon(GetModuleHandleW(std::ptr::null()), 32, 32, 1, 32, mask.as_ptr(), bgra.as_ptr()) }
}

fn icon_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = icon;
    fill(&mut data.szTip, "October Lantern");
    data
}

fn add_tray(hwnd: HWND, state: &mut ShellState) -> bool {
    let mut data = icon_data(hwnd, state.icon);
    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
        // Already there (e.g. added before Explorer restarted): fine.
        return unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0;
    }
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };
    true
}

fn run_command(hwnd: HWND, state: &mut ShellState, command: ShellCommand) {
    match command {
        ShellCommand::Tray => {
            state.tray = add_tray(hwnd, state);
            let _ = state.tx.send(Event::Tray(state.tray));
        }
        ShellCommand::Shortcuts { message, point_ask } => {
            let mut bound = false;
            for (id, keys) in [(HOTKEY_COMPOSE, &message), (HOTKEY_CAPTURE, &point_ask)] {
                unsafe { UnregisterHotKey(hwnd, id) };
                let Some(hotkey) = super::Hotkey::parse(keys) else { continue };
                let mut modifiers = MOD_NOREPEAT;
                for (on, flag) in [(hotkey.ctrl, MOD_CONTROL), (hotkey.alt, MOD_ALT), (hotkey.shift, MOD_SHIFT)] {
                    if on {
                        modifiers |= flag;
                    }
                }
                // Virtual-key codes for Space, A–Z and 0–9 are their ASCII codes.
                if unsafe { RegisterHotKey(hwnd, id, modifiers, hotkey.key as u32) } != 0 {
                    bound = true;
                } else {
                    let label = crate::model::hotkey_label(keys);
                    let _ = state.tx.send(Event::Status(format!("Shortcuts: {label} is taken by another app")));
                }
            }
            let _ = state.tx.send(Event::Shortcuts(bound));
        }
        ShellCommand::Notify { agent, title, body } => {
            // Notifications come from the tray icon; it has to be there.
            if !state.tray {
                state.tray = add_tray(hwnd, state);
                let _ = state.tx.send(Event::Tray(state.tray));
            }
            let mut data = icon_data(hwnd, state.icon);
            data.uFlags = NIF_INFO;
            fill(&mut data.szInfoTitle, &title);
            fill(&mut data.szInfo, if body.is_empty() { " " } else { &body });
            data.dwInfoFlags = NIIF_INFO;
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0 {
                state.notified = Some(agent);
            } else {
                let _ = state.tx.send(Event::Status("Notification: Windows didn't show it".into()));
            }
        }
    }
}

fn tray_menu(hwnd: HWND, tx: &Sender<Event>) {
    let items = super::tray_menu();
    unsafe {
        let menu = CreatePopupMenu();
        // Menu ids are the item's position + 1 (0 means nothing was chosen).
        for (index, item) in items.iter().enumerate() {
            match item {
                Some((label, _)) => AppendMenuW(menu, MF_STRING, index + 1, wide(label).as_ptr()),
                None => AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()),
            };
        }
        let mut at = POINT { x: 0, y: 0 };
        GetCursorPos(&mut at);
        // The menu closes when clicking elsewhere only if its window is in front.
        SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, at.x, at.y, 0, hwnd, std::ptr::null()) as usize;
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        if let Some(Some((_, id))) = chosen.checked_sub(1).and_then(|index| items.get(index)) {
            let _ = tx.send(super::tray_event(id));
        }
    }
}

unsafe extern "system" fn shell_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = STATE.with(|cell| {
        // A message can arrive while one is being handled (Windows delivers sent messages while
        // a call waits); leave those to the default handler rather than borrow twice.
        let Ok(mut borrowed) = cell.try_borrow_mut() else { return false };
        let Some(state) = borrowed.as_mut() else { return false };
        match msg {
            WM_SHELL_COMMAND => {
                while let Ok(command) = state.commands.try_recv() {
                    run_command(hwnd, state, command);
                }
                true
            }
            WM_TRAY => {
                // NOTIFYICON_VERSION_4: the event is in the low word of lparam.
                match (lparam & 0xffff) as u32 {
                    WM_LBUTTONUP | NIN_SELECT => {
                        let _ = state.tx.send(Event::Activate("show".into()));
                    }
                    NIN_BALLOONUSERCLICK => {
                        let agent = state.notified.take().unwrap_or_else(|| "show".into());
                        let _ = state.tx.send(Event::Activate(agent));
                    }
                    NIN_BALLOONTIMEOUT => state.notified = None,
                    WM_CONTEXTMENU => {
                        let tx = state.tx.clone();
                        drop(borrowed);
                        tray_menu(hwnd, &tx);
                        return true;
                    }
                    _ => {}
                }
                true
            }
            WM_HOTKEY if wparam as i32 == HOTKEY_COMPOSE => {
                let _ = state.tx.send(Event::Activate("compose".into()));
                true
            }
            // Point & Ask is held to point and speak. Windows reports only the press, so its key
            // (lParam's high word) is watched until it comes up.
            WM_HOTKEY if wparam as i32 == HOTKEY_CAPTURE => {
                let _ = state.tx.send(Event::Activate("point-press".into()));
                let key = ((lparam as usize >> 16) & 0xFFFF) as i32;
                let tx = state.tx.clone();
                std::thread::spawn(move || {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
                    while unsafe { GetAsyncKeyState(key) } as u16 & 0x8000 != 0 && std::time::Instant::now() < deadline {
                        std::thread::sleep(std::time::Duration::from_millis(30));
                    }
                    let _ = tx.send(Event::Activate("point-release".into()));
                });
                true
            }
            m if m == state.taskbar_created && state.tray => {
                add_tray(hwnd, state);
                true
            }
            _ => false,
        }
    });
    if handled { 0 } else { unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) } }
}

pub fn tray(tx: Sender<Event>) {
    send(&tx, ShellCommand::Tray);
}

pub fn shortcuts(tx: Sender<Event>, message: String, point_ask: String) {
    send(&tx, ShellCommand::Shortcuts { message, point_ask });
}

/// Whether Windows shows notifications at all (Settings › System › Notifications); it is off
/// when `ToastEnabled` is 0. Lantern's own come from its tray icon.
pub fn notifications_allowed() -> Option<bool> {
    let (key, value) = (wide(r"Software\Microsoft\Windows\CurrentVersion\PushNotifications"), wide("ToastEnabled"));
    let mut data = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    // No value: Windows' default, on.
    Some(status != ERROR_SUCCESS || data != 0)
}
pub fn notify(tx: Sender<Event>, agent: String, title: String, body: String) {
    send(&tx, ShellCommand::Notify { agent, title, body });
}

// ---------- dictation ----------

/// whisper.cpp wants 16 kHz mono 16-bit audio; the system's audio mapper converts the
/// microphone to that, so no resampling happens here.
const RATE: u32 = 16_000;
/// Ten minutes at most, as one dictation.
const MAX_SAMPLES: usize = RATE as usize * 600;

pub struct Recording {
    stop: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<Result<Vec<i16>>>>,
    path: PathBuf,
}

impl Recording {
    pub fn start() -> Result<Self> {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (opened, ready) = mpsc::channel();
        let worker = {
            let stop = stop.clone();
            std::thread::spawn(move || record(&stop, opened))
        };
        match ready.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => bail!("{error}"),
            Err(_) => bail!("The microphone recorder stopped"),
        }
        let path = crate::settings::data_dir().join("dictation").join(format!("{}.wav", uuid::Uuid::new_v4().simple()));
        Ok(Self { stop, worker: Some(worker), path })
    }
    pub fn finish(mut self, settings: Settings) -> Result<String> {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let samples = self.worker.take().context("not recording")?.join().map_err(|_| anyhow::anyhow!("the recorder crashed"))??;
        anyhow::ensure!(!samples.is_empty(), "No audio was recorded");
        crate::settings::private_write(&self.path, &wav(&samples))?;
        let output = super::command(&settings.whisper)
            .args(["-m", &settings.whisper_model, "-f"])
            .arg(&self.path)
            .args(["-l", &settings.speech_language, "-nt", "-np"])
            .output()
            .context("Run scripts/setup-dictation.ps1 or configure a whisper.cpp executable and model in Settings")?;
        anyhow::ensure!(output.status.success(), "Transcription failed: {}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8(output.stdout)?.trim().to_owned();
        anyhow::ensure!(!text.is_empty(), "No speech detected");
        Ok(text)
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Records until `stop`, saying through `opened` whether the microphone could be opened.
fn record(stop: &std::sync::atomic::AtomicBool, opened: Sender<Result<(), String>>) -> Result<Vec<i16>> {
    use windows_sys::Win32::Media::Audio::{
        CALLBACK_EVENT, HWAVEIN, WAVE_FORMAT_PCM, WAVE_MAPPER, WAVEFORMATEX, WAVEHDR, WHDR_DONE, waveInAddBuffer, waveInClose, waveInOpen,
        waveInPrepareHeader, waveInReset, waveInStart, waveInUnprepareHeader,
    };
    use windows_sys::Win32::Media::MMSYSERR_NOERROR;
    use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    const BUFFERS: usize = 4;
    // A quarter of a second each.
    const BYTES: usize = RATE as usize / 4 * 2;
    unsafe {
        let event = CreateEventW(std::ptr::null(), 0, 0, std::ptr::null());
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: 1,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * 2,
            nBlockAlign: 2,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut device: HWAVEIN = std::ptr::null_mut();
        let status = waveInOpen(&mut device, WAVE_MAPPER, &format, event as usize, 0, CALLBACK_EVENT);
        if status != MMSYSERR_NOERROR {
            CloseHandle(event);
            let _ = opened.send(Err("No microphone is available (check Settings › Privacy › Microphone)".into()));
            bail!("no microphone");
        }
        // Buffers stay at fixed addresses while Windows fills them.
        let mut data: Vec<Vec<u8>> = (0..BUFFERS).map(|_| vec![0u8; BYTES]).collect();
        let mut headers: Vec<WAVEHDR> =
            data.iter_mut().map(|d| WAVEHDR { lpData: d.as_mut_ptr(), dwBufferLength: BYTES as u32, ..std::mem::zeroed() }).collect();
        let size = std::mem::size_of::<WAVEHDR>() as u32;
        for header in headers.iter_mut() {
            waveInPrepareHeader(device, header, size);
            waveInAddBuffer(device, header, size);
        }
        waveInStart(device);
        let _ = opened.send(Ok(()));
        let mut samples: Vec<i16> = Vec::new();
        let take = |header: &mut WAVEHDR, samples: &mut Vec<i16>| {
            let bytes = std::slice::from_raw_parts(header.lpData, header.dwBytesRecorded as usize);
            samples.extend(bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)));
            header.dwFlags &= !WHDR_DONE;
        };
        while !stop.load(std::sync::atomic::Ordering::SeqCst) && samples.len() < MAX_SAMPLES {
            WaitForSingleObject(event, 100);
            for header in headers.iter_mut().filter(|h| h.dwFlags & WHDR_DONE != 0) {
                take(header, &mut samples);
                waveInAddBuffer(device, header, size);
            }
        }
        // Hands back the buffers in progress, filled as far as they got.
        waveInReset(device);
        for header in headers.iter_mut() {
            if header.dwFlags & WHDR_DONE != 0 {
                take(header, &mut samples);
            }
            waveInUnprepareHeader(device, header, size);
        }
        waveInClose(device);
        CloseHandle(event);
        drop(data);
        Ok(samples)
    }
}

/// A 16 kHz mono 16-bit WAV file.
fn wav(samples: &[i16]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(RATE.to_le_bytes());
    out.extend((RATE * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    out.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_files_carry_16khz_mono_pcm() {
        let file = wav(&[0, 1, -1]);
        assert_eq!(file.len(), 44 + 6);
        assert_eq!(&file[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(file[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u16::from_le_bytes(file[22..24].try_into().unwrap()), 1);
        assert_eq!(&file[44..], &[0, 0, 1, 0, 255, 255]);
    }
}
