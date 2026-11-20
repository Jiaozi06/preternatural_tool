use std::collections::{HashMap, HashSet};
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

pub struct RunningApp {
    name: String,
    pub path: String,
    pub title: String,
    pid: u32,
}

pub fn foreground_pid() -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
    pid
}

pub fn same_path(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
}

fn process_path(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut value = vec![0; 32768];
    let mut length = value.len() as u32;
    let ok = unsafe {
        let result = QueryFullProcessImageNameW(process, 0, value.as_mut_ptr(), &mut length);
        CloseHandle(process);
        result != 0
    };
    ok.then(|| String::from_utf16_lossy(&value[..length as usize]))
}

pub fn running_apps() -> Vec<RunningApp> {
    let mut windows: HashMap<u32, String> = HashMap::new();
    unsafe {
        EnumWindows(
            Some(collect_window),
            (&mut windows as *mut HashMap<u32, String>) as LPARAM,
        )
    };
    let mut apps: Vec<_> = windows
        .into_iter()
        .filter_map(|(pid, title)| {
            let path = process_path(pid)?;
            Some(RunningApp {
                name: name(&path).to_owned(),
                path,
                title,
                pid,
            })
        })
        .collect();
    apps.sort_unstable_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.pid.cmp(&b.pid))
    });
    let mut paths = HashSet::new();
    apps.retain(|app| paths.insert(app.path.to_ascii_lowercase()));
    apps
}

unsafe extern "system" fn collect_window(window: HWND, data: LPARAM) -> i32 {
    if IsWindowVisible(window) == 0 {
        return 1;
    }
    let style = GetWindowLongW(window, GWL_EXSTYLE) as u32;
    if style & WS_EX_TOOLWINDOW != 0 && style & WS_EX_APPWINDOW == 0 {
        return 1;
    }
    let mut cloaked: u32 = 0;
    DwmGetWindowAttribute(
        window,
        DWMWA_CLOAKED as u32,
        (&mut cloaked as *mut u32).cast(),
        4,
    );
    if cloaked != 0 {
        return 1;
    }
    let mut pid = 0;
    GetWindowThreadProcessId(window, &mut pid);
    if pid == 0 || pid == GetCurrentProcessId() {
        return 1;
    }
    let mut title = [0; 512];
    let length = GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32);
    if length > 0 {
        let windows = &mut *(data as *mut HashMap<u32, String>);
        windows
            .entry(pid)
            .or_insert_with(|| String::from_utf16_lossy(&title[..length as usize]));
    }
    1
}

pub struct ForegroundTarget {
    path: String,
    last_pid: u32,
    matched: bool,
}

impl ForegroundTarget {
    pub fn new(path: String) -> Self {
        Self {
            path,
            last_pid: 0,
            matched: false,
        }
    }

    pub fn matching_pid(&mut self) -> u32 {
        let pid = foreground_pid();
        if pid != self.last_pid {
            self.last_pid = pid;
            self.matched =
                pid != 0 && process_path(pid).is_some_and(|path| same_path(&self.path, &path));
        }
        if self.matched {
            pid
        } else {
            0
        }
    }
}
