//! The pill and panel on Windows (CONTRACT §7). UNVERIFIED on a real machine (§8.2c): topmost and
//! tool-window styles keep them on top and out of the taskbar and Alt-Tab; the pill also never
//! activates. Presence on every virtual desktop is not established by these styles alone.
//! Windows 11 frames every top-level window (a 1px border, rounded corners and a shadow), which
//! shows as a faint rectangle around Lantern's rounded glass; [`unframe`] turns that off.
use super::shell::Role;
use eframe::egui::{Pos2, Rect, pos2};
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetCursorPos, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId, HWND_TOPMOST,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WM_NCCALCSIZE, WS_CAPTION,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

/// Work areas (each monitor without its taskbar), in physical pixels.
pub fn work_areas() -> Vec<Rect> {
    displays(false)
}

/// Whole monitors, in physical pixels (Point & Ask's selection overlay).
pub fn monitors() -> Vec<Rect> {
    displays(true)
}

fn displays(whole: bool) -> Vec<Rect> {
    struct Out {
        whole: bool,
        areas: Vec<Rect>,
    }
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        let out = unsafe { &mut *(data as *mut Out) };
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            let r = if out.whole { info.rcMonitor } else { info.rcWork };
            out.areas.push(Rect::from_min_max(pos2(r.left as f32, r.top as f32), pos2(r.right as f32, r.bottom as f32)));
        }
        1
    }
    let mut out = Out { whole, areas: Vec::new() };
    unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(each), &mut out as *mut _ as LPARAM) };
    out.areas
}

/// This process's top-level window with `title`.
fn find(title: &str) -> Option<HWND> {
    struct Search {
        title: Vec<u16>,
        pid: u32,
        found: HWND,
    }
    unsafe extern "system" fn each(window: HWND, data: LPARAM) -> i32 {
        let search = unsafe { &mut *(data as *mut Search) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid != search.pid {
            return 1;
        }
        let mut text = [0u16; 256];
        let len = unsafe { GetWindowTextW(window, text.as_mut_ptr(), text.len() as i32) } as usize;
        if text[..len] == search.title[..] {
            search.found = window;
            return 0;
        }
        1
    }
    let mut search = Search { title: title.encode_utf16().collect(), pid: std::process::id(), found: std::ptr::null_mut() };
    unsafe { EnumWindows(Some(each), &mut search as *mut _ as LPARAM) };
    (!search.found.is_null()).then_some(search.found)
}

/// Gives a Lantern window its role. Returns false until the window exists.
pub fn adopt(title: &str, role: Role) -> bool {
    let Some(window) = find(title) else { return false };
    unsafe {
        let mut style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        style = (style & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
        if role == Role::Pill {
            style |= WS_EX_NOACTIVATE;
        }
        SetWindowLongPtrW(window, GWL_EXSTYLE, style as isize);
        // An undecorated window still has its caption bits, and DWM draws a red system close
        // button at its top right on hover (closing the window under Lantern). Drop them.
        let frame = GetWindowLongPtrW(window, GWL_STYLE) as u32;
        SetWindowLongPtrW(
            window,
            GWL_STYLE,
            (frame & !(WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_THICKFRAME)) as isize,
        );
        unframe(window);
        SetWindowPos(window, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
    true
}

/// Takes the frame off one of Lantern's borderless windows that has no other role (the menu, the
/// pill's tip). Returns false until the window exists.
pub fn borderless(title: &str) -> bool {
    let Some(window) = find(title) else { return false };
    unframe(window);
    unsafe {
        SetWindowPos(window, std::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER | SWP_FRAMECHANGED)
    };
    true
}

/// No system frame on a borderless, transparent window: no Windows 11 border, rounded corners or
/// shadow (the glass draws its own), and no 1px strip at the top. winit keeps a sliver of
/// non-client area there for the shadow it asks for on undecorated windows (egui-winit always
/// asks on Windows); a subclass gives the whole window to the client area instead. Windows 10
/// doesn't know the border and corner attributes; their errors are ignored. Callers follow with
/// `SWP_FRAMECHANGED` so the frame is recalculated.
fn unframe(window: HWND) {
    let set = |attribute: i32, value: u32| unsafe {
        DwmSetWindowAttribute(window, attribute as u32, &value as *const u32 as *const _, std::mem::size_of::<u32>() as u32)
    };
    let _ = set(DWMWA_NCRENDERING_POLICY, DWMNCRP_DISABLED as u32);
    let _ = set(DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE);
    let _ = set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND as u32);
    unsafe { SetWindowSubclass(window, Some(no_frame), NO_FRAME, 0) };
}

const NO_FRAME: usize = 0x4c61_6e74; // "Lant"

unsafe extern "system" fn no_frame(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _: usize, _: usize) -> LRESULT {
    if message == WM_NCCALCSIZE && wparam != 0 {
        return 0; // the client area is the whole window
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

/// The global pointer, in physical pixels: reliable on Windows, so the pill keeps the Mac's
/// polled 6pt hover halo.
pub fn cursor_position() -> Option<Pos2> {
    let mut point = POINT { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut point) } != 0).then(|| pos2(point.x as f32, point.y as f32))
}

/// Apps where a Task makes no sense (a normal session instead), by executable.
const TERMINALS: [&str; 9] = [
    "windowsterminal.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "conhost.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "mintty.exe",
    "openconsole.exe",
];

/// The app in front: its name (the executable's description, else its file name), the window's
/// title, and whether it's a terminal. Lantern's own windows never take focus, so this is still
/// the person's app when they click the pill.
pub fn foreground_app() -> Option<crate::model::AppContext> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if pid == 0 || pid == std::process::id() {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut len) } != 0;
    unsafe { CloseHandle(process) };
    if !ok {
        return None;
    }
    let exe = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize]));
    let file = exe.file_name()?.to_string_lossy().to_lowercase();
    let stem = exe.file_stem()?.to_string_lossy().into_owned();
    let mut text = [0u16; 512];
    let title_len = unsafe { GetWindowTextW(window, text.as_mut_ptr(), text.len() as i32) } as usize;
    let title = String::from_utf16_lossy(&text[..title_len]);
    Some(crate::model::AppContext {
        name: file_description(&exe).unwrap_or(stem),
        is_terminal: TERMINALS.contains(&file.as_str()),
        id: file,
        window: (!title.is_empty()).then_some(title),
        pid: Some(pid as i64),
        ..Default::default()
    })
}

/// The executable's FileDescription ("Visual Studio Code"), from its version resource.
fn file_description(exe: &std::path::Path) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let path: Vec<u16> = exe.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut()) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let query = |key: &str| -> Option<(*const u8, u32)> {
        let key: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
        let mut value = std::ptr::null_mut();
        let mut len = 0u32;
        (unsafe { VerQueryValueW(data.as_ptr().cast(), key.as_ptr(), &mut value, &mut len) } != 0 && !value.is_null() && len > 0)
            .then_some((value as *const u8, len))
    };
    // The first language/code page the file lists.
    let (translation, len) = query("\\VarFileInfo\\Translation")?;
    if len < 4 {
        return None;
    }
    let pair = unsafe { std::slice::from_raw_parts(translation as *const u16, 2) };
    let (description, chars) = query(&format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", pair[0], pair[1]))?;
    let text = unsafe { std::slice::from_raw_parts(description as *const u16, chars as usize) };
    let name = String::from_utf16_lossy(text).trim_end_matches('\0').trim().to_string();
    (!name.is_empty()).then_some(name)
}
