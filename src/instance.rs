use std::io;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetLastError, ERROR_ALREADY_EXISTS, HANDLE,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId};

pub struct Instance(HANDLE);

impl Instance {
    pub fn acquire() -> io::Result<Option<Self>> {
        let name = wide("Local\\LvwanMutationToolbox.Instance");
        unsafe {
            SetLastError(0);
            let handle = CreateMutexW(null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let already_exists = GetLastError() == ERROR_ALREADY_EXISTS;
            let instance = Self(handle);
            if already_exists || other_window() {
                Ok(None)
            } else {
                Ok(Some(instance))
            }
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub fn other_window() -> bool {
    let class = wide("Window Class");
    let title = wide(crate::WINDOW_TITLE);
    let current = unsafe { GetCurrentProcessId() };
    let mut window = null_mut();
    loop {
        window = unsafe { FindWindowExW(null_mut(), window, class.as_ptr(), title.as_ptr()) };
        if window.is_null() {
            return false;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid != 0 && pid != current {
            return true;
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
