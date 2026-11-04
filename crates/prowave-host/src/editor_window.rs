//! The native window a plugin's editor lives in. A plugin draws its own interface into a window the
//! host gives it, and egui has no such thing to offer, so this is a plain Win32 window. Its messages
//! are delivered by the message loop the main window already runs on this thread.
use crate::HostError;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Once;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW,
    GetWindowRect, IsIconic, LoadCursorW, RegisterClassW, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, IDC_ARROW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_RESTORE, SW_SHOW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_SIZE, WNDCLASSW, WS_CAPTION, WS_EX_APPWINDOW, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_THICKFRAME,
};

const CLASS_NAME: PCWSTR = w!("ProWavePluginEditor");

/// What the window procedure notes for the UI thread to act on, where the plugin can be reached.
#[derive(Default)]
struct Inbox {
    close: AtomicBool,
    width: AtomicI32,
    height: AtomicI32,
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the pointer is set right after the window is made, from the Box in `EditorWindow`,
    // which destroys this window before it is dropped. Until it is set this reads 0.
    let inbox = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Inbox).as_ref() };
    match (msg, inbox) {
        (WM_CLOSE, Some(inbox)) => {
            // Hidden now for a prompt answer. The owner destroys it after the plugin's GUI.
            // SAFETY: `hwnd` is this window, on its own thread.
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            inbox.close.store(true, Ordering::Release);
            LRESULT(0)
        }
        (WM_SIZE, Some(inbox)) => {
            let (width, height) = (
                (lparam.0 & 0xffff) as i32,
                ((lparam.0 >> 16) & 0xffff) as i32,
            );
            if width > 0 && height > 0 {
                inbox.width.store(width, Ordering::Release);
                inbox.height.store(height, Ordering::Release);
            }
            LRESULT(0)
        }
        // SAFETY: the arguments are the ones this procedure was called with.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn register_class() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // SAFETY: the class is made of a static name and a procedure that lives as long as the
        // program, and registering twice is prevented here. A failure shows when the window is made.
        unsafe {
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS_NAME,
                ..WNDCLASSW::default()
            };
            RegisterClassW(&class);
        }
    });
}

fn style(resizable: bool) -> WINDOW_STYLE {
    let fixed = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    if resizable {
        fixed | WS_THICKFRAME | WS_MAXIMIZEBOX
    } else {
        fixed
    }
}

/// A hidden top-level window for one plugin editor, owned by the main window so it stays above it
/// and goes away with it.
pub(crate) struct EditorWindow {
    hwnd: HWND,
    resizable: bool,
    inbox: Box<Inbox>,
}

impl EditorWindow {
    pub(crate) fn new(
        title: &str,
        resizable: bool,
        owner: Option<isize>,
        position: Option<(i32, i32)>,
    ) -> Result<Self, HostError> {
        register_class();
        let inbox = Box::new(Inbox::default());
        let (x, y) = position.unwrap_or((CW_USEDEFAULT, CW_USEDEFAULT));
        let owner = owner.map(|h| HWND(h as *mut c_void));
        // SAFETY: the class is registered, the title outlives the call and `owner` came from the
        // main window, which lives until the program ends.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_APPWINDOW,
                CLASS_NAME,
                &HSTRING::from(title),
                style(resizable),
                x,
                y,
                640,
                480,
                owner,
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                None,
            )
        }
        .map_err(|e| HostError(format!("cannot make the editor window: {e}")))?;
        // SAFETY: `hwnd` was just made. The Box keeps the pointer valid until `drop` below, which
        // destroys the window first.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*inbox as *const Inbox as isize) };
        Ok(Self {
            hwnd,
            resizable,
            inbox,
        })
    }

    /// The handle a plugin embeds itself in.
    pub(crate) fn handle(&self) -> *mut c_void {
        self.hwnd.0
    }

    /// Physical pixels to one logical pixel on the window's screen.
    pub(crate) fn scale(&self) -> f64 {
        // SAFETY: `self.hwnd` lives as long as `self`.
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        if dpi == 0 {
            1.0
        } else {
            f64::from(dpi) / 96.0
        }
    }

    /// Sizes the window so that the area inside the frame is `width` by `height`.
    pub(crate) fn set_client_size(&self, width: u32, height: u32) {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        // SAFETY: `rect` is a valid local and `self.hwnd` lives as long as `self`.
        unsafe {
            let _ = AdjustWindowRectEx(
                &mut rect,
                style(self.resizable),
                false,
                WINDOW_EX_STYLE(WS_EX_APPWINDOW.0),
            );
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    pub(crate) fn show(&self) {
        // SAFETY: `self.hwnd` lives as long as `self`.
        let _ = unsafe { ShowWindow(self.hwnd, SW_SHOW) };
    }

    pub(crate) fn hide(&self) {
        // SAFETY: `self.hwnd` lives as long as `self`.
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    /// Brings an editor that is already open to the front.
    pub(crate) fn raise(&self) {
        // SAFETY: `self.hwnd` lives as long as `self`.
        unsafe {
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            }
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    /// Whether the user asked to close the window since last asked.
    pub(crate) fn take_close_request(&self) -> bool {
        self.inbox.close.swap(false, Ordering::AcqRel)
    }

    /// The size the window was last given, by the user or otherwise, since last asked.
    pub(crate) fn take_new_size(&self) -> Option<(u32, u32)> {
        let width = self.inbox.width.swap(0, Ordering::AcqRel);
        let height = self.inbox.height.swap(0, Ordering::AcqRel);
        (width > 0 && height > 0).then_some((width as u32, height as u32))
    }

    /// Where the window is on the screen, to open there next time.
    pub(crate) fn position(&self) -> Option<(i32, i32)> {
        let mut rect = RECT::default();
        // SAFETY: `rect` is a valid local and `self.hwnd` lives as long as `self`.
        let (got, minimized) = unsafe {
            (
                GetWindowRect(self.hwnd, &mut rect).is_ok(),
                IsIconic(self.hwnd).as_bool(),
            )
        };
        (got && !minimized).then_some((rect.left, rect.top))
    }

    #[cfg(test)]
    pub(crate) fn describe(&self) -> (bool, (i32, i32), bool) {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetClientRect, GetWindow, IsWindowVisible, GW_CHILD,
        };
        let mut rect = RECT::default();
        // SAFETY: `rect` is a valid local and `self.hwnd` lives as long as `self`.
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
            (
                IsWindowVisible(self.hwnd).as_bool(),
                (rect.right, rect.bottom),
                GetWindow(self.hwnd, GW_CHILD).is_ok(),
            )
        }
    }

    #[cfg(test)]
    pub(crate) fn request_close(&self) {
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
        // SAFETY: `self.hwnd` lives as long as `self`.
        unsafe { SendMessageW(self.hwnd, WM_CLOSE, None, None) };
    }
}

impl Drop for EditorWindow {
    fn drop(&mut self) {
        // SAFETY: this destroys the window on the thread that made it, once. The pointer to the
        // inbox is cleared first so no message reaches it during the destruction.
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Runs the messages of this thread's windows, where nothing else does (tests).
#[cfg(test)]
pub(crate) fn pump_messages_for(time: std::time::Duration) {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    let start = std::time::Instant::now();
    while start.elapsed() < time {
        let mut msg = MSG::default();
        // SAFETY: `msg` is a valid local filled in by the call before being used.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
