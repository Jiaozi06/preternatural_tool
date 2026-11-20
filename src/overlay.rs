use crate::macros::{
    PICKUP_ENABLED, QUICK_ENABLED, SIDE_ENABLED, SPEAR_ENABLED, SPIN_ENABLED, WHIP_ENABLED,
    WHIP_SWITCH_ENABLED,
};
use crate::processes::ForegroundTarget;
use std::cell::{Cell, RefCell};
use std::io;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WIDTH: i32 = 154;
const ROW: i32 = 23;
const HEADING: i32 = 32;
const FEATURES: [(&str, usize); 7] = [
    ("快捷拾取", PICKUP_ENABLED),
    ("长枪瞬切", SPEAR_ENABLED),
    ("双鞭", WHIP_ENABLED),
    ("鞭子瞬切", WHIP_SWITCH_ENABLED),
    ("转圈圈", SPIN_ENABLED),
    ("快速飞", QUICK_ENABLED),
    ("左右飞", SIDE_ENABLED),
];
const HEIGHT: i32 = HEADING + ROW * FEATURES.len() as i32 + 8;
const RED: COLORREF = rgb(246, 100, 102);
const GREEN: COLORREF = rgb(56, 214, 142);
const ORANGE: COLORREF = rgb(255, 174, 66);

pub struct Overlay {
    window: HWND,
    state: Box<OverlayState>,
}

struct OverlayState {
    target: RefCell<Option<(String, ForegroundTarget)>>,
    mask: Cell<usize>,
    cooldown: Cell<usize>,
    visible: Cell<bool>,
    position: Cell<(i32, i32)>,
    opacity: Cell<u8>,
}

impl Overlay {
    pub fn create(opacity: u8) -> io::Result<Self> {
        let class = wide("LvwanMutationToolbox.Overlay");
        let instance = unsafe { GetModuleHandleW(null()) };
        let descriptor = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&descriptor) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let state = Box::new(OverlayState {
            target: RefCell::new(None),
            mask: Cell::new(0),
            cooldown: Cell::new(0),
            visible: Cell::new(false),
            position: Cell::new((0, 0)),
            opacity: Cell::new(opacity_to_alpha(opacity)),
        });
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW,
                class.as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                WIDTH,
                HEIGHT,
                null_mut(),
                null_mut(),
                instance,
                (&*state as *const OverlayState).cast(),
            )
        };
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }
        let overlay = Self { window, state };
        unsafe {
            if SetLayeredWindowAttributes(window, 0, opacity_to_alpha(opacity), LWA_ALPHA) == 0 {
                return Err(io::Error::last_os_error());
            }
            let region = CreateRoundRectRgn(0, 0, WIDTH + 1, HEIGHT + 1, 20, 20);
            if !region.is_null() && SetWindowRgn(window, region, 0) == 0 {
                DeleteObject(region);
            }
            if SetTimer(window, 1, 120, None) == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(overlay)
    }

    pub fn update(&self, target: Option<&str>, mask: usize, cooldown: usize, opacity: u8) {
        let mut previous = self.state.target.borrow_mut();
        let path = previous.as_ref().map(|(path, _)| path.as_str());
        let target_changed = path != target;
        if target_changed {
            *previous =
                target.map(|path| (path.to_owned(), ForegroundTarget::new(path.to_owned())));
        }
        drop(previous);
        let mask_changed = self.state.mask.replace(mask) != mask;
        let cooldown_changed = self.state.cooldown.replace(cooldown) != cooldown;
        let opacity = opacity_to_alpha(opacity);
        if self.state.opacity.replace(opacity) != opacity {
            unsafe { SetLayeredWindowAttributes(self.window, 0, opacity, LWA_ALPHA) };
        }
        if mask_changed || cooldown_changed {
            unsafe { InvalidateRect(self.window, null(), 0) };
        }
        if target_changed {
            self.state.refresh(self.window);
        }
    }
}

fn opacity_to_alpha(opacity: u8) -> u8 {
    ((u16::from(opacity.clamp(20, 100)) * 255) / 100) as u8
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            KillTimer(self.window, 1);
            DestroyWindow(self.window);
        }
    }
}

impl OverlayState {
    fn refresh(&self, window: HWND) {
        let matched = self
            .target
            .borrow_mut()
            .as_mut()
            .is_some_and(|(_, target)| target.matching_pid() != 0);
        if !matched {
            if self.visible.replace(false) {
                unsafe { ShowWindow(window, SW_HIDE) };
            }
            return;
        }
        let monitor = unsafe { MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return;
        }
        let work = info.rcWork;
        let x = work.left + 18;
        let y = (work.top + (work.bottom - work.top) * 2 / 3 - HEIGHT / 2)
            .min((work.bottom - HEIGHT).max(work.top))
            .max(work.top);
        if !self.visible.get() || self.position.get() != (x, y) {
            if unsafe {
                SetWindowPos(
                    window,
                    HWND_TOPMOST,
                    x,
                    y,
                    WIDTH,
                    HEIGHT,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            } != 0
            {
                self.position.set((x, y));
                self.visible.set(true);
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let state = (GetWindowLongPtrW(window, GWLP_USERDATA) as *const OverlayState).as_ref();
    match (message, state) {
        (WM_TIMER, Some(state)) => {
            state.refresh(window);
            0
        }
        (WM_PAINT, Some(state)) => {
            paint(window, state.mask.get(), state.cooldown.get());
            0
        }
        (WM_ERASEBKGND, _) => 1,
        (WM_NCHITTEST, _) => HTTRANSPARENT as LRESULT,
        (WM_MOUSEACTIVATE, _) => MA_NOACTIVATE as LRESULT,
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

unsafe fn paint(window: HWND, mask: usize, cooldown: usize) {
    let mut paint = PAINTSTRUCT::default();
    let dc = BeginPaint(window, &mut paint);
    if dc.is_null() {
        return;
    }
    let background = CreateSolidBrush(rgb(23, 30, 42));
    FillRect(dc, &paint.rcPaint, background);
    DeleteObject(background);
    let font_name = wide("Microsoft YaHei UI");
    let font = CreateFontW(
        -15,
        0,
        0,
        0,
        FW_NORMAL as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        font_name.as_ptr(),
    );
    let previous_font = SelectObject(dc, font);
    let previous_pen = SelectObject(dc, GetStockObject(NULL_PEN));
    let on = CreateSolidBrush(GREEN);
    let off = CreateSolidBrush(RED);
    let cooling = CreateSolidBrush(ORANGE);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, rgb(162, 179, 199));
    draw_text(dc, 13, 8, "功能状态");
    SetTextColor(dc, rgb(239, 244, 250));
    for (index, (label, bit)) in FEATURES.into_iter().enumerate() {
        let y = HEADING + ROW * index as i32;
        let brush = if cooldown & bit != 0 {
            cooling
        } else if mask & bit != 0 {
            on
        } else {
            off
        };
        let previous_brush = SelectObject(dc, brush);
        Ellipse(dc, 14, y + 6, 23, y + 15);
        SelectObject(dc, previous_brush);
        draw_text(dc, 33, y + 1, label);
    }
    SelectObject(dc, previous_pen);
    SelectObject(dc, previous_font);
    DeleteObject(on);
    DeleteObject(off);
    DeleteObject(cooling);
    DeleteObject(font);
    EndPaint(window, &paint);
}

unsafe fn draw_text(dc: HDC, x: i32, y: i32, text: &str) {
    let text: Vec<u16> = text.encode_utf16().collect();
    TextOutW(dc, x, y, text.as_ptr(), text.len() as i32);
}

const fn rgb(red: u32, green: u32, blue: u32) -> COLORREF {
    red | (green << 8) | (blue << 16)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_status_window_does_not_activate_or_capture_input() {
        let overlay = Overlay::create(90).unwrap();
        let attributes = unsafe { GetWindowLongW(overlay.window, GWL_EXSTYLE) } as u32;
        let required =
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
        assert_eq!(attributes & required, required);
        assert_eq!(unsafe { IsWindowVisible(overlay.window) }, 0);
        assert_eq!(
            unsafe { SendMessageW(overlay.window, WM_NCHITTEST, 0, 0) },
            HTTRANSPARENT as LRESULT
        );
        overlay.update(None, PICKUP_ENABLED | QUICK_ENABLED, 0, 90);
        assert_eq!(overlay.state.mask.get(), PICKUP_ENABLED | QUICK_ENABLED);
    }
}
