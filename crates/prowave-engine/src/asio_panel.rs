//! An ASIO driver's own control panel. The audio library we use has no call for it, so this asks the
//! driver directly: its class is named in the registry, and the panel is one of its methods.
//!
//! The caller stops the audio first, so that the driver is free, and starts it again afterwards,
//! when any change made in the panel takes effect. The driver lives on a thread of its own for as
//! long as the panel may be open: some panels return at once and stay up until the driver is let go.

/// A driver's panel that was asked for. Dropping this lets the driver go, and with it the panel.
pub struct AsioPanel {
    #[cfg(all(windows, feature = "asio", target_pointer_width = "64"))]
    stop: Option<std::sync::mpsc::Sender<()>>,
    #[cfg(all(windows, feature = "asio", target_pointer_width = "64"))]
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(all(windows, feature = "asio", target_pointer_width = "64")))]
impl AsioPanel {
    pub fn open(_driver: &str, _owner: Option<isize>) -> Result<Self, String> {
        Err("This build has no ASIO".to_string())
    }
}

#[cfg(all(windows, feature = "asio", target_pointer_width = "64"))]
mod driver {
    use std::ffi::c_void;
    use windows::core::{IUnknown, Interface, GUID, PCWSTR};
    use windows::Win32::System::Com::{
        CLSIDFromString, CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    /// Places in the driver's table of methods, after the three every COM object starts with. From
    /// `iasiodrv.h` in the ASIO SDK.
    const INIT: usize = 3;
    const CONTROL_PANEL: usize = 21;
    const RELEASE: usize = 2;
    /// The driver says it has no panel.
    const ASE_NOT_PRESENT: i32 = -1000;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The driver's class, from where ASIO drivers register themselves.
    fn class_of(name: &str) -> Result<GUID, String> {
        let key = wide(&format!("SOFTWARE\\ASIO\\{name}"));
        let value = wide("CLSID");
        let mut text = [0u16; 64];
        let mut size = std::mem::size_of_val(&text) as u32;
        // SAFETY: the strings end in a zero and outlive the calls, and the buffer's size is given.
        unsafe {
            let found = RegGetValueW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(text.as_mut_ptr().cast()),
                Some(&mut size),
            );
            if found.is_err() {
                return Err(format!("{name} is not registered as an ASIO driver"));
            }
            CLSIDFromString(PCWSTR(text.as_ptr()))
                .map_err(|e| format!("{name} has a bad registration: {e}"))
        }
    }

    /// The method at `slot` of the object at `object`.
    ///
    /// SAFETY: `object` must be a live COM object with at least that many methods.
    unsafe fn method(object: *mut c_void, slot: usize) -> *const c_void {
        unsafe {
            let table = *(object as *const *const *const c_void);
            *table.add(slot)
        }
    }

    /// Runs on the panel's thread: makes the driver, asks for the panel, says how that went, and
    /// keeps the panel's messages moving until told to stop.
    pub(super) fn run(
        name: &str,
        owner: Option<isize>,
        result: &std::sync::mpsc::Sender<Result<(), String>>,
        stop: &std::sync::mpsc::Receiver<()>,
    ) {
        // SAFETY: this thread's own COM setup, undone below. The driver is created and used here
        // only, through the methods the ASIO SDK gives in this order, and released once.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let mut driver: *mut c_void = std::ptr::null_mut();
            let opened = class_of(name).and_then(|class| {
                let made: IUnknown = CoCreateInstance(&class, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| format!("{name} could not be opened: {e}"))?;
                // A driver answers to its own class as its interface.
                made.query(&class, &mut driver)
                    .ok()
                    .map_err(|e| format!("{name} could not be opened: {e}"))?;
                type Init = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
                let init: Init = std::mem::transmute(method(driver, INIT));
                if init(driver, owner.unwrap_or(0) as *mut c_void) == 0 {
                    return Err(format!("{name} would not start"));
                }
                type Panel = unsafe extern "system" fn(*mut c_void) -> i32;
                let panel: Panel = std::mem::transmute(method(driver, CONTROL_PANEL));
                match panel(driver) {
                    ASE_NOT_PRESENT => Err(format!("{name} has no control panel")),
                    _ => Ok(()),
                }
            });
            let failed = opened.is_err();
            let _ = result.send(opened);
            while !failed
                && matches!(
                    stop.recv_timeout(std::time::Duration::from_millis(15)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                )
            {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if !driver.is_null() {
                type Release = unsafe extern "system" fn(*mut c_void) -> u32;
                let release: Release = std::mem::transmute(method(driver, RELEASE));
                release(driver);
            }
            CoUninitialize();
        }
        let _ = IUnknown::IID;
    }
}

#[cfg(all(windows, feature = "asio", target_pointer_width = "64"))]
impl AsioPanel {
    /// Shows the panel of the driver called `driver`. Stop the audio first.
    pub fn open(driver: &str, owner: Option<isize>) -> Result<Self, String> {
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let name = driver.to_string();
        let thread = std::thread::spawn(move || driver::run(&name, owner, &result_tx, &stop_rx));
        let mut panel = Self {
            stop: Some(stop_tx),
            thread: Some(thread),
        };
        // A modal panel does not come back until it is closed. That is fine: the program goes on
        // drawing only once this answers, so give it a moment and carry on.
        match result_rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Err(e)) => {
                panel.finish();
                Err(e)
            }
            _ => Ok(panel),
        }
    }

    fn finish(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(all(windows, feature = "asio", target_pointer_width = "64"))]
impl Drop for AsioPanel {
    fn drop(&mut self) {
        self.finish();
    }
}
