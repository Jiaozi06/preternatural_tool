use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

const WHEEL: u32 = 0x0800;
const KEY_UP: u32 = 0x0002;
const SCAN_CODE: u32 = 0x0008;

#[repr(C)]
#[derive(Clone, Copy)]
struct MouseInput {
    x: i32,
    y: i32,
    data: u32,
    flags: u32,
    time: u32,
    extra: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KeyboardInput {
    vk: u16,
    scan: u16,
    flags: u32,
    time: u32,
    extra: usize,
}

#[repr(C)]
union InputData {
    mouse: MouseInput,
    keyboard: KeyboardInput,
}

#[repr(C)]
pub struct Input {
    kind: u32,
    data: InputData,
}

#[repr(C)]
struct Message {
    window: *mut c_void,
    id: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: [i32; 2],
    private: u32,
}

#[repr(C)]
struct HookKey {
    vk: u32,
    scan: u32,
    flags: u32,
    time: u32,
    extra: usize,
}

#[repr(C)]
struct HookMouse {
    point: [i32; 2],
    data: u32,
    flags: u32,
    time: u32,
    extra: usize,
}

#[link(name = "user32")]
extern "system" {
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    fn MapVirtualKeyW(code: u32, kind: u32) -> u32;
    fn GetAsyncKeyState(code: i32) -> i16;
    fn SetWindowsHookExW(
        kind: i32,
        callback: extern "system" fn(i32, usize, isize) -> isize,
        module: *mut c_void,
        thread: u32,
    ) -> *mut c_void;
    fn CallNextHookEx(hook: *mut c_void, code: i32, wparam: usize, lparam: isize) -> isize;
    fn UnhookWindowsHookEx(hook: *mut c_void) -> i32;
    fn PeekMessageW(
        message: *mut Message,
        window: *mut c_void,
        min: u32,
        max: u32,
        flags: u32,
    ) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(module: *const u16) -> *mut c_void;
}

pub fn send(inputs: &[Input]) -> io::Result<()> {
    let count = inputs.len() as u32;
    let sent = unsafe { SendInput(count, inputs.as_ptr(), size_of::<Input>() as i32) };
    if sent == count {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "Windows 仅发送了 {sent}/{count} 个输入事件（{}）",
        io::Error::last_os_error()
    )))
}

pub fn scan(vk: u32) -> io::Result<u16> {
    let scan = unsafe { MapVirtualKeyW(vk, 0) } as u16;
    if scan == 0 {
        return Err(io::Error::other("无法获取按键扫描码"));
    }
    Ok(scan)
}

pub fn key(scan: u16, up: bool) -> Input {
    Input {
        kind: 1,
        data: InputData {
            keyboard: KeyboardInput {
                vk: 0,
                scan,
                flags: SCAN_CODE | if up { KEY_UP } else { 0 },
                time: 0,
                extra: OWN_INPUT,
            },
        },
    }
}

fn mouse(flags: u32, data: u32) -> Input {
    Input {
        kind: 0,
        data: InputData {
            mouse: MouseInput {
                x: 0,
                y: 0,
                data,
                flags,
                time: 0,
                extra: OWN_INPUT,
            },
        },
    }
}

pub fn wheel_down() -> Input {
    mouse(WHEEL, (-120_i32) as u32)
}

pub fn mouse_move(dx: i32, dy: i32) -> Input {
    Input {
        kind: 0,
        data: InputData {
            mouse: MouseInput {
                x: dx,
                y: dy,
                data: 0,
                flags: 0x0001,
                time: 0,
                extra: OWN_INPUT,
            },
        },
    }
}

pub fn mouse_button(vk: u32, up: bool) -> Option<Input> {
    let (down_flag, data) = match vk {
        1 => (0x0002, 0),
        2 => (0x0008, 0),
        4 => (0x0020, 0),
        5 => (0x0080, 1),
        6 => (0x0080, 2),
        _ => return None,
    };
    Some(mouse(if up { down_flag << 1 } else { down_flag }, data))
}

static BINDINGS: [AtomicUsize; 7] = [const { AtomicUsize::new(0) }; 7];
static SPACE_DOWN: AtomicUsize = AtomicUsize::new(0);
static LEFT_PRESSED: AtomicBool = AtomicBool::new(false);
static LEFT_DOWN: AtomicBool = AtomicBool::new(false);
const OWN_INPUT: usize = 0x57464D;
// A consumed hotkey may not appear in the asynchronous key state. These
// hold actions use the hook's down and up events to decide when to run.
const HOOK_TRACKED_HOLDS: usize =
    crate::macros::WHIP_ENABLED | crate::macros::WHIP_SWITCH_ENABLED | crate::macros::SPIN_ENABLED;
static ENABLED: AtomicUsize = AtomicUsize::new(0);
static TARGET_PID: AtomicUsize = AtomicUsize::new(0);
static HELD: AtomicUsize = AtomicUsize::new(0);
static CAPTURED: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static PRESSED: AtomicUsize = AtomicUsize::new(0);

fn matches_foreground() -> bool {
    let pid = TARGET_PID.load(Relaxed);
    pid != 0 && crate::processes::foreground_pid() as usize == pid
}

fn handle_button(vk: u32, up: bool, foreground: bool) -> bool {
    let Some(index) = BINDINGS
        .iter()
        .position(|binding| binding.load(Relaxed) == vk as usize)
    else {
        return false;
    };
    let bit = 1 << index;
    if up {
        HELD.fetch_and(!bit, Relaxed);
        ACTIVE.fetch_and(!bit, Relaxed);
        return CAPTURED.fetch_and(!bit, Relaxed) & bit != 0;
    }
    // A held key keeps its original routing through auto-repeat and key-up.
    if HELD.fetch_or(bit, Relaxed) & bit != 0 {
        return CAPTURED.load(Relaxed) & bit != 0;
    }
    if !foreground || ENABLED.load(Relaxed) & bit == 0 {
        return false;
    }
    CAPTURED.fetch_or(bit, Relaxed);
    ACTIVE.fetch_or(bit, Relaxed);
    PRESSED.fetch_or(bit, Relaxed);
    true
}

extern "system" fn keyboard_hook(code: i32, message: usize, data: isize) -> isize {
    if code == 0 && matches!(message, 0x0100 | 0x0101 | 0x0104 | 0x0105) {
        let event = unsafe { &*(data as *const HookKey) };
        if event.flags & 0x10 == 0 || event.extra != OWN_INPUT {
            let up = matches!(message, 0x0101 | 0x0105);
            if event.vk == 0x20 {
                SPACE_DOWN.store(usize::from(!up), Relaxed);
            }
            if handle_button(event.vk, up, matches_foreground()) {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, message, data) }
}

fn mouse_hook_button(message: usize, data: u32) -> Option<(u32, bool)> {
    match message {
        0x0204 | 0x0205 => Some((2, message == 0x0205)),
        0x0207 | 0x0208 => Some((4, message == 0x0208)),
        0x020b | 0x020c => match data >> 16 {
            1 => Some((5, message == 0x020c)),
            2 => Some((6, message == 0x020c)),
            _ => None,
        },
        _ => None,
    }
}

extern "system" fn mouse_hook(code: i32, message: usize, data: isize) -> isize {
    if code == 0 {
        let event = unsafe { &*(data as *const HookMouse) };
        if event.flags & 1 == 0 || event.extra != OWN_INPUT {
            if message == 0x0201 {
                LEFT_PRESSED.store(true, Relaxed);
                LEFT_DOWN.store(true, Relaxed);
            } else if message == 0x0202 {
                LEFT_DOWN.store(false, Relaxed);
            }
            if let Some((vk, up)) = mouse_hook_button(message, event.data) {
                if handle_button(vk, up, matches_foreground()) {
                    return 1;
                }
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, message, data) }
}

pub struct Hotkeys {
    keyboard: *mut c_void,
    mouse: *mut c_void,
}

impl Hotkeys {
    pub fn install(bindings: [u32; 7]) -> io::Result<Self> {
        let held = |vk| usize::from(unsafe { GetAsyncKeyState(vk as i32) } < 0);
        let mut held_mask = 0;
        for (index, (binding, code)) in BINDINGS.iter().zip(bindings).enumerate() {
            binding.store(code as usize, Relaxed);
            held_mask |= held(code) << index;
        }
        HELD.store(held_mask, Relaxed);
        SPACE_DOWN.store(held(0x20), Relaxed);
        LEFT_PRESSED.store(false, Relaxed);
        LEFT_DOWN.store(held(1) != 0, Relaxed);
        ENABLED.store(0, Relaxed);
        TARGET_PID.store(0, Relaxed);
        CAPTURED.store(0, Relaxed);
        ACTIVE.store(0, Relaxed);
        PRESSED.store(0, Relaxed);
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        let keyboard = unsafe { SetWindowsHookExW(13, keyboard_hook, module, 0) };
        if keyboard.is_null() {
            let error = io::Error::last_os_error();
            clear_bindings();
            return Err(error);
        }
        let mut hooks = Self {
            keyboard,
            mouse: null_mut(),
        };
        hooks.mouse = unsafe { SetWindowsHookExW(14, mouse_hook, module, 0) };
        if hooks.mouse.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(hooks)
    }

    pub fn read(&self, enabled: usize, target_pid: u32) -> usize {
        TARGET_PID.store(target_pid as usize, Relaxed);
        ENABLED.store(enabled, Relaxed);
        let mut message: Message = unsafe { zeroed() };
        while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, 1) } != 0 {
            unsafe { DispatchMessageW(&message) };
        }
        let mut physically_down = 0;
        for (index, binding) in BINDINGS.iter().enumerate() {
            let vk = binding.load(Relaxed) as i32;
            if vk != 0 && unsafe { GetAsyncKeyState(vk) } < 0 {
                physically_down |= 1 << index;
            }
        }
        // Reconcile missed key-up messages, common when focus changes or input
        // arrives through a remote-control layer.
        HELD.store(physically_down, Relaxed);
        CAPTURED.fetch_and(physically_down, Relaxed);
        ACTIVE.fetch_and(enabled & (physically_down | HOOK_TRACKED_HOLDS), Relaxed);
        PRESSED.fetch_and(enabled, Relaxed);
        SPACE_DOWN.store(usize::from(unsafe { GetAsyncKeyState(0x20) } < 0), Relaxed);
        LEFT_DOWN.store(unsafe { GetAsyncKeyState(1) } < 0, Relaxed);
        ACTIVE.load(Relaxed)
    }

    pub fn take_pressed(&self) -> usize {
        PRESSED.swap(0, Relaxed)
    }

    pub fn suspend(&self) {
        TARGET_PID.store(0, Relaxed);
        ENABLED.store(0, Relaxed);
        ACTIVE.store(0, Relaxed);
        PRESSED.store(0, Relaxed);
        LEFT_PRESSED.store(false, Relaxed);
    }

    pub fn space_down(&self) -> bool {
        SPACE_DOWN.load(Relaxed) != 0
    }

    pub fn take_left_press(&self) -> bool {
        LEFT_PRESSED.swap(false, Relaxed)
    }

    pub fn left_down(&self) -> bool {
        LEFT_DOWN.load(Relaxed)
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        unsafe {
            if !self.mouse.is_null() {
                UnhookWindowsHookEx(self.mouse);
            }
            UnhookWindowsHookEx(self.keyboard);
        }
        clear_bindings();
    }
}

fn clear_bindings() {
    ENABLED.store(0, Relaxed);
    TARGET_PID.store(0, Relaxed);
    HELD.store(0, Relaxed);
    CAPTURED.store(0, Relaxed);
    ACTIVE.store(0, Relaxed);
    PRESSED.store(0, Relaxed);
    LEFT_PRESSED.store(false, Relaxed);
    for binding in &BINDINGS {
        binding.store(0, Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_features_pass_through_and_existing_presses_keep_their_routing() {
        BINDINGS[0].store(71, Relaxed);
        BINDINGS[1].store(72, Relaxed);
        ENABLED.store(2, Relaxed);
        HELD.store(0, Relaxed);
        CAPTURED.store(0, Relaxed);
        ACTIVE.store(0, Relaxed);

        assert!(!handle_button(71, false, true));
        assert!(handle_button(72, false, true));
        assert_eq!(ACTIVE.load(Relaxed), 2);
        ENABLED.store(1, Relaxed);
        ACTIVE.fetch_and(1, Relaxed);
        assert!(!handle_button(71, false, true));
        assert!(handle_button(72, false, false));
        assert_eq!(ACTIVE.load(Relaxed), 0);
        assert!(!handle_button(71, true, true));
        assert!(handle_button(72, true, false));
        assert!(!handle_button(71, false, false));
        assert!(!handle_button(71, false, true));
        assert!(!handle_button(71, true, true));
        assert!(handle_button(71, false, true));
        assert_eq!(ACTIVE.load(Relaxed), 1);
        assert!(!handle_button(72, false, true));
        clear_bindings();
        BINDINGS[2].store(0x74, Relaxed);
        ENABLED.store(4, Relaxed);
        assert!(!handle_button(0x75, false, true));
        assert!(!handle_button(71, false, true));
        assert!(handle_button(0x74, false, true));
        clear_bindings();
    }

    #[test]
    fn mouse_buttons_match_windows_messages_and_attack_events() {
        for (vk, down, up, data) in [
            (2, 0x0204, 0x0205, 0),
            (4, 0x0207, 0x0208, 0),
            (5, 0x020b, 0x020c, 1),
            (6, 0x020b, 0x020c, 2),
        ] {
            assert_eq!(mouse_hook_button(down, data << 16), Some((vk, false)));
            assert_eq!(mouse_hook_button(up, data << 16), Some((vk, true)));
            let press = unsafe { mouse_button(vk, false).unwrap().data.mouse };
            let release = unsafe { mouse_button(vk, true).unwrap().data.mouse };
            assert_eq!(press.data, data);
            assert_eq!(release.flags, press.flags << 1);
            assert_eq!(press.extra, OWN_INPUT);
        }
        assert_eq!(mouse_hook_button(0x0201, 0), None);
        assert_eq!(mouse_hook_button(0x0202, 0), None);
    }
}
