use std::cell::Cell;
use std::io;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::{Shell::*, WindowsAndMessaging::*};

const TRAY_CALLBACK: u32 = WM_APP + 1;
const RESTORE: u32 = 1;
const EXIT: u32 = 2;

pub struct Tray(Box<TrayState>);

struct TrayState {
    window: Cell<HWND>,
    icon: HICON,
    visible: Cell<bool>,
    taskbar_created: u32,
}

impl Tray {
    pub fn install(window: HWND) -> io::Result<Self> {
        let taskbar_created = wide("TaskbarCreated");
        let state = Box::new(TrayState {
            window: Cell::new(window),
            icon: unsafe { LoadIconW(GetModuleHandleW(null()), 1usize as *const u16) },
            visible: Cell::new(false),
            taskbar_created: unsafe { RegisterWindowMessageW(taskbar_created.as_ptr()) },
        });
        if state.icon.is_null() {
            return Err(io::Error::last_os_error());
        }
        unsafe {
            ChangeWindowMessageFilterEx(window, TRAY_CALLBACK, MSGFLT_ALLOW, null_mut());
            if state.taskbar_created != 0 {
                ChangeWindowMessageFilterEx(
                    window,
                    state.taskbar_created,
                    MSGFLT_ALLOW,
                    null_mut(),
                );
            }
            if SetWindowSubclass(
                window,
                Some(window_message),
                1,
                (&*state as *const TrayState) as usize,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self(state))
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.0.detach();
    }
}

impl TrayState {
    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.window.get(),
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: TRAY_CALLBACK,
            hIcon: self.icon,
            ..Default::default()
        };
        for (slot, value) in data
            .szTip
            .iter_mut()
            .zip(crate::WINDOW_TITLE.encode_utf16())
        {
            *slot = value;
        }
        data
    }

    fn remove_icon(&self) {
        if self.visible.replace(false) {
            unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data()) };
        }
    }

    fn detach(&self) {
        self.remove_icon();
        let window = self.window.replace(null_mut());
        if !window.is_null() {
            unsafe { RemoveWindowSubclass(window, Some(window_message), 1) };
        }
    }

    fn minimize(&self) -> bool {
        if !self.visible.get() {
            if unsafe { Shell_NotifyIconW(NIM_ADD, &self.data()) } == 0 {
                return false;
            }
            self.visible.set(true);
        }
        unsafe { ShowWindow(self.window.get(), SW_HIDE) };
        true
    }

    fn restore(&self) {
        let window = self.window.get();
        if window.is_null() {
            return;
        }
        unsafe {
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
        self.remove_icon();
    }

    fn menu(&self) {
        let window = self.window.get();
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        let mut position = POINT::default();
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                RESTORE as usize,
                wide("打开工具箱").as_ptr(),
            );
            AppendMenuW(menu, MF_STRING, EXIT as usize, wide("退出").as_ptr());
            GetCursorPos(&mut position);
            SetForegroundWindow(window);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                position.x,
                position.y,
                0,
                window,
                null(),
            );
            DestroyMenu(menu);
            match command as u32 {
                RESTORE => self.restore(),
                EXIT => {
                    PostMessageW(window, WM_CLOSE, 0, 0);
                }
                _ => {
                    PostMessageW(window, WM_NULL, 0, 0);
                }
            }
        }
    }
}

unsafe extern "system" fn window_message(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    state: usize,
) -> LRESULT {
    let state = &*(state as *const TrayState);
    match message {
        WM_SYSCOMMAND if (wparam as u32 & 0xfff0) == SC_MINIMIZE && state.minimize() => return 0,
        TRAY_CALLBACK => {
            match lparam as u32 {
                WM_LBUTTONUP => state.restore(),
                WM_RBUTTONUP => state.menu(),
                _ => {}
            }
            return 0;
        }
        WM_NCDESTROY => state.detach(),
        message if message == state.taskbar_created && state.visible.replace(false) => {
            if !state.minimize() {
                state.restore();
            }
        }
        _ => {}
    }
    DefSubclassProc(window, message, wparam, lparam)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
