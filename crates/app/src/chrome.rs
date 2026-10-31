//! Match the native Windows caption to the app's graphite palette.
#[cfg(windows)]
pub fn apply(window: &gpui::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
    };
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    // DWM color attributes use COLORREF (0x00bbggrr), not our UI's RGB order.
    for (attribute, value) in [
        (DWMWA_USE_IMMERSIVE_DARK_MODE, 1u32),
        (DWMWA_CAPTION_COLOR, 0x0023201b),
        (DWMWA_TEXT_COLOR, 0x00dfe3dc),
    ] {
        // SAFETY: GPUI owns this live HWND; DWM copies this correctly sized
        // BOOL/COLORREF synchronously. Unsupported cosmetic attributes are nonfatal.
        unsafe {
            let _ = DwmSetWindowAttribute(
                handle.hwnd.get() as _,
                attribute as u32,
                (&value as *const u32).cast(),
                size_of::<u32>() as u32,
            );
        }
    }
}
#[cfg(not(windows))]
pub fn apply(_: &gpui::Window) {}
