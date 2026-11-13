//! The launch screen: the logo, what is being done and how far along, from the moment the program
//! starts until the main window has drawn. It runs on a thread of its own, so it keeps moving while
//! the main thread is busy starting the audio engine. Only Windows has one so far.

/// How long it stays up at the least, so that a fast start does not just flash it.
#[cfg(windows)]
const SHORTEST: std::time::Duration = std::time::Duration::from_millis(900);

#[cfg(windows)]
mod window {
    use super::SHORTEST;
    use std::sync::Mutex;
    use std::time::Instant;
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        AddFontMemResourceEx, BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
        CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect,
        InvalidateRect, SelectObject, SetBkMode, SetStretchBltMode, SetTextColor, StretchDIBits,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
        DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT,
        DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HALFTONE, HDC, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
        SRCCOPY, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{
        GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetMessageW, GetSystemMetrics, LoadCursorW, PostQuitMessage, RegisterClassW, SetTimer,
        SetWindowPos, TranslateMessage, CS_DROPSHADOW, HWND_TOPMOST, IDC_ARROW, MSG, SM_CXSCREEN,
        SM_CYSCREEN, SWP_NOACTIVATE, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WM_TIMER,
        WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
    };

    const CLASS: PCWSTR = w!("OvrtoneSplash");
    const WIDTH: f32 = 560.0;
    const HEIGHT: f32 = 340.0;
    const LOGO_WIDTH: f32 = 340.0;
    use super::SplashColors;

    /// A colour as Windows spells it.
    fn colorref([r, g, b]: [u8; 3]) -> COLORREF {
        COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
    }

    struct State {
        colors: SplashColors,
        /// The logo on the background colour, 4 bytes a pixel, blue first.
        logo: Vec<u8>,
        logo_size: (i32, i32),
        footer: String,
        step: String,
        target: f32,
        shown: f32,
        closing: bool,
        began: Instant,
    }

    static STATE: Mutex<Option<State>> = Mutex::new(None);

    /// `logo` is a dark mark on nothing, red, green, blue and alpha, `size` wide and high. It is drawn
    /// in the text colour, on the background, of `colors`.
    pub fn show(logo: &[u8], size: (u32, u32), footer: String, colors: SplashColors) {
        let Ok(mut state) = STATE.lock() else {
            return;
        };
        if state.is_some() || logo.len() != (size.0 * size.1 * 4) as usize {
            return;
        }
        let mut on_background = Vec::with_capacity(logo.len());
        for pixel in logo.as_chunks::<4>().0 {
            // How much of the pixel is the mark: dark and solid.
            let dark = 1.0 - (f32::from(pixel[0]) + f32::from(pixel[1]) + f32::from(pixel[2])) / 765.0;
            let cover = f32::from(pixel[3]) / 255.0 * dark;
            let mix = |k: usize| {
                let (under, on) = (f32::from(colors.background[k]), f32::from(colors.text[k]));
                (under + (on - under) * cover).round() as u8
            };
            on_background.extend_from_slice(&[mix(2), mix(1), mix(0), 255]);
        }
        *state = Some(State {
            colors,
            logo: on_background,
            logo_size: (size.0 as i32, size.1 as i32),
            footer,
            step: "Starting".to_string(),
            target: 0.05,
            shown: 0.0,
            closing: false,
            began: Instant::now(),
        });
        drop(state);
        let started = std::thread::Builder::new()
            .name("ovrtone-splash".into())
            .spawn(run);
        if started.is_err() {
            if let Ok(mut state) = STATE.lock() {
                *state = None;
            }
        }
    }

    pub fn step(text: &str, fraction: f32) {
        if let Ok(mut state) = STATE.lock() {
            if let Some(state) = state.as_mut() {
                state.step = text.to_string();
                state.target = fraction.clamp(0.0, 1.0);
            }
        }
    }

    pub fn close() {
        if let Ok(mut state) = STATE.lock() {
            if let Some(state) = state.as_mut() {
                state.closing = true;
                state.target = 1.0;
            }
        }
    }

    fn run() {
        static INTER: &[u8] = include_bytes!("../../ovrtone-app/assets/fonts/Inter-Regular.ttf");
        // SAFETY: what is passed lives through each call: the class's name and procedure and the
        // font's bytes are static, and `msg` is a local the call fills in. The window is used only
        // on this thread, which runs its message loop to the end.
        unsafe {
            let mut added = 0u32;
            let _ = AddFontMemResourceEx(
                INTER.as_ptr().cast(),
                INTER.len() as u32,
                None,
                std::ptr::addr_of_mut!(added),
            );
            let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let class = WNDCLASSW {
                style: CS_DROPSHADOW,
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS,
                ..WNDCLASSW::default()
            };
            RegisterClassW(&class);
            let made = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS,
                &HSTRING::from("Ovrtone"),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                WIDTH as i32,
                HEIGHT as i32,
                None,
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                None,
            );
            let Ok(hwnd) = made else {
                return;
            };
            let scale = scale_of(hwnd);
            let (w, h) = ((WIDTH * scale) as i32, (HEIGHT * scale) as i32);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                (GetSystemMetrics(SM_CXSCREEN) - w) / 2,
                (GetSystemMetrics(SM_CYSCREEN) - h) / 2,
                w,
                h,
                SWP_NOACTIVATE,
            );
            SetTimer(Some(hwnd), 1, 33, None);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if let Ok(mut state) = STATE.lock() {
            *state = None;
        }
    }

    /// SAFETY: `hwnd` must be a live window.
    unsafe fn scale_of(hwnd: HWND) -> f32 {
        GetDpiForWindow(hwnd) as f32 / 96.0
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_TIMER => {
                let mut finished = false;
                if let Ok(mut state) = STATE.lock() {
                    if let Some(state) = state.as_mut() {
                        state.shown += (state.target - state.shown) * 0.12;
                        finished = state.closing && state.began.elapsed() >= SHORTEST;
                    }
                }
                if finished {
                    let _ = DestroyWindow(hwnd);
                } else {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                paint(hwnd, hdc);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Draws everything on a picture first, then all at once, so that it does not flicker.
    ///
    /// SAFETY: `hwnd` and `hdc` must be the window and the device context of a paint in progress.
    unsafe fn paint(hwnd: HWND, hdc: HDC) {
        let Ok(guard) = STATE.lock() else {
            return;
        };
        let Some(state) = guard.as_ref() else {
            return;
        };
        let scale = scale_of(hwnd);
        let px = |v: f32| (v * scale).round() as i32;
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let (w, h) = (client.right, client.bottom);
        let memory = CreateCompatibleDC(Some(hdc));
        let picture = CreateCompatibleBitmap(hdc, w, h);
        let old_picture = SelectObject(memory, picture.into());
        let fill = |rect: RECT, color: COLORREF| {
            let brush = CreateSolidBrush(color);
            FillRect(memory, &rect, brush);
            let _ = DeleteObject(brush.into());
        };
        let colors = state.colors;
        fill(client, colorref(colors.background));
        // The logo.
        let (lw, lh) = state.logo_size;
        let shown_w = px(LOGO_WIDTH);
        let shown_h = shown_w * lh / lw.max(1);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: lw,
                biHeight: -lh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        SetStretchBltMode(memory, HALFTONE);
        StretchDIBits(
            memory,
            (w - shown_w) / 2,
            px(70.0),
            shown_w,
            shown_h,
            0,
            0,
            lw,
            lh,
            Some(state.logo.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        // The mark, its arcs going out like sound while the work goes on.
        let mark = px(78.0).max(8);
        let seconds = state.began.elapsed().as_secs_f32();
        let mark_pixels = super::mark_frame(mark as usize, seconds, &colors);
        let mark_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: mark,
                biHeight: -mark,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        StretchDIBits(
            memory,
            (w - mark) / 2,
            px(162.0),
            mark,
            mark,
            0,
            0,
            mark,
            mark,
            Some(mark_pixels.as_ptr().cast()),
            &mark_info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        // The words.
        let font = CreateFontW(
            -px(12.5),
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
            DEFAULT_PITCH.0.into(),
            w!("Inter"),
        );
        let old_font = SelectObject(memory, font.into());
        SetBkMode(memory, TRANSPARENT);
        let margin = px(28.0);
        let text = |words: &str, top: f32, color: COLORREF, right: bool| {
            let mut line: Vec<u16> = words.encode_utf16().collect();
            let mut rect = RECT {
                left: margin,
                top: px(top),
                right: w - margin,
                bottom: px(top + 18.0),
            };
            SetTextColor(memory, color);
            let side = if right { DT_RIGHT } else { DT_LEFT };
            DrawTextW(
                memory,
                &mut line,
                &mut rect,
                side | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            );
        };
        text(&state.step, 262.0, colorref(colors.text), false);
        text(&state.footer, 306.0, colorref(colors.dim), false);
        // How far along.
        let (bar_top, bar_bottom) = (px(288.0), px(291.0));
        let bar_w = w - margin * 2;
        fill(
            RECT {
                left: margin,
                top: bar_top,
                right: margin + bar_w,
                bottom: bar_bottom,
            },
            colorref(super::mix(colors.background, colors.text, 0.15)),
        );
        fill(
            RECT {
                left: margin,
                top: bar_top,
                right: margin + (bar_w as f32 * state.shown.clamp(0.0, 1.0)) as i32,
                bottom: bar_bottom,
            },
            colorref(super::mix(colors.background, colors.text, 0.75)),
        );
        let lit_w = (bar_w as f32 * state.shown.clamp(0.0, 1.0)) as i32;
        if lit_w > px(20.0) {
            let glint = px(36.0);
            let travel = lit_w + glint;
            let at = margin - glint + ((seconds * 0.6).fract() * travel as f32) as i32;
            fill(
                RECT {
                    left: at.max(margin),
                    top: bar_top,
                    right: (at + glint).min(margin + lit_w),
                    bottom: bar_bottom,
                },
                colorref(colors.text),
            );
        }
        let _ = BitBlt(hdc, 0, 0, w, h, Some(memory), 0, 0, SRCCOPY);
        SelectObject(memory, old_font);
        let _ = DeleteObject(font.into());
        SelectObject(memory, old_picture);
        let _ = DeleteObject(picture.into());
        let _ = DeleteDC(memory);
    }
}

/// The colours of the launch screen, red, green and blue: they come from the theme in use, so that the
/// screen looks like the program it opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplashColors {
    pub background: [u8; 3],
    pub text: [u8; 3],
    pub dim: [u8; 3],
}

impl Default for SplashColors {
    fn default() -> Self {
        Self {
            background: [0x17, 0x19, 0x1d],
            text: [0xe6, 0xe7, 0xea],
            dim: [0x8f, 0x92, 0x99],
        }
    }
}

/// `t` of the way from `a` to `b`.
fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    [0, 1, 2].map(|k| (f32::from(a[k]) + (f32::from(b[k]) - f32::from(a[k])) * t).round() as u8)
}

/// One frame of the mark, `size` pixels square, blue, green, red and a spare byte to a pixel: a ring,
/// and two arcs to its right that light in turn, going out like sound. Drawn here, edges smoothed, as
/// Windows' own drawing has rough ones.
fn mark_frame(size: usize, seconds: f32, colors: &SplashColors) -> Vec<u8> {
    // Blue, green, red, as the picture's pixels have them.
    let bgr = |c: [u8; 3]| [f32::from(c[2]), f32::from(c[1]), f32::from(c[0])];
    let background = bgr(colors.background);
    let ring = bgr(colors.text);
    let arc_bright = bgr(colors.text);
    let arc_dim = bgr(mix(colors.background, colors.text, 0.25));
    // The mark's geometry, on its own 512 grid: a ring and two arcs about the same centre.
    const CENTRE: (f32, f32) = (189.0, 256.0);
    let parts: [(f32, f32, f32); 3] = [(110.0, 44.0, 180.0), (188.0, 44.0, 56.0), (251.0, 30.0, 38.0)];
    let scale = size as f32 / 512.0;
    // A wave going out: each arc at its brightest a little after the one inside it.
    let glow = |k: usize| {
        let phase = (seconds * 1.4 - k as f32 * 0.35).rem_euclid(1.0);
        (1.0 - (phase * 2.0 - 1.0).abs()).powf(1.5)
    };
    let mut out = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (gx, gy) = (
                (x as f32 + 0.5) / scale - CENTRE.0,
                (y as f32 + 0.5) / scale - CENTRE.1,
            );
            let distance = (gx * gx + gy * gy).sqrt();
            let angle = gy.atan2(gx).to_degrees().abs();
            let mut color = background;
            for (k, (radius, width, half_angle)) in parts.iter().enumerate() {
                // How much of this pixel the part covers, in pixels from its edges.
                let across = (width / 2.0 - (distance - radius).abs()) * scale + 0.5;
                let along = if k == 0 {
                    1.0
                } else {
                    ((half_angle - angle) * distance.to_radians() * scale + 0.5).clamp(0.0, 1.0)
                };
                let cover = across.clamp(0.0, 1.0) * along;
                if cover <= 0.0 {
                    continue;
                }
                let ink = if k == 0 {
                    ring
                } else {
                    let g = glow(k - 1);
                    [0, 1, 2].map(|c| arc_dim[c] + (arc_bright[c] - arc_dim[c]) * g)
                };
                color = [0, 1, 2].map(|c| color[c] + (ink[c] - color[c]) * cover);
            }
            let at = (y * size + x) * 4;
            out[at] = color[0] as u8;
            out[at + 1] = color[1] as u8;
            out[at + 2] = color[2] as u8;
        }
    }
    out
}

/// Puts the screen up. `logo` is red, green, blue and alpha, `size` wide and high.
pub fn show(logo: &[u8], size: (u32, u32), footer: String, colors: SplashColors) {
    #[cfg(windows)]
    window::show(logo, size, footer, colors);
    #[cfg(not(windows))]
    let _ = (logo, size, footer, colors);
}

/// Says what is being done now, and how far along that is, 0..=1.
pub fn step(text: &str, fraction: f32) {
    #[cfg(windows)]
    window::step(text, fraction);
    #[cfg(not(windows))]
    let _ = (text, fraction);
}

/// The main window has drawn: the screen goes, once it has been up long enough to be read.
pub fn close() {
    #[cfg(windows)]
    window::close();
}

#[cfg(test)]
mod mark_tests {
    /// The mark's arcs change as it plays, and its ring is there all the time.
    #[test]
    fn the_mark_moves_and_keeps_its_ring() {
        let size = 96;
        let colors = super::SplashColors::default();
        let (a, b) = (super::mark_frame(size, 0.0, &colors), super::mark_frame(size, 0.37, &colors));
        assert_eq!(a.len(), size * size * 4);
        assert_ne!(a, b, "it moves");
        // A point on the ring: left of the centre by its radius.
        let scale = size as f32 / 512.0;
        let (x, y) = (((189.0 - 110.0) * scale) as usize, (256.0 * scale) as usize);
        let at = (y * size + x) * 4;
        assert!(a[at] > 150 && a[at] == b[at], "the ring is lit and still");
    }
}

#[cfg(all(test, windows))]
#[allow(clippy::unwrap_used)]
mod tests {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

    #[test]
    #[ignore = "puts a window on the screen"]
    fn the_screen_comes_up_and_goes() {
        let found = || unsafe { FindWindowW(w!("OvrtoneSplash"), None) }.is_ok();
        assert!(!found());
        super::show(&[255; 16], (2, 2), "Test".to_string(), super::SplashColors::default());
        std::thread::sleep(std::time::Duration::from_millis(700));
        assert!(found(), "it should be up");
        super::step("A step", 0.5);
        super::close();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(!found(), "it should have gone");
    }
}
