//! Hold a per-session Windows marker so installers never close active SSH work.
use std::{
    io,
    os::windows::io::{FromRawHandle as _, OwnedHandle},
};

// The packaging helper reads this constant and passes it to Inno Setup.
pub const APP_MUTEX_NAME: &str = r"Local\io.github.last-refuge.ghostlink.running";

pub fn running_marker() -> io::Result<OwnedHandle> {
    let name: Vec<u16> = APP_MUTEX_NAME.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the name is NUL-terminated and remains live for this call. Default
    // security is used; ownership is not requested, so multiple app processes
    // can hold the marker at once.
    let handle = unsafe {
        windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, name.as_ptr())
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: CreateMutexW returned a valid owned kernel handle.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}
