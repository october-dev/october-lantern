use crate::show_overlay::{Display, Screen, Window};
use anyhow::{Context, Result};
use eframe::egui::{Pos2, Rect, pos2};
use std::{cell::RefCell, collections::VecDeque, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
        Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint},
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        WindowsAndMessaging::*,
    },
};

thread_local! { static CLICKS: RefCell<VecDeque<Pos2>> = const { RefCell::new(VecDeque::new()) }; }

pub(super) struct Desktop {
    hook: HHOOK,
}

impl Drop for Desktop {
    fn drop(&mut self) {
        if !self.hook.is_null() {
            unsafe { UnhookWindowsHookEx(self.hook) };
        }
    }
}

impl Desktop {
    pub fn connect() -> Result<Self> {
        Ok(Self { hook: ptr::null_mut() })
    }

    pub fn display(&mut self, previous: Option<Display>, _scale: f32) -> Result<Display> {
        let monitors = super::super::shell_win::monitors();
        let pointer = super::super::shell_win::cursor_position().context("Couldn't locate the pointer")?;
        let bounds = match previous {
            Some(d) => monitors.into_iter().find(|r| *r == d.bounds),
            None => monitors.iter().copied().find(|r| r.contains(pointer)).or_else(|| monitors.first().copied()),
        }
        .context("That display is no longer connected. Start Show me again.")?;
        let at = bounds.center();
        let monitor = unsafe { MonitorFromPoint(POINT { x: at.x as i32, y: at.y as i32 }, MONITOR_DEFAULTTONEAREST) };
        let (mut x, mut y) = (0, 0);
        anyhow::ensure!(
            unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) } >= 0 && x > 0,
            "Couldn't read this display's scale"
        );
        Ok(Display { bounds, scale: x as f32 / 96.0 })
    }

    pub fn capture(&mut self, display: Display) -> Result<image::RgbaImage> {
        super::super::windows::capture_rectangle(display.bounds)
    }

    pub fn screen(&mut self, display: Display) -> Result<Screen> {
        let display = self.display(Some(display), display.scale)?;
        struct Read {
            display: Rect,
            windows: Vec<Window>,
        }
        unsafe extern "system" fn each(window: HWND, data: LPARAM) -> i32 {
            let read = unsafe { &mut *(data as *mut Read) };
            if unsafe { IsWindowVisible(window) } == 0 || own(window) {
                return 1;
            }
            let mut cloaked: u32 = 0;
            unsafe { DwmGetWindowAttribute(window, DWMWA_CLOAKED as u32, (&mut cloaked as *mut u32).cast(), 4) };
            if cloaked != 0 {
                return 1;
            }
            let mut r: RECT = unsafe { std::mem::zeroed() };
            if unsafe { GetWindowRect(window, &mut r) } != 0 {
                let bounds = Rect::from_min_max(pos2(r.left as f32, r.top as f32), pos2(r.right as f32, r.bottom as f32));
                if bounds.is_positive() && bounds.intersects(read.display) {
                    read.windows.push(Window { id: window as usize as u64, bounds });
                }
            }
            1
        }
        let mut read = Read { display: display.bounds, windows: Vec::new() };
        anyhow::ensure!(
            unsafe { EnumWindows(Some(each), (&mut read as *mut Read) as LPARAM) } != 0,
            "Couldn't observe the desktop windows"
        );
        let front = unsafe { GetForegroundWindow() };
        Ok(Screen { display, foreground: (!front.is_null() && !own(front)).then_some(front as usize as u64), windows: read.windows })
    }

    pub fn observe(&mut self) -> Result<()> {
        CLICKS.with(|q| q.borrow_mut().clear());
        self.hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), GetModuleHandleW(ptr::null()), 0) };
        anyhow::ensure!(!self.hook.is_null(), "Windows didn't allow click observation");
        Ok(())
    }

    pub fn clicks(&mut self) -> Result<Vec<Pos2>> {
        // WH_MOUSE_LL callbacks run on the installing thread, which must keep pumping messages.
        let mut message: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(CLICKS.with(|q| q.borrow_mut().drain(..).collect()))
    }

    pub fn reduce_motion() -> bool {
        let mut enabled: i32 = 0;
        // An unavailable setting is still, never an assumption that animations are allowed.
        (unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut enabled as *mut i32).cast(), 0) }) == 0 || enabled == 0
    }

    pub fn place(title: &str, display: Display) -> bool {
        use super::super::{shell::Role, shell_win};
        let Some(window) = shell_win::find(title) else { return false };
        if !shell_win::adopt(title, Role::Pill) {
            return false;
        }
        unsafe {
            let style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(window, GWL_EXSTYLE, (style | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED) as isize);
            // Physical desktop coordinates, including negative origins and mixed-DPI monitors.
            SetWindowPos(
                window,
                HWND_TOPMOST,
                display.bounds.min.x.round() as i32,
                display.bounds.min.y.round() as i32,
                display.bounds.width().round() as i32,
                display.bounds.height().round() as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) != 0
        }
    }

    pub fn hide(title: &str) {
        if let Some(window) = super::super::shell_win::find(title) {
            unsafe { ShowWindow(window, SW_HIDE) };
        }
    }
}

fn own(window: HWND) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    pid == std::process::id()
}

unsafe extern "system" fn mouse(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && matches!(message as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN) {
        // The hook only copies coordinates into a bounded local queue. No capture, window
        // enumeration, engine call or logging is done on this timing-sensitive callback.
        let point = unsafe { &*(data as *const MSLLHOOKSTRUCT) }.pt;
        CLICKS.with(|q| {
            let mut q = q.borrow_mut();
            if q.len() < 32 {
                q.push_back(pos2(point.x as f32, point.y as f32));
            }
        });
    }
    // Never consume, synthesize or modify the person's click.
    unsafe { CallNextHookEx(ptr::null_mut(), code, message, data) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput},
    };

    static CLICKED_WINDOW: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "system" fn target(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if message == WM_LBUTTONDOWN {
            CLICKED_WINDOW.store(window as usize, Ordering::SeqCst);
        }
        unsafe { DefWindowProcW(window, message, w, l) }
    }

    struct Windows(Vec<HWND>);
    impl Drop for Windows {
        fn drop(&mut self) {
            for window in &self.0 {
                unsafe { DestroyWindow(*window) };
            }
        }
    }

    /// Deliberately opt-in: synthesizes one click on the disposable Windows CI desktop.
    #[test]
    #[ignore = "requires an isolated interactive Windows desktop; moves the mouse and creates test windows"]
    fn native_windows_overlay_passes_clicks_through_without_taking_focus() -> Result<()> {
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<_>>();
        let name = wide(&format!("LanternOverlayTest-{}", uuid::Uuid::new_v4()));
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        let class =
            WNDCLASSW { lpfnWndProc: Some(target), hInstance: instance, lpszClassName: name.as_ptr(), ..unsafe { std::mem::zeroed() } };
        anyhow::ensure!(unsafe { RegisterClassW(&class) } != 0, "Couldn't register native test window");
        let mut windows = Windows(Vec::new());
        let mut desktop = Desktop::connect()?;
        let display = desktop.display(None, 1.0)?;
        let at = display.bounds.min + eframe::egui::vec2(150.0, 150.0);
        let target_title = wide("Lantern overlay test target");
        let under = unsafe {
            CreateWindowExW(
                0,
                name.as_ptr(),
                target_title.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                at.x as i32,
                at.y as i32,
                200,
                200,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        anyhow::ensure!(!under.is_null(), "Couldn't create target window");
        windows.0.push(under);
        unsafe { SetForegroundWindow(under) };
        let before = unsafe { GetForegroundWindow() };
        let title = format!("October Lantern · Show me native-{}", uuid::Uuid::new_v4());
        let text = wide(&title);
        let overlay = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                name.as_ptr(),
                text.as_ptr(),
                WS_POPUP,
                0,
                0,
                10,
                10,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        anyhow::ensure!(!overlay.is_null(), "Couldn't create overlay window");
        windows.0.push(overlay);
        unsafe { SetLayeredWindowAttributes(overlay, 0, 0, LWA_ALPHA) };
        desktop.observe()?;
        assert!(Desktop::place(&title, display));
        assert_eq!(unsafe { GetForegroundWindow() }, before, "overlay took focus");
        let style = unsafe { GetWindowLongPtrW(overlay, GWL_EXSTYLE) } as u32;
        assert_eq!(
            style & (WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_TOPMOST),
            WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_TOPMOST
        );
        let mut bounds: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetWindowRect(overlay, &mut bounds) };
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (display.bounds.min.x as i32, display.bounds.min.y as i32, display.bounds.max.x as i32, display.bounds.max.y as i32)
        );
        unsafe { SetCursorPos(at.x as i32 + 50, at.y as i32 + 50) };
        let input =
            |flag| INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flag, ..unsafe { std::mem::zeroed() } } } };
        let inputs = [input(MOUSEEVENTF_LEFTDOWN), input(MOUSEEVENTF_LEFTUP)];
        assert_eq!(unsafe { SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) }, 2);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = false;
        while !observed || CLICKED_WINDOW.load(Ordering::SeqCst) != under as usize {
            observed |= !desktop.clicks()?.is_empty();
            assert!(Instant::now() < deadline, "click didn't reach both target and observer");
            std::thread::sleep(Duration::from_millis(10));
        }
        Desktop::hide(&title);
        assert_eq!(unsafe { IsWindowVisible(overlay) }, 0);
        let image = desktop.capture(display)?;
        assert_eq!(image.dimensions(), (display.bounds.width() as u32, display.bounds.height() as u32));
        Ok(())
    }
}
