//! The native window a plugin's editor lives in. A plugin draws its own interface into a window the
//! host gives it, and egui has no such thing to offer, so this is a plain Win32 window. Its messages
//! are delivered by the message loop the main window already runs on this thread.
//!
//! The window has a bar across its top that is ours (bypass, presets, A/B compare), drawn here with
//! GDI, and under it a child window that is what the plugin embeds itself in. The plugin only ever
//! hears of the size of that child.
use crate::{HeaderClick, HeaderState, HostError};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Once;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, InvalidateRect, ScreenToClient, SelectObject, SetBkMode, SetTextColor,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER,
    DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, GetClientRect, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, IsIconic,
    LoadCursorW, MoveWindow, RegisterClassW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TrackPopupMenu, CS_DROPSHADOW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HTCAPTION,
    IDC_ARROW, MF_SEPARATOR, MF_STRING, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_RESTORE,
    SW_SHOW, TPM_NONOTIFY, TPM_RETURNCMD, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_ERASEBKGND,
    WM_LBUTTONUP, WM_NCHITTEST, WM_PAINT, WM_SIZE, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_POPUP,
    WS_THICKFRAME, WS_VISIBLE,
};

/// Moves each new editor along, so that they do not open exactly on top of each other.
static NEXT_PLACE: AtomicI32 = AtomicI32::new(0);

const CLASS_NAME: PCWSTR = w!("ProWavePluginEditor");
const CONTENT_CLASS: PCWSTR = w!("ProWavePluginContent");

/// How tall the bar is, in logical pixels.
const HEADER_HEIGHT: f64 = 28.0;
/// The title bar over it, and the close button at its end.
const TITLE_HEIGHT: f64 = 26.0;
const CLOSE_WIDTH: f64 = 40.0;
const MARGIN: f64 = 6.0;
const GAP: f64 = 4.0;
const BYPASS_WIDTH: f64 = 56.0;
const COMPARE_WIDTH: f64 = 76.0;
const COPY_WIDTH: f64 = 70.0;
/// The preset button takes the room that is left, between these.
const PRESET_MIN: f64 = 72.0;
const PRESET_MAX: f64 = 260.0;

/// Where the bar's buttons go in a window `width` physical pixels wide: each with its left and right
/// edge. In a narrow window the ones at the end give way, so the window never has to be wider than
/// the plugin.
fn header_cells(width: i32, scale: f64) -> Vec<(HeaderClick, i32, i32)> {
    let px = |logical: f64| (logical * scale).round() as i32;
    let room = |extras: &[f64]| {
        let taken: f64 = extras.iter().map(|w| w + GAP).sum();
        f64::from(width) / scale - MARGIN * 2.0 - BYPASS_WIDTH - GAP - taken
    };
    let (compare, copy) = if room(&[COMPARE_WIDTH, COPY_WIDTH]) >= PRESET_MIN {
        (true, true)
    } else if room(&[COMPARE_WIDTH]) >= PRESET_MIN {
        (true, false)
    } else {
        (false, false)
    };
    let extras: &[f64] = match (compare, copy) {
        (true, true) => &[COMPARE_WIDTH, COPY_WIDTH],
        (true, false) => &[COMPARE_WIDTH],
        _ => &[],
    };
    let presets = room(extras).clamp(0.0, PRESET_MAX);
    let mut cells = Vec::with_capacity(4);
    let mut x = MARGIN;
    let mut add = |click, w: f64| {
        cells.push((click, px(x), px(x + w)));
        x += w + GAP;
    };
    add(HeaderClick::Bypass, BYPASS_WIDTH);
    add(HeaderClick::Presets, presets);
    if compare {
        add(HeaderClick::Compare, COMPARE_WIDTH);
    }
    if copy {
        add(HeaderClick::CopyCompare, COPY_WIDTH);
    }
    cells
}

/// What a button says.
fn header_label(click: HeaderClick, state: &HeaderState) -> String {
    match click {
        HeaderClick::Bypass => "Bypass".to_string(),
        HeaderClick::Presets => match &state.preset {
            Some(name) => format!("{name} ▾"),
            None => "Presets ▾".to_string(),
        },
        HeaderClick::Compare if state.on_b => "Compare: B".to_string(),
        HeaderClick::Compare => "Compare: A".to_string(),
        HeaderClick::CopyCompare if state.on_b => "Copy B to A".to_string(),
        HeaderClick::CopyCompare => "Copy A to B".to_string(),
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

/// Physical pixels to one logical pixel of this window.
fn scale_of(hwnd: HWND) -> f64 {
    // SAFETY: `hwnd` is a window of this thread that is still there.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        f64::from(dpi) / 96.0
    }
}

fn title_height(hwnd: HWND) -> i32 {
    (TITLE_HEIGHT * scale_of(hwnd)).round() as i32
}

fn close_width(hwnd: HWND) -> i32 {
    (CLOSE_WIDTH * scale_of(hwnd)).round() as i32
}

/// The title bar and the button bar together: what is above the plugin.
fn header_height(hwnd: HWND) -> i32 {
    title_height(hwnd) + (HEADER_HEIGHT * scale_of(hwnd)).round() as i32
}

/// Draws the bar.
///
/// SAFETY: `hwnd` must be the editor window, handling `WM_PAINT` on its own thread.
unsafe fn paint_header(hwnd: HWND, state: &HeaderState, title: &str) {
    let scale = scale_of(hwnd);
    let height = header_height(hwnd);
    let top = title_height(hwnd);
    let mut client = RECT::default();
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: the handles made here are given back here, and every pointer is to a local.
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        let dc = BeginPaint(hwnd, &mut paint);
        let fill = |rect: &RECT, color: COLORREF| {
            let brush = CreateSolidBrush(color);
            FillRect(dc, rect, brush);
            let _ = DeleteObject(brush.into());
        };
        fill(
            &RECT {
                bottom: height,
                ..client
            },
            rgb(0x3a, 0x3a, 0x3e),
        );
        let font = CreateFontW(
            -((12.0 * scale).round() as i32),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            u32::from(DEFAULT_PITCH.0),
            w!("Segoe UI"),
        );
        let old = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        // The title bar: the plugin's name, and the close button at the end.
        fill(
            &RECT {
                bottom: top,
                ..client
            },
            rgb(0x26, 0x26, 0x2a),
        );
        SetTextColor(dc, rgb(0xd8, 0xd8, 0xdc));
        let mut name = RECT {
            left: (10.0 * scale).round() as i32,
            right: client.right - close_width(hwnd),
            top: 0,
            bottom: top,
        };
        let mut label: Vec<u16> = title.encode_utf16().collect();
        DrawTextW(
            dc,
            &mut label,
            &mut name,
            DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        let mut close = RECT {
            left: client.right - close_width(hwnd),
            right: client.right,
            top: 0,
            bottom: top,
        };
        let mut cross: Vec<u16> = "✕".encode_utf16().collect();
        DrawTextW(
            dc,
            &mut cross,
            &mut close,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        let inset = (3.0 * scale).round() as i32;
        for (click, left, right) in header_cells(client.right, scale) {
            let mut cell = RECT {
                left,
                right,
                top: top + inset,
                bottom: height - inset,
            };
            let lit = match click {
                HeaderClick::Bypass => state.bypass,
                HeaderClick::Compare => state.on_b,
                _ => false,
            };
            let (back, text) = if lit {
                (rgb(0xd9, 0xa0, 0x2a), rgb(0x10, 0x10, 0x10))
            } else {
                (rgb(0x55, 0x55, 0x5a), rgb(0xf0, 0xf0, 0xf0))
            };
            fill(&cell, back);
            cell.left += inset;
            cell.right -= inset;
            SetTextColor(dc, text);
            let mut label: Vec<u16> = header_label(click, state).encode_utf16().collect();
            DrawTextW(
                dc,
                &mut label,
                &mut cell,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }
        SelectObject(dc, old);
        let _ = DeleteObject(font.into());
        let _ = EndPaint(hwnd, &paint);
    }
}

/// What the window procedure notes for the UI thread to act on, where the plugin can be reached.
#[derive(Default)]
struct Inbox {
    close: AtomicBool,
    /// The size of the part the plugin has.
    width: AtomicI32,
    height: AtomicI32,
    /// The child window the plugin embeds itself in.
    content: Cell<isize>,
    /// What the bar shows, and the button last pressed. Only ever touched on the window's thread.
    header: RefCell<HeaderState>,
    click: Cell<Option<HeaderClick>>,
    /// The plugin's name, in the title bar.
    title: RefCell<String>,
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the pointer is set right after the window is made, from the Box in `EditorWindow`,
    // which destroys this window before it is dropped. Until it is set this reads 0. The content
    // window's class has its own procedure, so this is only ever the editor window.
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
            let bar = header_height(hwnd);
            let (width, height) = (
                (lparam.0 & 0xffff) as i32,
                ((lparam.0 >> 16) & 0xffff) as i32 - bar,
            );
            if width > 0 && height > 0 {
                inbox.width.store(width, Ordering::Release);
                inbox.height.store(height, Ordering::Release);
                let content = HWND(inbox.content.get() as *mut c_void);
                // SAFETY: the child lives as long as this window. The bar is drawn anew.
                unsafe {
                    let _ = MoveWindow(content, 0, bar, width, height, true);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        (WM_NCHITTEST, Some(_)) => {
            // The title bar is ours, so it has to say that it moves the window.
            let mut at = POINT {
                x: i32::from((lparam.0 & 0xffff) as u16 as i16),
                y: i32::from(((lparam.0 >> 16) & 0xffff) as u16 as i16),
            };
            let mut client = RECT::default();
            // SAFETY: both are valid locals, and `hwnd` is this window.
            let inside = unsafe {
                let _ = ScreenToClient(hwnd, &mut at);
                GetClientRect(hwnd, &mut client).is_ok()
            };
            if inside
                && (0..title_height(hwnd)).contains(&at.y)
                && (0..client.right - close_width(hwnd)).contains(&at.x)
            {
                LRESULT(HTCAPTION as isize)
            } else {
                // SAFETY: the arguments are the ones this procedure was called with.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        (WM_ERASEBKGND, Some(_)) => LRESULT(1),
        (WM_PAINT, Some(inbox)) => {
            // SAFETY: this is the editor window, on its own thread, asked to paint.
            unsafe { paint_header(hwnd, &inbox.header.borrow(), &inbox.title.borrow()) };
            LRESULT(0)
        }
        (WM_LBUTTONUP, Some(inbox)) => {
            let (x, y) = (
                i32::from((lparam.0 & 0xffff) as u16 as i16),
                i32::from(((lparam.0 >> 16) & 0xffff) as u16 as i16),
            );
            let mut client = RECT::default();
            // SAFETY: `client` is a valid local and `hwnd` is this window.
            let _ = unsafe { GetClientRect(hwnd, &mut client) };
            if (0..title_height(hwnd)).contains(&y) && x >= client.right - close_width(hwnd) {
                // SAFETY: `hwnd` is this window, on its own thread.
                let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
                inbox.close.store(true, Ordering::Release);
            } else if y >= title_height(hwnd) && y < header_height(hwnd) {
                let hit = header_cells(client.right, scale_of(hwnd))
                    .into_iter()
                    .find(|(_, left, right)| (*left..*right).contains(&x));
                if let Some((click, ..)) = hit {
                    inbox.click.set(Some(click));
                }
            }
            LRESULT(0)
        }
        // SAFETY: the arguments are the ones this procedure was called with.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The content window's procedure: nothing of ours happens there.
unsafe extern "system" fn content_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the arguments are the ones this procedure was called with.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn register_class() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // SAFETY: the class is made of a static name and a procedure that lives as long as the
        // program, and registering twice is prevented here. A failure shows when the window is made.
        unsafe {
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW,
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS_NAME,
                ..WNDCLASSW::default()
            };
            RegisterClassW(&class);
            // What the plugin embeds itself in: its messages need nothing from us.
            let content = WNDCLASSW {
                lpfnWndProc: Some(content_proc),
                hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CONTENT_CLASS,
                ..WNDCLASSW::default()
            };
            RegisterClassW(&content);
        }
    });
}

/// No frame of the system's: the title bar is ours. A plugin that can be resized gets a thin border
/// to drag.
fn popup_style(resizable: bool) -> WINDOW_STYLE {
    let fixed = WS_POPUP | WS_CLIPCHILDREN;
    if resizable {
        fixed | WS_THICKFRAME
    } else {
        fixed
    }
}

/// A hidden window for one plugin editor with Pro Wave's own title bar. Owned by the main window: it
/// stays above it, has no place in the taskbar, and hides and goes away with it.
pub(crate) struct EditorWindow {
    hwnd: HWND,
    /// The child window under the bar, which goes with `hwnd`.
    content: HWND,
    look: (WINDOW_STYLE, WINDOW_EX_STYLE),
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
        let owner = owner.map(|h| HWND(h as *mut c_void));
        let look = (popup_style(resizable), WINDOW_EX_STYLE(0));
        // Over the main window, each a little further along than the last.
        let usual = {
            let step = NEXT_PLACE.fetch_add(1, Ordering::Relaxed).rem_euclid(8) * 28;
            let mut main = RECT::default();
            if let Some(owner) = owner {
                // SAFETY: `main` is a valid local. If the window is gone this only fails.
                let _ = unsafe { GetWindowRect(owner, &mut main) };
            }
            (main.left + 220 + step, main.top + 140 + step)
        };
        let (x, y) = position.unwrap_or(usual);
        // SAFETY: the class is registered, the title outlives the call and `owner` came from the
        // main window, which lives until the program ends.
        let hwnd = unsafe {
            CreateWindowExW(
                look.1,
                CLASS_NAME,
                &HSTRING::from(title),
                look.0,
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
        // SAFETY: the class is registered and `hwnd` was just made. Destroying `hwnd` destroys this
        // child with it.
        let content = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CONTENT_CLASS,
                None,
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
                0,
                header_height(hwnd),
                640,
                480,
                Some(hwnd),
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                None,
            )
        }
        .map_err(|e| HostError(format!("cannot make the editor window: {e}")));
        let content = match content {
            Ok(content) => content,
            Err(e) => {
                // SAFETY: `hwnd` was just made here, and nothing else has it.
                let _ = unsafe { DestroyWindow(hwnd) };
                return Err(e);
            }
        };
        inbox.content.set(content.0 as isize);
        *inbox.title.borrow_mut() = title.to_string();
        // SAFETY: `hwnd` was just made. The Box keeps the pointer valid until `drop` below, which
        // destroys the window first.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*inbox as *const Inbox as isize) };
        Ok(Self {
            hwnd,
            content,
            look,
            inbox,
        })
    }

    /// The handle a plugin embeds itself in.
    pub(crate) fn handle(&self) -> *mut c_void {
        self.content.0
    }

    /// What the bar shows from now on.
    pub(crate) fn show_header(&self, state: &HeaderState) {
        if *self.inbox.header.borrow() != *state {
            *self.inbox.header.borrow_mut() = state.clone();
            // SAFETY: `self.hwnd` lives as long as `self`.
            let _ = unsafe { InvalidateRect(Some(self.hwnd), None, false) };
        }
    }

    /// The button pressed in the bar since last asked.
    pub(crate) fn take_header_click(&self) -> Option<HeaderClick> {
        self.inbox.click.take()
    }

    /// Shows a menu under the bar's preset button and waits for the choice, by its place in
    /// `items`. An empty item is a separator.
    pub(crate) fn header_menu(&self, items: &[String]) -> Option<usize> {
        let mut client = RECT::default();
        // SAFETY: the menu made here is destroyed here, every string outlives the call that takes
        // it, and `self.hwnd` lives as long as `self`.
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
            let left = header_cells(client.right, self.scale())
                .into_iter()
                .find(|cell| cell.0 == HeaderClick::Presets)
                .map_or(0, |cell| cell.1);
            let mut at = POINT {
                x: left,
                y: header_height(self.hwnd),
            };
            let _ = ClientToScreen(self.hwnd, &mut at);
            let menu = CreatePopupMenu().ok()?;
            for (i, item) in items.iter().enumerate() {
                let _ = if item.is_empty() {
                    AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())
                } else {
                    AppendMenuW(menu, MF_STRING, i + 1, &HSTRING::from(item.as_str()))
                };
            }
            let _ = SetForegroundWindow(self.hwnd);
            let picked = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                at.x,
                at.y,
                None,
                self.hwnd,
                None,
            );
            let _ = DestroyMenu(menu);
            (picked.0 as usize).checked_sub(1)
        }
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

    /// Sizes the window so that the part the plugin has is `width` by `height`.
    pub(crate) fn set_client_size(&self, width: u32, height: u32) {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32 + header_height(self.hwnd),
        };
        // SAFETY: `rect` is a valid local and `self.hwnd` lives as long as `self`.
        unsafe {
            let _ = AdjustWindowRectEx(&mut rect, self.look.0, false, self.look.1);
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
        self.keep_inside();
        // SAFETY: `self.hwnd` lives as long as `self`.
        let _ = unsafe { ShowWindow(self.hwnd, SW_SHOW) };
    }

    /// Moves it to where it can be seen: a place saved with another screen plugged in may be off
    /// all of them now.
    fn keep_inside(&self) {
        let mut mine = RECT::default();
        // SAFETY: `mine` is a valid local and `self.hwnd` lives as long as `self`.
        unsafe {
            if GetWindowRect(self.hwnd, &mut mine).is_err() {
                return;
            }
            let (left, top) = (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
            );
            let (right, bottom) = (
                left + GetSystemMetrics(SM_CXVIRTUALSCREEN),
                top + GetSystemMetrics(SM_CYVIRTUALSCREEN),
            );
            let x = mine.left.clamp(left, (right - 120).max(left));
            let y = mine.top.clamp(top, (bottom - 60).max(top));
            if (x, y) != (mine.left, mine.top) {
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
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
        use windows::Win32::UI::WindowsAndMessaging::{GetWindow, IsWindowVisible, GW_CHILD};
        let mut rect = RECT::default();
        // SAFETY: `rect` is a valid local and both windows live as long as `self`.
        unsafe {
            let _ = GetClientRect(self.content, &mut rect);
            (
                IsWindowVisible(self.hwnd).as_bool(),
                (rect.right, rect.bottom),
                GetWindow(self.content, GW_CHILD).is_ok(),
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
