#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod flight;
mod instance;
mod macros;
mod overlay;
mod processes;
mod shortcuts;
mod tray;
mod updates;
mod whip;
mod windows;

use eframe::egui::{self, Color32, RichText, Stroke};
use flight::{DEFAULT_JET_DELAY_MS, DEFAULT_SIDE_MS};
use macros::{
    keyboard_key, Settings, DEFAULT_DELAYS, DEFAULT_PICKUP_INTERVAL_MS, DEFAULT_SWITCH_COOLDOWN_MS,
    F_REPEAT_ENABLED, PICKUP_ENABLED, QUICK_ENABLED, SIDE_ENABLED, SPEAR_ENABLED, SPIN_ENABLED,
    WHIP_ENABLED, WHIP_NO_SLOW_ENABLED, WHIP_SWITCH_ENABLED,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use whip::{DEFAULT_CHARGE_MS, DEFAULT_DUAL_EQUIP_MS, DEFAULT_RECOVERY_MS, DEFAULT_WAIT_MS};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND,
};

const ACCENT: Color32 = Color32::from_rgb(0, 95, 184);
const WINDOW_TITLE: &str = "绿玩变异工具箱 v1.3 ©️Jiaozi";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const MUTED: Color32 = Color32::from_rgb(82, 86, 94);
const FIELD_WIDTH: f32 = 136.0;
const FIELD_HEIGHT: f32 = 28.0;
const WINDOW_FIELD_WIDTH: f32 = 246.0;
const SLOTS: [u8; 7] = [7, 1, 2, 3, 4, 5, 6];
const SETTINGS_KEY: &str = "macro-settings";
const DEFAULT_QUICK: Binding = Binding::new(0x75, "F6");
const DEFAULT_SIDE: Binding = Binding::new(0x76, "F7");
const DEFAULT_WHIP: Binding = Binding::new(0x78, "F9");
const DEFAULT_WHIP_SWITCH: Binding = Binding::new(0x79, "F10");
const DEFAULT_SPIN: Binding = Binding::new(0x7a, "F11");
const DEFAULT_WHIP_NO_SLOW: Binding = Binding::new(0x7b, "F12");
const DELAY_FIELDS: [&str; 5] = [
    "攻击键按住",
    "松开攻击后，等待切换",
    "切换位按住",
    "松开切换位后，等待切回",
    "武器位按住",
];

#[link(name = "shell32")]
extern "system" {
    fn IsUserAnAdmin() -> i32;
}

enum WorkerEvent {
    Ready,
    Failed(String),
}

struct Worker {
    stop: Arc<AtomicBool>,
    enabled: Arc<AtomicUsize>,
    cooldown: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
    events: Receiver<WorkerEvent>,
}

impl Worker {
    fn start(settings: Settings, mask: usize) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let enabled = Arc::new(AtomicUsize::new(mask));
        let worker_enabled = Arc::clone(&enabled);
        let cooldown = Arc::new(AtomicUsize::new(0));
        let worker_cooldown = Arc::clone(&cooldown);
        let (sender, events) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("hotkeys".into())
            .spawn(move || {
                let result = macros::run(
                    settings,
                    &worker_stop,
                    &worker_enabled,
                    &worker_cooldown,
                    || {
                        let _ = sender.send(WorkerEvent::Ready);
                    },
                );
                if let Err(error) = result {
                    let _ = sender.send(WorkerEvent::Failed(error.to_string()));
                }
            })?;
        Ok(Self {
            stop,
            enabled,
            cooldown,
            thread: Some(thread),
            events,
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct App {
    pickup: Binding,
    pickup_enabled: bool,
    f_repeat_enabled: bool,
    pickup_interval_ms: u32,
    spear: Binding,
    spear_enabled: bool,
    weapon: u8,
    swap: u8,
    attack: Binding,
    listening: Option<HotkeyField>,
    delays: [u32; 5],
    spear_cooldown_ms: u32,
    whip: Binding,
    whip_enabled: bool,
    whip_slots: [u8; 2],
    whip_equip_ms: u32,
    whip_charge_ms: u32,
    whip_wait_ms: u32,
    whip_switch: Binding,
    whip_switch_enabled: bool,
    whip_switch_slots: [u8; 2],
    whip_switch_charge_ms: u32,
    whip_switch_recovery_ms: u32,
    whip_switch_return_ms: u32,
    whip_switch_cooldown_ms: u32,
    quick: Binding,
    quick_enabled: bool,
    jet_slot: u8,
    jet_equip_delay_ms: u32,
    jet_delay_ms: u32,
    side: Binding,
    side_enabled: bool,
    side_ms: u32,
    side_segments: Vec<flight::SideSegment>,
    side_random_max_ms: u32,
    side_toggle_mode: bool,
    quick_auto_side: bool,
    spin: Binding,
    spin_enabled: bool,
    spin_pixels: u32,
    whip_no_slow: Binding,
    whip_no_slow_enabled: bool,
    revolver_slot: u8,
    whip_no_slow_charge_ms: u32,
    revolver_wait_ms: u32,
    page: Page,
    target_path: String,
    target_title: String,
    running_apps: Vec<processes::RunningApp>,
    apps_refreshed: Option<Instant>,
    worker: Option<Worker>,
    tray: Option<tray::Tray>,
    overlay: Option<overlay::Overlay>,
    overlay_enabled: bool,
    overlay_opacity: u8,
    update_mirror: bool,
    update_receiver: Option<Receiver<Result<updates::OnlineManifest, String>>>,
    download_receiver: Option<Receiver<Result<std::path::PathBuf, String>>>,
    download_status: Option<String>,
    online_manifest: Option<updates::OnlineManifest>,
    update_status: Option<String>,
    running: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Pickup,
    Battle,
    Flight,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HotkeyField {
    Pickup,
    Spear,
    Quick,
    Side,
    Whip,
    WhipSwitch,
    Spin,
    WhipNoSlow,
    Attack,
}

#[derive(Clone, Copy)]
struct Binding {
    code: u32,
    label: &'static str,
}

impl Binding {
    const fn new(code: u32, label: &'static str) -> Self {
        Self { code, label }
    }

    fn mouse(button: egui::PointerButton) -> Self {
        match button {
            egui::PointerButton::Primary => Self::new(1, "鼠标左键"),
            egui::PointerButton::Secondary => Self::new(2, "鼠标右键"),
            egui::PointerButton::Middle => Self::new(4, "鼠标中键"),
            egui::PointerButton::Extra1 => Self::new(5, "鼠标侧键 1"),
            egui::PointerButton::Extra2 => Self::new(6, "鼠标侧键 2"),
        }
    }

    fn from_code(code: u32) -> Option<Self> {
        use egui::PointerButton::{Extra1, Extra2, Middle, Primary, Secondary};
        let button = match code {
            1 => Primary,
            2 => Secondary,
            4 => Middle,
            5 => Extra1,
            6 => Extra2,
            _ => {
                let label = egui::Key::ALL
                    .iter()
                    .map(|key| key.name())
                    .find(|label| keyboard_key(label) == Some(code))?;
                return Some(Self::new(code, label));
            }
        };
        Some(Self::mouse(button))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedSettings {
    pickup: u32,
    #[serde(default = "enabled_by_default")]
    pickup_enabled: bool,
    #[serde(default = "enabled_by_default")]
    f_repeat_enabled: bool,
    pickup_interval_ms: u32,
    spear: u32,
    #[serde(default = "enabled_by_default")]
    spear_enabled: bool,
    weapon: u8,
    swap: u8,
    attack: u32,
    delays: [u32; 5],
    #[serde(default = "default_switch_cooldown_ms")]
    spear_cooldown_ms: u32,
    #[serde(default)]
    flight: SavedFlight,
    #[serde(default)]
    whip: SavedWhip,
    #[serde(default = "default_whip_equip_ms")]
    whip_equip_ms: u32,
    #[serde(default = "default_whip_switch")]
    whip_switch: SavedWhip,
    #[serde(default = "default_whip_switch_return_ms")]
    whip_switch_return_ms: u32,
    #[serde(default = "default_switch_cooldown_ms")]
    whip_switch_cooldown_ms: u32,
    #[serde(default)]
    target_path: String,
    #[serde(default)]
    target_title: String,
    #[serde(default = "enabled_by_default")]
    overlay_enabled: bool,
    #[serde(default = "default_overlay_opacity")]
    overlay_opacity: u8,
    #[serde(default)]
    spin: SavedSpin,
    #[serde(default)]
    whip_no_slow: SavedWhipNoSlow,
    #[serde(default)]
    update_mirror: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedWhipNoSlow {
    key: u32,
    enabled: bool,
    slot: u8,
    charge_ms: u32,
    #[serde(default = "default_revolver_wait_ms")]
    revolver_wait_ms: u32,
}

fn default_revolver_wait_ms() -> u32 {
    1000
}

impl Default for SavedWhipNoSlow {
    fn default() -> Self {
        Self {
            key: DEFAULT_WHIP_NO_SLOW.code,
            enabled: false,
            slot: 2,
            charge_ms: 1000,
            revolver_wait_ms: default_revolver_wait_ms(),
        }
    }
}

fn default_switch_cooldown_ms() -> u32 {
    DEFAULT_SWITCH_COOLDOWN_MS
}

fn default_whip_equip_ms() -> u32 {
    DEFAULT_DUAL_EQUIP_MS
}

fn default_overlay_opacity() -> u8 {
    90
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedSpin {
    key: u32,
    enabled: bool,
    pixels: u32,
}

impl Default for SavedSpin {
    fn default() -> Self {
        Self {
            key: DEFAULT_SPIN.code,
            enabled: true,
            pixels: 8,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedFlight {
    quick: u32,
    quick_enabled: bool,
    jet_slot: u8,
    #[serde(default = "flight::default_equip_delay_ms")]
    jet_equip_delay_ms: u32,
    jet_delay_ms: u32,
    side: u32,
    side_enabled: bool,
    #[serde(deserialize_with = "deserialize_side_interval")]
    side_ms: u32,
    #[serde(default = "flight::default_side_segments")]
    side_segments: Vec<flight::SideSegment>,
    #[serde(default)]
    side_random_max_ms: u32,
    #[serde(default)]
    side_toggle_mode: bool,
    #[serde(default)]
    quick_auto_side: bool,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum SavedSideInterval {
    Single(u32),
    Legacy([u32; 2]),
}

fn deserialize_side_interval<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(
        match <SavedSideInterval as serde::Deserialize>::deserialize(deserializer)? {
            SavedSideInterval::Single(value) => value,
            SavedSideInterval::Legacy([a, b]) => ((u64::from(a) + u64::from(b)) / 2) as u32,
        },
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedWhip {
    key: u32,
    enabled: bool,
    slots: [u8; 2],
    charge_ms: u32,
    wait_ms: u32,
}

impl Default for SavedWhip {
    fn default() -> Self {
        Self {
            key: DEFAULT_WHIP.code,
            enabled: true,
            slots: [1, 2],
            charge_ms: DEFAULT_CHARGE_MS,
            wait_ms: DEFAULT_WAIT_MS,
        }
    }
}

fn default_whip_switch() -> SavedWhip {
    SavedWhip {
        key: DEFAULT_WHIP_SWITCH.code,
        wait_ms: DEFAULT_RECOVERY_MS,
        ..SavedWhip::default()
    }
}

impl Default for SavedFlight {
    fn default() -> Self {
        Self {
            quick: DEFAULT_QUICK.code,
            quick_enabled: true,
            jet_slot: 7,
            jet_equip_delay_ms: flight::default_equip_delay_ms(),
            jet_delay_ms: DEFAULT_JET_DELAY_MS,
            side: DEFAULT_SIDE.code,
            side_enabled: true,
            side_ms: DEFAULT_SIDE_MS,
            side_segments: flight::default_side_segments(),
            side_random_max_ms: 0,
            side_toggle_mode: false,
            quick_auto_side: false,
        }
    }
}

fn enabled_by_default() -> bool {
    true
}

fn default_whip_switch_return_ms() -> u32 {
    20
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        setup_style(ctx);
        let no_slow = SavedWhipNoSlow::default();
        Self {
            pickup: Binding::new(u32::from(b'G'), "G"),
            pickup_enabled: true,
            f_repeat_enabled: true,
            pickup_interval_ms: DEFAULT_PICKUP_INTERVAL_MS,
            spear: Binding::new(u32::from(b'H'), "H"),
            spear_enabled: true,
            weapon: 1,
            swap: 2,
            attack: Binding::mouse(egui::PointerButton::Primary),
            listening: None,
            delays: DEFAULT_DELAYS,
            spear_cooldown_ms: DEFAULT_SWITCH_COOLDOWN_MS,
            whip: DEFAULT_WHIP,
            whip_enabled: true,
            whip_slots: [1, 2],
            whip_equip_ms: DEFAULT_DUAL_EQUIP_MS,
            whip_charge_ms: DEFAULT_CHARGE_MS,
            whip_wait_ms: DEFAULT_WAIT_MS,
            whip_switch: DEFAULT_WHIP_SWITCH,
            whip_switch_enabled: true,
            whip_switch_slots: [1, 2],
            whip_switch_charge_ms: DEFAULT_CHARGE_MS,
            whip_switch_recovery_ms: DEFAULT_RECOVERY_MS,
            whip_switch_return_ms: default_whip_switch_return_ms(),
            whip_switch_cooldown_ms: DEFAULT_SWITCH_COOLDOWN_MS,
            quick: DEFAULT_QUICK,
            quick_enabled: true,
            jet_slot: 7,
            jet_equip_delay_ms: flight::default_equip_delay_ms(),
            jet_delay_ms: DEFAULT_JET_DELAY_MS,
            side: DEFAULT_SIDE,
            side_enabled: true,
            side_ms: DEFAULT_SIDE_MS,
            side_segments: flight::default_side_segments(),
            side_random_max_ms: 0,
            side_toggle_mode: false,
            quick_auto_side: false,
            spin: DEFAULT_SPIN,
            spin_enabled: true,
            spin_pixels: 8,
            whip_no_slow: DEFAULT_WHIP_NO_SLOW,
            whip_no_slow_enabled: no_slow.enabled,
            revolver_slot: no_slow.slot,
            whip_no_slow_charge_ms: no_slow.charge_ms,
            revolver_wait_ms: no_slow.revolver_wait_ms,
            page: Page::Pickup,
            target_path: String::new(),
            target_title: String::new(),
            running_apps: Vec::new(),
            apps_refreshed: None,
            worker: None,
            tray: None,
            overlay: None,
            overlay_enabled: true,
            overlay_opacity: default_overlay_opacity(),
            update_mirror: false,
            update_receiver: None,
            download_receiver: None,
            download_status: None,
            online_manifest: None,
            update_status: None,
            running: false,
            error: None,
        }
    }

    fn settings(&self) -> Result<Settings, &str> {
        let pickup = self.pickup.code;
        let spear = self.spear.code;
        let bindings = [
            pickup,
            spear,
            self.quick.code,
            self.side.code,
            self.whip.code,
            self.whip_switch.code,
            self.spin.code,
            self.whip_no_slow.code,
        ];
        if bindings
            .iter()
            .enumerate()
            .any(|(index, code)| bindings[..index].contains(code))
        {
            return Err("请为各功能设置不同的热键。");
        }
        if self.weapon == self.swap {
            return Err("武器位和切换位需要使用不同的数字。");
        }
        if self.whip_slots[0] == self.whip_slots[1] {
            return Err("一鞭位和二鞭位需要使用不同的数字。");
        }
        if self.whip_switch_slots[0] == self.whip_switch_slots[1] {
            return Err("鞭子瞬切的武器位和物品位需要使用不同的数字。");
        }
        Ok(Settings {
            pickup,
            pickup_interval_ms: self.pickup_interval_ms,
            spear,
            weapon: u32::from(b'0' + self.weapon),
            swap: u32::from(b'0' + self.swap),
            attack: self.attack.code,
            delays: self.delays,
            spear_cooldown_ms: self.spear_cooldown_ms,
            quick: self.quick.code,
            jet_slot: u32::from(b'0' + self.jet_slot),
            jet_equip_delay_ms: self.jet_equip_delay_ms,
            jet_delay_ms: self.jet_delay_ms,
            side: self.side.code,
            side_ms: self.side_ms.max(1),
            side_segments: {
                let mut segments = self.side_segments.clone();
                segments.sort_by_key(|segment| segment.after_ms);
                for segment in &mut segments {
                    segment.duration_ms = segment.duration_ms.max(1);
                }
                segments
            },
            side_random_max_ms: self.side_random_max_ms,
            side_toggle_mode: self.side_toggle_mode,
            quick_auto_side: self.quick_auto_side,
            target_path: self.target_path.clone(),
            whip: self.whip.code,
            whip_slots: self.whip_slots.map(|slot| u32::from(b'0' + slot)),
            whip_equip_ms: self.whip_equip_ms,
            whip_charge_ms: self.whip_charge_ms,
            whip_wait_ms: self.whip_wait_ms,
            whip_switch: self.whip_switch.code,
            whip_switch_slots: self.whip_switch_slots.map(|slot| u32::from(b'0' + slot)),
            whip_switch_charge_ms: self.whip_switch_charge_ms,
            whip_switch_recovery_ms: self.whip_switch_recovery_ms,
            whip_switch_return_ms: self.whip_switch_return_ms,
            whip_switch_cooldown_ms: self.whip_switch_cooldown_ms,
            spin: self.spin.code,
            spin_pixels: self.spin_pixels,
            whip_no_slow: self.whip_no_slow.code,
            revolver_slot: u32::from(b'0' + self.revolver_slot),
            whip_no_slow_charge_ms: self.whip_no_slow_charge_ms,
            revolver_wait_ms: self.revolver_wait_ms,
        })
    }

    fn restore(&mut self, storage: &dyn eframe::Storage) {
        let Some(saved) = eframe::get_value::<SavedSettings>(storage, SETTINGS_KEY) else {
            return;
        };
        let pickup = Binding::from_code(saved.pickup).filter(|binding| binding.code != 1);
        let spear = Binding::from_code(saved.spear).filter(|binding| binding.code != 1);
        if let (Some(pickup), Some(spear)) = (pickup, spear) {
            if pickup.code != spear.code {
                self.pickup = pickup;
                self.spear = spear;
            }
        }
        self.attack = Binding::from_code(saved.attack).unwrap_or(self.attack);
        if SLOTS.contains(&saved.weapon) {
            self.weapon = saved.weapon;
        }
        if SLOTS.contains(&saved.swap) {
            self.swap = saved.swap;
        }
        self.pickup_interval_ms = saved.pickup_interval_ms.max(1);
        self.delays = saved.delays;
        self.delays[2] = self.delays[2].max(20);
        self.delays[4] = self.delays[4].max(20);
        self.spear_cooldown_ms = saved.spear_cooldown_ms;
        self.pickup_enabled = saved.pickup_enabled;
        self.f_repeat_enabled = saved.f_repeat_enabled;
        self.spear_enabled = saved.spear_enabled;
        let flight = saved.flight;
        let existing = [self.pickup.code, self.spear.code];
        self.quick = restore_binding(flight.quick, DEFAULT_QUICK, &existing, true);
        self.side = restore_binding(
            flight.side,
            DEFAULT_SIDE,
            &[existing[0], existing[1], self.quick.code],
            true,
        );
        let whip = saved.whip;
        self.whip = restore_binding(
            whip.key,
            DEFAULT_WHIP,
            &[existing[0], existing[1], self.quick.code, self.side.code],
            false,
        );
        self.whip_enabled = whip.enabled;
        if whip.slots.iter().all(|slot| SLOTS.contains(slot)) {
            self.whip_slots = whip.slots;
        }
        self.whip_charge_ms = whip.charge_ms.max(1);
        self.whip_equip_ms = saved.whip_equip_ms;
        self.whip_wait_ms = whip.wait_ms;
        let whip_switch = saved.whip_switch;
        self.whip_switch = restore_binding(
            whip_switch.key,
            DEFAULT_WHIP_SWITCH,
            &[
                existing[0],
                existing[1],
                self.quick.code,
                self.side.code,
                self.whip.code,
            ],
            false,
        );
        self.whip_switch_enabled = whip_switch.enabled;
        if whip_switch.slots.iter().all(|slot| SLOTS.contains(slot)) {
            self.whip_switch_slots = whip_switch.slots;
        }
        self.whip_switch_charge_ms = whip_switch.charge_ms.max(1);
        self.whip_switch_recovery_ms = whip_switch.wait_ms;
        self.whip_switch_return_ms = saved.whip_switch_return_ms;
        self.whip_switch_cooldown_ms = saved.whip_switch_cooldown_ms;
        let spin = saved.spin;
        self.spin = restore_binding(
            spin.key,
            DEFAULT_SPIN,
            &[
                self.pickup.code,
                self.spear.code,
                self.quick.code,
                self.side.code,
                self.whip.code,
                self.whip_switch.code,
            ],
            false,
        );
        self.spin_enabled = spin.enabled;
        self.spin_pixels = spin.pixels.max(1);
        let no_slow = saved.whip_no_slow;
        self.whip_no_slow = restore_binding(
            no_slow.key,
            DEFAULT_WHIP_NO_SLOW,
            &[
                self.pickup.code,
                self.spear.code,
                self.quick.code,
                self.side.code,
                self.whip.code,
                self.whip_switch.code,
                self.spin.code,
            ],
            false,
        );
        self.whip_no_slow_enabled = no_slow.enabled;
        if SLOTS.contains(&no_slow.slot) {
            self.revolver_slot = no_slow.slot;
        }
        self.whip_no_slow_charge_ms = no_slow.charge_ms.clamp(10, 60_000);
        self.revolver_wait_ms = no_slow.revolver_wait_ms.clamp(10, 60_000);
        self.quick_enabled = flight.quick_enabled;
        self.side_enabled = flight.side_enabled;
        if SLOTS.contains(&flight.jet_slot) {
            self.jet_slot = flight.jet_slot;
        }
        self.jet_equip_delay_ms = flight.jet_equip_delay_ms;
        self.jet_delay_ms = flight.jet_delay_ms;
        self.side_ms = flight.side_ms.max(1);
        self.side_segments = flight.side_segments;
        self.side_random_max_ms = flight.side_random_max_ms;
        self.side_toggle_mode = flight.side_toggle_mode;
        self.quick_auto_side = flight.quick_auto_side;
        self.target_path = saved.target_path;
        self.target_title = saved.target_title;
        self.overlay_enabled = saved.overlay_enabled;
        self.overlay_opacity = saved.overlay_opacity.clamp(20, 100);
        self.update_mirror = saved.update_mirror;
    }

    fn enabled_mask(&self) -> usize {
        (usize::from(self.pickup_enabled) * PICKUP_ENABLED)
            | (usize::from(self.f_repeat_enabled) * F_REPEAT_ENABLED)
            | (usize::from(self.spear_enabled) * SPEAR_ENABLED)
            | (usize::from(self.quick_enabled) * QUICK_ENABLED)
            | (usize::from(self.side_enabled) * SIDE_ENABLED)
            | (usize::from(self.whip_enabled) * WHIP_ENABLED)
            | (usize::from(self.whip_switch_enabled) * WHIP_SWITCH_ENABLED)
            | (usize::from(self.spin_enabled) * SPIN_ENABLED)
            | (usize::from(self.whip_no_slow_enabled) * WHIP_NO_SLOW_ENABLED)
    }

    fn start(&mut self) {
        self.listening = None;
        self.error = None;
        if instance::other_window() {
            self.error = Some("另一个工具箱实例仍在运行，请先退出旧版（含托盘）。".into());
            return;
        }
        if self.target_path.is_empty() {
            self.page = Page::Settings;
            self.error = Some("请先选择运行进程。".into());
            return;
        }
        let result = self.settings().map_err(str::to_owned).and_then(|settings| {
            Worker::start(settings, self.enabled_mask()).map_err(|error| error.to_string())
        });
        match result {
            Ok(worker) => self.worker = Some(worker),
            Err(error) => self.error = Some(error),
        }
    }

    fn stop(&mut self) {
        self.worker = None;
        self.running = false;
    }

    fn fetch_online_manifest(&mut self) {
        self.online_manifest = None;
        self.update_status = Some("正在获取在线信息…".into());
        let mirror = self.update_mirror;
        let (sender, receiver) = mpsc::channel();
        self.update_receiver = Some(receiver);
        if let Err(error) = thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = sender.send(updates::fetch_manifest(mirror));
            })
        {
            self.update_receiver = None;
            self.update_status = Some(format!("检查失败：{error}"));
        }
    }

    fn download_update(&mut self, url: String, version: String) {
        if self.download_receiver.is_some() {
            return;
        }
        self.download_status = Some(format!("正在下载 v{version}…"));
        let (sender, receiver) = mpsc::channel();
        self.download_receiver = Some(receiver);
        if let Err(error) = thread::Builder::new()
            .name("update-download".into())
            .spawn(move || {
                let _ = sender.send(updates::download_update(&url, &version));
            })
        {
            self.download_receiver = None;
            self.download_status = Some(format!("启动下载失败：{error}"));
        }
    }

    fn capture_hotkey(&mut self, ctx: &egui::Context) -> bool {
        let Some(field) = self.listening else {
            return false;
        };
        let enabled = match field {
            HotkeyField::Pickup => self.pickup_enabled,
            HotkeyField::Spear | HotkeyField::Attack => self.spear_enabled,
            HotkeyField::Quick => self.quick_enabled,
            HotkeyField::Side => self.side_enabled,
            HotkeyField::Whip => self.whip_enabled,
            HotkeyField::WhipSwitch => self.whip_switch_enabled,
            HotkeyField::Spin => self.spin_enabled,
            HotkeyField::WhipNoSlow => self.whip_no_slow_enabled,
        };
        if !enabled {
            self.listening = None;
            self.error = None;
            return false;
        }
        let event = ctx.input_mut(|input| {
            let event = input.events.iter().position(|event| match event {
                egui::Event::Key {
                    pressed: true,
                    repeat: false,
                    ..
                } => true,
                egui::Event::PointerButton {
                    button,
                    pressed: false,
                    ..
                } => *button != egui::PointerButton::Primary || field == HotkeyField::Attack,
                _ => false,
            })?;
            Some(input.events.remove(event))
        });
        let (binding, mouse_click) = match event {
            Some(egui::Event::Key {
                key: egui::Key::Escape,
                ..
            }) => {
                self.listening = None;
                self.error = None;
                return false;
            }
            Some(egui::Event::Key { key, modifiers, .. }) => {
                let label = key.name();
                let Some(code) = keyboard_key(label).filter(|_| modifiers.is_none()) else {
                    self.error = Some("请按一个受支持的键盘键或鼠标键。Esc 可取消。".into());
                    return false;
                };
                (Binding::new(code, label), false)
            }
            Some(egui::Event::PointerButton { button, .. }) => (
                Binding::mouse(button),
                button == egui::PointerButton::Primary,
            ),
            _ => return false,
        };
        if matches!(
            field,
            HotkeyField::Quick | HotkeyField::Side | HotkeyField::Spin
        ) && binding.code == 0x20
        {
            self.error = Some("空格用于停止快速飞，请为飞行选择其他热键。".into());
            return false;
        }
        let defaults = [
            (HotkeyField::Pickup, self.pickup),
            (HotkeyField::Spear, self.spear),
            (HotkeyField::Quick, self.quick),
            (HotkeyField::Side, self.side),
            (HotkeyField::Whip, self.whip),
            (HotkeyField::WhipSwitch, self.whip_switch),
            (HotkeyField::Spin, self.spin),
            (HotkeyField::WhipNoSlow, self.whip_no_slow),
        ];
        if field != HotkeyField::Attack
            && defaults
                .iter()
                .any(|(other, assigned)| *other != field && binding.code == assigned.code)
        {
            self.error = Some("这个键已分配给另一个功能，请按其他键。".into());
            return false;
        }
        let current = match field {
            HotkeyField::Pickup => &mut self.pickup,
            HotkeyField::Spear => &mut self.spear,
            HotkeyField::Quick => &mut self.quick,
            HotkeyField::Side => &mut self.side,
            HotkeyField::Whip => &mut self.whip,
            HotkeyField::WhipSwitch => &mut self.whip_switch,
            HotkeyField::Spin => &mut self.spin,
            HotkeyField::WhipNoSlow => &mut self.whip_no_slow,
            HotkeyField::Attack => &mut self.attack,
        };
        *current = binding;
        self.listening = None;
        self.error = None;
        mouse_click
    }

    fn hotkey_button(&mut self, ui: &mut egui::Ui, field: HotkeyField) {
        let listening = self.listening == Some(field);
        let label = match field {
            HotkeyField::Pickup => self.pickup.label,
            HotkeyField::Spear => self.spear.label,
            HotkeyField::Quick => self.quick.label,
            HotkeyField::Side => self.side.label,
            HotkeyField::Whip => self.whip.label,
            HotkeyField::WhipSwitch => self.whip_switch.label,
            HotkeyField::Spin => self.spin.label,
            HotkeyField::WhipNoSlow => self.whip_no_slow.label,
            HotkeyField::Attack => self.attack.label,
        };
        let button = if listening {
            egui::Button::new(RichText::new("请按键 · Esc 取消").color(ACCENT))
                .fill(Color32::from_rgb(233, 243, 254))
                .stroke(Stroke::new(1.0_f32, ACCENT))
        } else {
            egui::Button::new(label)
        };
        let response =
            ui.add_sized(field_size(), button)
                .on_hover_text(if field == HotkeyField::Attack {
                    "点击后按键盘键或鼠标键"
                } else {
                    "支持字母、数字、功能键、空格、Tab 和鼠标右键、中键、侧键"
                });
        if response.clicked() {
            self.listening = if listening { None } else { Some(field) };
            self.error = None;
            ui.memory_mut(|memory| memory.surrender_focus(response.id));
        }
    }

    fn battle_settings(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            feature_header(ui, "长枪瞬切", &mut self.spear_enabled);
            if !self.spear_enabled {
                return;
            }
            ui.colored_label(MUTED, "按一次执行一轮：攻击 → 切换位 → 武器位");
            ui.add_space(4.0);
            ui.add_enabled_ui(self.worker.is_none(), |ui| {
                row(ui, "瞬切热键", |ui| {
                    self.hotkey_button(ui, HotkeyField::Spear)
                });
                row(ui, "武器位（主键盘）", |ui| {
                    slot_field(ui, "weapon", &mut self.weapon)
                });
                row(ui, "切换位（主键盘）", |ui| {
                    slot_field(ui, "swap", &mut self.swap)
                });
                row(ui, "攻击键", |ui| {
                    self.hotkey_button(ui, HotkeyField::Attack)
                });
                ui.add_space(5.0);
                ui.separator();
                row(ui, "动作延迟", |ui| {
                    if ui
                        .add_sized(field_size(), egui::Button::new("恢复默认"))
                        .clicked()
                    {
                        self.delays = DEFAULT_DELAYS;
                    }
                });
                for (index, (label, delay)) in
                    DELAY_FIELDS.into_iter().zip(&mut self.delays).enumerate()
                {
                    row(ui, label, |ui| {
                        let minimum = if matches!(index, 2 | 4) { 20 } else { 0 };
                        ui.add_sized(
                            field_size(),
                            egui::DragValue::new(delay)
                                .suffix(" ms")
                                .range(minimum..=u32::MAX),
                        );
                    });
                }
                row(ui, "切回武器后冷却", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.spear_cooldown_ms).suffix(" ms"),
                    )
                    .on_hover_text("整轮完成后暂不接受下一次触发，避免伤害尚未生效就开始下一轮。");
                });
            });
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            feature_header(ui, "双鞭", &mut self.whip_enabled);
            if !self.whip_enabled {
                return;
            }
            ui.colored_label(MUTED, "按住热键，从一鞭开始交替蓄力，松手停止。");
            ui.add_space(4.0);
            ui.add_enabled_ui(self.worker.is_none(), |ui| {
                row(ui, "双鞭热键", |ui| {
                    self.hotkey_button(ui, HotkeyField::Whip)
                });
                for (index, label) in ["一鞭位（主键盘）", "二鞭位（主键盘）"]
                    .into_iter()
                    .enumerate()
                {
                    row(ui, label, |ui| {
                        slot_field(ui, label, &mut self.whip_slots[index])
                    });
                }
                row(ui, "切槽后等待", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.whip_equip_ms).suffix(" ms"),
                    )
                    .on_hover_text("两把鞭子共用此设置。松开数字键后等待，再按住左键蓄力。");
                });
                row(ui, "蓄力时间", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.whip_charge_ms)
                            .range(1..=u32::MAX)
                            .suffix(" ms"),
                    );
                });
                row(ui, "松开后等待", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.whip_wait_ms).suffix(" ms"),
                    );
                });
            });
        });
    }

    fn whip_no_slow_settings(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            feature_header(ui, "卡鞭子无减速", &mut self.whip_no_slow_enabled);
            if !self.whip_no_slow_enabled {
                return;
            }
            ui.colored_label(MUTED, "先装备鞭子，按一次热键执行。");
            ui.add_enabled_ui(self.worker.is_none(), |ui| {
                row(ui, "热键", |ui| {
                    self.hotkey_button(ui, HotkeyField::WhipNoSlow)
                });
                row(ui, "左轮位（主键盘）", |ui| {
                    slot_field(ui, "whip_no_slow_revolver", &mut self.revolver_slot)
                });
                for (label, duration_ms) in [
                    ("切左轮前蓄力", &mut self.whip_no_slow_charge_ms),
                    ("切左轮后等待", &mut self.revolver_wait_ms),
                ] {
                    row(ui, label, |ui| {
                        let mut seconds = f64::from(*duration_ms) / 1000.0;
                        if ui
                            .add_sized(
                                field_size(),
                                egui::DragValue::new(&mut seconds)
                                    .speed(0.01)
                                    .range(0.01..=60.0)
                                    .fixed_decimals(2)
                                    .suffix(" 秒"),
                            )
                            .changed()
                        {
                            *duration_ms = (seconds * 1000.0).round() as u32;
                        }
                    });
                }
            });
        });
    }

    fn whip_switch_settings(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            feature_header(ui, "鞭子瞬切", &mut self.whip_switch_enabled);
            if !self.whip_switch_enabled {
                return;
            }
            ui.colored_label(MUTED, "先装备鞭子，再按住热键循环。");
            ui.add_space(4.0);
            ui.add_enabled_ui(self.worker.is_none(), |ui| {
                row(ui, "鞭子瞬切热键", |ui| {
                    self.hotkey_button(ui, HotkeyField::WhipSwitch)
                });
                row(ui, "鞭子武器位（主键盘）", |ui| {
                    slot_field(ui, "whip_switch_weapon", &mut self.whip_switch_slots[0])
                });
                row(ui, "物品位（主键盘）", |ui| {
                    slot_field(ui, "whip_switch_item", &mut self.whip_switch_slots[1])
                });
                row(ui, "蓄力时间", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.whip_switch_charge_ms)
                            .range(1..=u32::MAX)
                            .suffix(" ms"),
                    );
                });
                row(ui, "伤害生效等待", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.whip_switch_recovery_ms).suffix(" ms"),
                    );
                });
                row(ui, "切换位到切回间隔", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.whip_switch_return_ms).suffix(" ms"),
                    )
                    .on_hover_text("从松开切换位数字键开始，到按下武器位数字键为止。");
                });
                row(ui, "切回武器后冷却", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.whip_switch_cooldown_ms).suffix(" ms"),
                    )
                    .on_hover_text("本轮切回武器后等待，再开始下一轮；松开热键也会完成当前轮次。");
                });
            });
        });
    }

    fn flight_settings(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            feature_header(ui, "快速飞", &mut self.quick_enabled);
            if !self.quick_enabled {
                return;
            }
            ui.colored_label(MUTED, "按热键起飞。按住左键或长按空格接管，松开结束。");
            ui.add_space(4.0);
            ui.add_enabled_ui(self.worker.is_none(), |ui| {
                row(ui, "快速飞热键", |ui| {
                    self.hotkey_button(ui, HotkeyField::Quick)
                });
                row(ui, "喷包位（主键盘）", |ui| {
                    slot_field(ui, "jet", &mut self.jet_slot)
                });
                row(ui, "切到喷包后等待左键", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.jet_equip_delay_ms).suffix(" ms"),
                    );
                });
                row(ui, "按住左键后等待跳跃", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.jet_delay_ms).suffix(" ms"),
                    );
                });
                row(ui, "快速飞时自动左右飞", |ui| {
                    ui.checkbox(&mut self.quick_auto_side, "持续到接管结束");
                });
                ui.add_enabled_ui(self.quick_auto_side, |ui| {
                    ui.label(RichText::new("起飞分段速度").strong());
                    ui.colored_label(MUTED, "按起飞后的时间切换 A/D 单边持续时间。");
                    let mut remove_segment = None;
                    for (index, segment) in self.side_segments.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("第 {} 段：起飞后", index + 1));
                            ui.add(egui::DragValue::new(&mut segment.after_ms).suffix(" ms"));
                            ui.label("每边");
                            ui.add(egui::DragValue::new(&mut segment.duration_ms).suffix(" ms"));
                            if ui.small_button("移除").clicked() {
                                remove_segment = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove_segment {
                        self.side_segments.remove(index);
                    }
                    if ui.small_button("＋ 添加分段").clicked() {
                        let after_ms = self
                            .side_segments
                            .last()
                            .map_or(3000, |segment| segment.after_ms.saturating_add(1000));
                        self.side_segments.push(flight::SideSegment {
                            after_ms,
                            duration_ms: 400,
                        });
                    }
                });
            });
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            feature_header(ui, "左右飞", &mut self.side_enabled);
            if !self.side_enabled {
                return;
            }
            ui.colored_label(
                MUTED,
                if self.side_toggle_mode {
                    "按下热键开始交替按 A 和 D，再按一次停止。"
                } else {
                    "按住热键交替按 A 和 D，松手停止。"
                },
            );
            ui.add_space(4.0);
            ui.add_enabled_ui(self.worker.is_none(), |ui| {
                row(ui, "左右飞热键", |ui| {
                    self.hotkey_button(ui, HotkeyField::Side)
                });
                row(ui, "启动方式", |ui| {
                    ui.selectable_value(&mut self.side_toggle_mode, false, "按住开启");
                    ui.selectable_value(&mut self.side_toggle_mode, true, "按下切换");
                });
                row(ui, "每边持续时间", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.side_ms)
                            .suffix(" ms")
                            .range(1..=u32::MAX),
                    );
                });
                row(ui, "随机增加上限", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.side_random_max_ms).suffix(" ms"),
                    )
                    .on_hover_text("每次按 A 或 D 时各自重新抽取 0 至上限毫秒，设为 0 关闭。");
                });
            });
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            feature_header(ui, "转圈圈", &mut self.spin_enabled);
            if !self.spin_enabled {
                return;
            }
            ui.colored_label(
                MUTED,
                "按住热键持续向右转；每次移动像素越大，视角转得越快。",
            );
            ui.add_space(4.0);
            ui.add_enabled_ui(self.worker.is_none(), |ui| {
                row(ui, "转圈圈热键", |ui| {
                    self.hotkey_button(ui, HotkeyField::Spin)
                });
                row(ui, "每次移动像素", |ui| {
                    ui.add_sized(
                        field_size(),
                        egui::DragValue::new(&mut self.spin_pixels)
                            .range(1..=1000)
                            .suffix(" px"),
                    );
                });
            });
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        if self.online_manifest.is_none()
            && self.update_receiver.is_none()
            && self.update_status.is_none()
        {
            self.fetch_online_manifest();
        }
        let refresh = Duration::from_secs(1);
        if self
            .apps_refreshed
            .is_none_or(|instant| instant.elapsed() >= refresh)
        {
            self.running_apps = processes::running_apps();
            if self.target_title.is_empty() {
                if let Some(app) = self
                    .running_apps
                    .iter()
                    .find(|app| processes::same_path(&app.path, &self.target_path))
                {
                    self.target_title.clone_from(&app.title);
                }
            }
            self.apps_refreshed = Some(Instant::now());
        }
        ui.ctx().request_repaint_after(refresh);
        card(ui, |ui| {
            let current = self
                .running_apps
                .iter()
                .find(|app| processes::same_path(&app.path, &self.target_path))
                .map(|app| app.title.as_str())
                .unwrap_or_else(|| {
                    if self.target_path.is_empty() {
                        "选择窗口"
                    } else if self.target_title.is_empty() {
                        "已选择的窗口未打开"
                    } else {
                        &self.target_title
                    }
                })
                .to_owned();
            row_with_width(
                ui,
                RichText::new("运行进程").size(19.0).strong(),
                WINDOW_FIELD_WIDTH,
                |ui| {
                    ui.add_enabled_ui(self.worker.is_none(), |ui| {
                        dropdown(ui, "target_window", &current, WINDOW_FIELD_WIDTH, |ui| {
                            if self.running_apps.is_empty() {
                                ui.colored_label(MUTED, "没有可选的窗口");
                            }
                            egui::ScrollArea::vertical()
                                .max_height(270.0)
                                .show(ui, |ui| {
                                    for app in &self.running_apps {
                                        let selected =
                                            processes::same_path(&app.path, &self.target_path);
                                        let button = egui::Button::new(&app.title)
                                            .selected(selected)
                                            .truncate();
                                        if ui
                                            .add_sized([ui.available_width(), FIELD_HEIGHT], button)
                                            .on_hover_text(&app.title)
                                            .clicked()
                                        {
                                            self.target_path.clone_from(&app.path);
                                            self.target_title.clone_from(&app.title);
                                            self.error = None;
                                            ui.memory_mut(|mem| mem.close_popup());
                                        }
                                    }
                                });
                        });
                    });
                },
            );
            ui.colored_label(MUTED, "仅在所选应用位于前台时生效。");
            if self.worker.is_some() {
                ui.colored_label(MUTED, "停止热键后可以更换运行进程。");
            }
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            feature_header(ui, "功能悬浮窗", &mut self.overlay_enabled);
            ui.colored_label(MUTED, "启动热键后，在所选应用的屏幕左下侧显示功能状态。");
            row(ui, "不透明度", |ui| {
                ui.add_sized(
                    [FIELD_WIDTH, FIELD_HEIGHT],
                    egui::Slider::new(&mut self.overlay_opacity, 20..=100).suffix("%"),
                );
            });
            ui.colored_label(MUTED, "数值越低越透明；设置会自动保存。");
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("在线更新与日志").size(19.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("检查更新").clicked() {
                        self.fetch_online_manifest();
                    }
                    egui::ComboBox::from_id_salt("update_source")
                        .selected_text(if self.update_mirror {
                            "镜像源"
                        } else {
                            "官方源"
                        })
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(!self.update_mirror, "官方源").clicked() {
                                self.update_mirror = false;
                                self.fetch_online_manifest();
                            }
                            if ui.selectable_label(self.update_mirror, "镜像源").clicked() {
                                self.update_mirror = true;
                                self.fetch_online_manifest();
                            }
                        });
                });
            });
            ui.add_space(8.0);
            if let Some(status) = &self.update_status {
                ui.label(status);
            }
            if let Some(status) = &self.download_status {
                ui.label(status);
            }
            if let Some(manifest) = self.online_manifest.clone() {
                let newer = updates::is_newer(&manifest.version, CURRENT_VERSION);
                ui.label(if newer {
                    format!(
                        "发现新版本 v{}（当前 v{}）",
                        manifest.version, CURRENT_VERSION
                    )
                } else {
                    format!("当前版本 v{} 已是最新版本", CURRENT_VERSION)
                });
                if newer {
                    ui.horizontal(|ui| {
                        let download_url = if self.update_mirror {
                            manifest.mirror_download_url.clone()
                        } else {
                            manifest.download_url.clone()
                        };
                        let downloading = self.download_receiver.is_some();
                        let label = if downloading {
                            "正在下载…"
                        } else {
                            "下载并切换快捷方式"
                        };
                        if ui
                            .add_enabled(
                                !downloading,
                                egui::Button::new(RichText::new(label).color(Color32::WHITE))
                                    .fill(ACCENT),
                            )
                            .clicked()
                        {
                            self.download_update(download_url, manifest.version.clone());
                        }
                        ui.hyperlink_to("版本详情", &manifest.release_url);
                    });
                }
                ui.add_space(6.0);
                for entry in &manifest.changelog {
                    ui.label(format!("• {entry}"));
                }
            }
        });
    }
}

fn restore_binding(saved: u32, default: Binding, used: &[u32], reserve_space: bool) -> Binding {
    [
        saved,
        default.code,
        0x78,
        0x79,
        0x7a,
        0x7b,
        0x7c,
        0x7d,
        0x77,
        0x74,
    ]
    .into_iter()
    .filter(|code| *code != 1 && (!reserve_space || *code != 0x20) && !used.contains(code))
    .find_map(Binding::from_code)
    .expect("at least one function key is unassigned")
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let settings = SavedSettings {
            pickup: self.pickup.code,
            pickup_enabled: self.pickup_enabled,
            f_repeat_enabled: self.f_repeat_enabled,
            pickup_interval_ms: self.pickup_interval_ms,
            spear: self.spear.code,
            spear_enabled: self.spear_enabled,
            weapon: self.weapon,
            swap: self.swap,
            attack: self.attack.code,
            delays: self.delays,
            spear_cooldown_ms: self.spear_cooldown_ms,
            flight: SavedFlight {
                quick: self.quick.code,
                quick_enabled: self.quick_enabled,
                jet_slot: self.jet_slot,
                jet_equip_delay_ms: self.jet_equip_delay_ms,
                jet_delay_ms: self.jet_delay_ms,
                side: self.side.code,
                side_enabled: self.side_enabled,
                side_ms: self.side_ms,
                side_segments: self.side_segments.clone(),
                side_random_max_ms: self.side_random_max_ms,
                side_toggle_mode: self.side_toggle_mode,
                quick_auto_side: self.quick_auto_side,
            },
            whip: SavedWhip {
                key: self.whip.code,
                enabled: self.whip_enabled,
                slots: self.whip_slots,
                charge_ms: self.whip_charge_ms,
                wait_ms: self.whip_wait_ms,
            },
            whip_equip_ms: self.whip_equip_ms,
            whip_switch: SavedWhip {
                key: self.whip_switch.code,
                enabled: self.whip_switch_enabled,
                slots: self.whip_switch_slots,
                charge_ms: self.whip_switch_charge_ms,
                wait_ms: self.whip_switch_recovery_ms,
            },
            whip_switch_cooldown_ms: self.whip_switch_cooldown_ms,
            whip_switch_return_ms: self.whip_switch_return_ms,
            spin: SavedSpin {
                key: self.spin.code,
                enabled: self.spin_enabled,
                pixels: self.spin_pixels,
            },
            whip_no_slow: SavedWhipNoSlow {
                key: self.whip_no_slow.code,
                enabled: self.whip_no_slow_enabled,
                slot: self.revolver_slot,
                charge_ms: self.whip_no_slow_charge_ms,
                revolver_wait_ms: self.revolver_wait_ms,
            },
            update_mirror: self.update_mirror,
            target_path: self.target_path.clone(),
            target_title: self.target_title.clone(),
            overlay_enabled: self.overlay_enabled,
            overlay_opacity: self.overlay_opacity,
        };
        eframe::set_value(storage, SETTINGS_KEY, &settings);
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(2)
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let consumed_click = self.capture_hotkey(ctx);
        if let Some(receiver) = &self.download_receiver {
            match receiver.try_recv() {
                Ok(Ok(path)) => {
                    self.download_status =
                        Some(match shortcuts::set_desktop_shortcut_target(&path) {
                            Ok(()) => {
                                format!(
                                    "下载完成，桌面快捷方式已切换至 {}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                )
                            }
                            Err(error) => format!("下载完成，但快捷方式更新失败：{error}"),
                        });
                    self.download_receiver = None;
                }
                Ok(Err(error)) => {
                    self.download_status = Some(format!("下载失败：{error}"));
                    self.download_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.download_status = Some("下载失败：下载任务已退出。".into());
                    self.download_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
            }
        }
        if let Some(receiver) = &self.update_receiver {
            match receiver.try_recv() {
                Ok(Ok(manifest)) => {
                    self.update_status = None;
                    self.online_manifest = Some(manifest);
                    self.update_receiver = None;
                }
                Ok(Err(error)) => {
                    self.update_status = Some(format!("在线获取失败：{error}"));
                    self.update_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.update_status = Some("在线获取失败：网络请求已退出。".into());
                    self.update_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
            }
        }
        if let Some(worker) = &self.worker {
            match worker.events.try_recv() {
                Ok(WorkerEvent::Ready) => self.running = true,
                Ok(WorkerEvent::Failed(error)) => {
                    self.stop();
                    self.error = Some(error);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.stop();
                    self.error = Some("热键服务已退出。".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::TopBottomPanel::bottom("controls")
            .frame(
                egui::Frame::none()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(24.0, 14.0)),
            )
            .show(ctx, |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(Color32::from_rgb(172, 36, 36), error);
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    let status = if self.running && self.enabled_mask() == 0 {
                        "功能已暂停"
                    } else if self.running {
                        "运行中"
                    } else if self.worker.is_some() {
                        "正在启动"
                    } else {
                        "已停止"
                    };
                    let (dot, _) =
                        ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                    let color = if self.running && self.enabled_mask() != 0 {
                        Color32::from_rgb(16, 124, 65)
                    } else {
                        MUTED
                    };
                    ui.painter().circle_filled(dot.center(), 3.5, color);
                    ui.label(status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let active = self.worker.is_some();
                        let label = if active {
                            "停止热键"
                        } else {
                            "启动热键"
                        };
                        let action = egui::Button::new(RichText::new(label).color(Color32::WHITE))
                            .fill(ACCENT)
                            .min_size(egui::vec2(106.0, 34.0));
                        if ui.add_enabled(!consumed_click, action).clicked() {
                            if active {
                                self.stop();
                            } else {
                                self.start();
                            }
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(243, 245, 249))
                    .inner_margin(24.0),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.label(RichText::new("宏设置").size(28.0).strong());
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.page, Page::Pickup, "拾取");
                            ui.selectable_value(&mut self.page, Page::Battle, "战斗");
                            ui.selectable_value(&mut self.page, Page::Flight, "飞行");
                            ui.selectable_value(&mut self.page, Page::Settings, "设置");
                        });
                        ui.add_space(8.0);
                        ui.add_enabled_ui(!consumed_click, |ui| {
                            match self.page {
                                Page::Flight => return self.flight_settings(ui),
                                Page::Settings => return self.settings_page(ui),
                                Page::Battle => {
                                    self.battle_settings(ui);
                                    ui.add_space(12.0);
                                    self.whip_switch_settings(ui);
                                    ui.add_space(12.0);
                                    self.whip_no_slow_settings(ui);
                                    return;
                                }
                                Page::Pickup => {}
                            }
                            card(ui, |ui| {
                                feature_header(ui, "快捷拾取", &mut self.pickup_enabled);
                                if !self.pickup_enabled {
                                    return;
                                }
                                ui.label(
                                    RichText::new("按住拾取热键，连续发送滚轮下与 F。")
                                        .color(MUTED),
                                );
                                ui.add_space(4.0);
                                ui.add_enabled_ui(self.worker.is_none(), |ui| {
                                    row(ui, "拾取热键", |ui| {
                                        self.hotkey_button(ui, HotkeyField::Pickup)
                                    });
                                    row(ui, "拾取连发间隔", |ui| {
                                        ui.add_sized(
                                            field_size(),
                                            egui::DragValue::new(&mut self.pickup_interval_ms)
                                                .suffix(" ms")
                                                .range(1..=u32::MAX),
                                        );
                                    });
                                });
                            });
                            ui.add_space(12.0);
                            card(ui, |ui| {
                                feature_header(ui, "长按 F 连点", &mut self.f_repeat_enabled);
                                if self.f_repeat_enabled {
                                    ui.colored_label(
                                        MUTED,
                                        "单击 F 照常，按住 250 ms 后开始连点。",
                                    );
                                }
                            });
                        });
                    });
            });
        if let Some(worker) = &self.worker {
            worker.enabled.store(self.enabled_mask(), Ordering::Relaxed);
        }
        if let Some(overlay) = &self.overlay {
            let target = (self.running && self.overlay_enabled && !self.target_path.is_empty())
                .then_some(self.target_path.as_str());
            let cooldown = self
                .worker
                .as_ref()
                .map_or(0, |worker| worker.cooldown.load(Ordering::Relaxed));
            overlay.update(target, self.enabled_mask(), cooldown, self.overlay_opacity);
        }
    }
}

fn setup_style(ctx: &egui::Context) {
    let mut style = egui::Style {
        visuals: egui::Visuals::light(),
        ..Default::default()
    };
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    style.spacing.interact_size.y = FIELD_HEIGHT;
    style.spacing.button_padding = egui::vec2(10.0, 4.0);
    style.visuals.extreme_bg_color = Color32::from_rgb(246, 248, 252);
    let widgets = &mut style.visuals.widgets;
    widgets.inactive.bg_fill = Color32::from_rgb(247, 249, 253);
    widgets.inactive.weak_bg_fill = widgets.inactive.bg_fill;
    widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(215, 224, 235));
    widgets.hovered.bg_fill = Color32::from_rgb(232, 242, 253);
    widgets.hovered.weak_bg_fill = widgets.hovered.bg_fill;
    widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(147, 190, 230));
    widgets.active.bg_fill = Color32::from_rgb(219, 236, 251);
    widgets.active.weak_bg_fill = widgets.active.bg_fill;
    widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    for state in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.rounding = egui::Rounding::same(7.0);
        state.expansion = 0.0;
    }
    style.visuals.selection.bg_fill = Color32::from_rgb(207, 228, 250);
    style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    ctx.set_style(style);

    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
    let fonts_dir = std::path::PathBuf::from(windir).join("Fonts");
    let mut fonts = egui::FontDefinitions::default();
    for (name, filename) in [("cjk", "msyh.ttc"), ("segoe", "segoeui.ttf")] {
        if let Ok(bytes) = std::fs::read(fonts_dir.join(filename)) {
            fonts
                .font_data
                .insert(name.into(), egui::FontData::from_owned(bytes));
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, name.into());
        }
    }
    ctx.set_fonts(fonts);
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(223, 228, 236)))
        .rounding(10.0)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn feature_header(ui: &mut egui::Ui, label: &str, enabled: &mut bool) {
    row_with_width(ui, RichText::new(label).size(19.0).strong(), 48.0, |ui| {
        let (rect, mut response) =
            ui.allocate_exact_size(egui::vec2(48.0, FIELD_HEIGHT), egui::Sense::click());
        if response.clicked() {
            *enabled = !*enabled;
            response.mark_changed();
            if !*enabled {
                ui.memory_mut(|memory| memory.close_popup());
            }
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *enabled, label)
        });
        let response = response.on_hover_text(if *enabled {
            "关闭此功能"
        } else {
            "开启此功能"
        });
        let track = egui::Rect::from_center_size(rect.center(), egui::vec2(48.0, 24.0));
        let fill = match (*enabled, response.hovered()) {
            (true, true) => Color32::from_rgb(31, 105, 223),
            (true, false) => Color32::from_rgb(47, 124, 246),
            (false, true) => Color32::from_rgb(195, 199, 205),
            (false, false) => Color32::from_rgb(222, 225, 230),
        };
        let position = ui.ctx().animate_bool(response.id, *enabled);
        let knob = egui::pos2(
            egui::lerp(track.left() + 12.0..=track.right() - 12.0, position),
            track.center().y,
        );
        let painter = ui.painter();
        painter.rect_filled(track, 12.0, fill);
        painter.circle_filled(knob, 8.5, Color32::WHITE);
        if response.has_focus() {
            painter.rect_stroke(track.expand(2.0), 14.0, Stroke::new(1.0_f32, ACCENT));
        }
    });
}

fn row(ui: &mut egui::Ui, label: impl Into<egui::WidgetText>, field: impl FnOnce(&mut egui::Ui)) {
    row_with_width(ui, label, FIELD_WIDTH, field);
}

fn row_with_width(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    width: f32,
    field: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), FIELD_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(label);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, FIELD_HEIGHT),
                    egui::Layout::top_down(egui::Align::Center),
                    field,
                );
            });
        },
    );
}

fn field_size() -> [f32; 2] {
    [FIELD_WIDTH, FIELD_HEIGHT]
}

fn slot_field(ui: &mut egui::Ui, id: &str, value: &mut u8) {
    dropdown(ui, id, &value.to_string(), FIELD_WIDTH, |ui| {
        for number in SLOTS {
            let button = egui::Button::new(number.to_string()).selected(number == *value);
            if ui
                .add_sized([ui.available_width(), FIELD_HEIGHT], button)
                .clicked()
            {
                *value = number;
                ui.memory_mut(|mem| mem.close_popup());
            }
        }
    });
}

fn dropdown(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    width: f32,
    options: impl FnOnce(&mut egui::Ui),
) {
    let popup_id = ui.make_persistent_id(id);
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding.x = 24.0;
            ui.add_sized([width, FIELD_HEIGHT], egui::Button::new(label).truncate())
        })
        .inner
        .on_hover_text(label);
    if response.clicked() {
        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
    }
    let arrow = egui::pos2(response.rect.right() - 14.0, response.rect.center().y);
    ui.painter().add(egui::Shape::line(
        vec![
            arrow + egui::vec2(-3.5, -2.0),
            arrow + egui::vec2(0.0, 2.0),
            arrow + egui::vec2(3.5, -2.0),
        ],
        Stroke::new(1.2_f32, MUTED),
    ));
    egui::popup::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::popup::PopupCloseBehavior::CloseOnClickOutside,
        options,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestStorage(std::collections::HashMap<String, String>);

    impl eframe::Storage for TestStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn turning_off_a_feature_hides_and_preserves_its_details() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx);
        app.whip_switch_charge_ms = 275;
        let mut render = |events| {
            let input = egui::RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(550.0, 500.0),
                )),
                ..Default::default()
            };
            ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.whip_switch_settings(ui));
            })
        };
        let expanded = render(Vec::new());
        let shows_charge = |output: &egui::FullOutput| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "蓄力时间")
            })
        };
        assert!(shows_charge(&expanded));
        let position = expanded
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(track) if track.fill == Color32::from_rgb(47, 124, 246) => {
                    Some(track.rect.center())
                }
                _ => None,
            })
            .expect("expanded switch track");
        let pointer = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        render(vec![egui::Event::PointerMoved(position), pointer(true)]);
        let collapsed = render(vec![pointer(false)]);
        assert!(!shows_charge(&collapsed));
        render(vec![pointer(true)]);
        let reopened = render(vec![pointer(false)]);
        assert!(shows_charge(&reopened));
        assert!(app.whip_switch_enabled);
        assert_eq!(app.whip_switch_charge_ms, 275);
    }

    #[test]
    fn saves_and_restores_macro_configuration() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx);
        app.pickup = Binding::mouse(egui::PointerButton::Extra2);
        app.spear = Binding::from_code(0x77).unwrap();
        app.attack = Binding::mouse(egui::PointerButton::Secondary);
        app.weapon = 7;
        app.swap = 4;
        app.pickup_interval_ms = 13;
        app.delays = [2, 300, 2, 4, 8];
        app.pickup_enabled = false;
        app.quick = Binding::from_code(0x79).unwrap();
        app.quick_enabled = false;
        app.jet_slot = 5;
        app.jet_delay_ms = 75;
        app.jet_equip_delay_ms = 165;
        app.side_ms = 845;
        app.side_segments = vec![
            flight::SideSegment {
                after_ms: 1000,
                duration_ms: 210,
            },
            flight::SideSegment {
                after_ms: 2000,
                duration_ms: 320,
            },
        ];
        app.side_random_max_ms = 35;
        app.side_toggle_mode = true;
        app.quick_auto_side = true;
        app.spin = Binding::from_code(0x7b).unwrap();
        app.spin_enabled = false;
        app.spin_pixels = 23;
        app.target_path = "C:\\Games\\preternatural.exe".into();
        app.target_title = "超自然行动组".into();
        app.overlay_enabled = false;
        app.overlay_opacity = 67;
        app.whip_enabled = false;
        app.whip = Binding::from_code(0x7a).unwrap();
        app.whip_slots = [7, 3];
        app.whip_charge_ms = 600;
        app.whip_wait_ms = 140;
        app.whip_switch_enabled = false;
        app.whip_switch = Binding::from_code(0x7b).unwrap();
        app.whip_switch_slots = [4, 6];
        app.whip_switch_charge_ms = 340;
        app.whip_switch_recovery_ms = 280;

        let mut storage = TestStorage::default();
        eframe::App::save(&mut app, &mut storage);
        let mut reopened = App::new(&ctx);
        reopened.restore(&storage);
        assert_eq!(reopened.pickup.label, "鼠标侧键 2");
        assert_eq!(reopened.spear.label, "F8");
        assert_eq!(reopened.attack.label, "鼠标右键");
        assert_eq!((reopened.weapon, reopened.swap), (7, 4));
        assert_eq!(reopened.pickup_interval_ms, 13);
        assert_eq!(reopened.delays, [2, 300, 20, 4, 20]);
        assert_eq!(
            reopened.enabled_mask(),
            SPEAR_ENABLED | SIDE_ENABLED | F_REPEAT_ENABLED
        );
        assert_eq!(reopened.quick.label, "F10");
        assert_eq!(reopened.jet_slot, 5);
        assert_eq!(reopened.jet_delay_ms, 75);
        assert_eq!(reopened.settings().unwrap().jet_equip_delay_ms, 165);
        assert_eq!(reopened.side_ms, 845);
        assert_eq!(reopened.side_segments, app.side_segments);
        assert_eq!(reopened.side_random_max_ms, 35);
        assert!(reopened.side_toggle_mode);
        assert!(reopened.quick_auto_side);
        assert_eq!(reopened.spin.label, "F12");
        assert!(!reopened.spin_enabled);
        assert_eq!(reopened.spin_pixels, 23);
        assert_eq!(reopened.settings().unwrap().target_path, app.target_path);
        assert_eq!(reopened.target_title, "超自然行动组");
        assert!(!reopened.overlay_enabled);
        assert_eq!(reopened.overlay_opacity, 67);
        assert_eq!(reopened.whip.label, "F11");
        assert_eq!(reopened.whip_slots, [7, 3]);
        assert_eq!(reopened.whip_charge_ms, 600);
        assert_eq!(reopened.whip_wait_ms, 140);
        assert_eq!(reopened.whip_switch.label, "F12");
        assert_eq!(reopened.whip_switch_slots, [4, 6]);
        assert_eq!(reopened.whip_switch_charge_ms, 340);
        assert_eq!(reopened.whip_switch_recovery_ms, 280);
        assert_eq!((reopened.weapon, reopened.swap), (7, 4));
        assert!(reopened.worker.is_none());
    }

    fn input(ctx: &egui::Context, app: &mut App, event: egui::Event) {
        let input = egui::RawInput {
            events: vec![event],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            app.capture_hotkey(ctx);
        });
    }

    fn press(ctx: &egui::Context, app: &mut App, key: egui::Key) {
        input(
            ctx,
            app,
            egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        );
    }

    fn click(ctx: &egui::Context, app: &mut App, button: egui::PointerButton) {
        input(
            ctx,
            app,
            egui::Event::PointerButton {
                pos: egui::pos2(70.0, 70.0),
                button,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        );
    }

    #[test]
    fn hotkey_capture_accepts_keys_and_keeps_previous_binding_on_cancel() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx);
        app.listening = Some(HotkeyField::Pickup);
        press(&ctx, &mut app, egui::Key::H);
        assert!(app.error.is_some());
        assert_eq!(app.pickup.label, "G");
        press(&ctx, &mut app, egui::Key::F8);
        assert_eq!(app.pickup.label, "F8");
        assert!(app.listening.is_none());
        app.listening = Some(HotkeyField::Spear);
        press(&ctx, &mut app, egui::Key::Escape);
        assert_eq!(app.spear.label, "H");
        assert!(app.listening.is_none());
        assert_eq!(app.settings().unwrap().pickup_interval_ms, 2);
        app.pickup_interval_ms = 24;
        assert_eq!(app.settings().unwrap().pickup_interval_ms, 24);
    }

    #[test]
    fn mouse_capture_excludes_left_click_from_triggers_but_keeps_it_for_attacking() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx);
        app.listening = Some(HotkeyField::Pickup);
        click(&ctx, &mut app, egui::PointerButton::Primary);
        assert_eq!(app.settings().unwrap().pickup, u32::from(b'G'));
        assert!(app.listening.is_some());
        click(&ctx, &mut app, egui::PointerButton::Extra1);
        assert_eq!(app.settings().unwrap().pickup, 5);
        app.listening = Some(HotkeyField::Spear);
        click(&ctx, &mut app, egui::PointerButton::Extra1);
        assert!(app.error.is_some());
        click(&ctx, &mut app, egui::PointerButton::Middle);
        assert_eq!(app.settings().unwrap().spear, 4);
        app.listening = Some(HotkeyField::Attack);
        click(&ctx, &mut app, egui::PointerButton::Secondary);
        assert_eq!(app.settings().unwrap().attack, 2);
        app.listening = Some(HotkeyField::Attack);
        click(&ctx, &mut app, egui::PointerButton::Primary);
        assert_eq!(app.settings().unwrap().attack, 1);
    }
}

fn main() -> eframe::Result<()> {
    if unsafe { IsUserAnAdmin() } == 0 {
        startup_message("请通过 Windows 用户账户控制提示，以管理员身份运行工具箱。");
        return Ok(());
    }
    let _instance = match instance::Instance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            startup_message(
                "已有工具箱在运行。请关闭原窗口，或在托盘图标上右键选择「退出」，再打开本版。",
            );
            return Ok(());
        }
        Err(error) => {
            startup_message(&format!("无法检查工具箱是否已在运行：{error}"));
            return Ok(());
        }
    };
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/toolbox.png"))
        .expect("embedded application icon must be a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("LvwanMutationToolbox")
            .with_icon(icon)
            .with_inner_size([550.0, 780.0])
            .with_min_inner_size([450.0, 490.0]),
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(|cc| {
            let mut app = App::new(&cc.egui_ctx);
            if let Some(storage) = cc.storage {
                app.restore(&*storage);
            }
            if let Err(error) = shortcuts::create_desktop_shortcut() {
                app.error = Some(format!("桌面快捷方式创建失败：{error}"));
            }
            if let Ok(handle) = cc.window_handle() {
                if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                    match tray::Tray::install(handle.hwnd.get() as _) {
                        Ok(tray) => app.tray = Some(tray),
                        Err(error) => app.error = Some(format!("托盘初始化失败：{error}")),
                    }
                }
            }
            match overlay::Overlay::create(app.overlay_opacity) {
                Ok(overlay) => app.overlay = Some(overlay),
                Err(error) => {
                    app.error
                        .get_or_insert_with(|| format!("功能悬浮窗初始化失败：{error}"));
                }
            }
            Ok(Box::new(app))
        }),
    )
}

fn startup_message(message: &str) {
    let title: Vec<u16> = WINDOW_TITLE.encode_utf16().chain([0]).collect();
    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONWARNING | MB_SETFOREGROUND,
        );
    }
}
