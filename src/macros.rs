use crate::flight::{QuickFlight, SideFlight};
use crate::processes::{self, ForegroundTarget};
use crate::whip::{DualWhip, WhipSwitch};
use crate::windows::{key, mouse_button, mouse_move, scan, send, wheel_down, Hotkeys, Input};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const PICKUP_HOLD: Duration = Duration::from_millis(1);
const POLL_INTERVAL: Duration = Duration::from_millis(1);

pub const DEFAULT_PICKUP_INTERVAL_MS: u32 = 2;
pub const DEFAULT_DELAYS: [u32; 5] = [1, 420, 20, 10, 20];
pub const DEFAULT_SWITCH_COOLDOWN_MS: u32 = 420;
pub const PICKUP_ENABLED: usize = 1;
pub const SPEAR_ENABLED: usize = 2;
pub const QUICK_ENABLED: usize = 4;
pub const SIDE_ENABLED: usize = 8;
pub const WHIP_ENABLED: usize = 16;
pub const WHIP_SWITCH_ENABLED: usize = 32;
pub const SPIN_ENABLED: usize = 64;

pub struct Settings {
    pub pickup: u32,
    pub pickup_interval_ms: u32,
    pub spear: u32,
    pub weapon: u32,
    pub swap: u32,
    pub attack: u32,
    pub delays: [u32; 5],
    pub spear_cooldown_ms: u32,
    pub quick: u32,
    pub jet_slot: u32,
    pub jet_equip_delay_ms: u32,
    pub jet_delay_ms: u32,
    pub side: u32,
    pub side_ms: u32,
    pub side_segments: Vec<crate::flight::SideSegment>,
    pub side_random_max_ms: u32,
    pub side_toggle_mode: bool,
    pub quick_auto_side: bool,
    pub spin: u32,
    pub spin_pixels: u32,
    pub target_path: String,
    pub whip: u32,
    pub whip_slots: [u32; 2],
    pub whip_equip_ms: u32,
    pub whip_charge_ms: u32,
    pub whip_wait_ms: u32,
    pub whip_switch: u32,
    pub whip_switch_slots: [u32; 2],
    pub whip_switch_charge_ms: u32,
    pub whip_switch_recovery_ms: u32,
    pub whip_switch_cooldown_ms: u32,
}

pub fn keyboard_key(value: &str) -> Option<u32> {
    let value = value.trim().to_ascii_uppercase();
    match value.as_bytes() {
        [code @ (b'A'..=b'Z' | b'0'..=b'9')] => Some(u32::from(*code)),
        _ => match value.as_str() {
            "SPACE" => Some(0x20),
            "TAB" => Some(0x09),
            _ => value
                .strip_prefix('F')?
                .parse::<u32>()
                .ok()
                .filter(|number| (1..=24).contains(number))
                .map(|number| 0x6F + number),
        },
    }
}

fn pickup(f: u16, scroll: bool) -> io::Result<()> {
    let press = if scroll {
        send(&[wheel_down(), key(f, false)])
    } else {
        send(&[key(f, false)])
    };
    if press.is_ok() {
        thread::sleep(PICKUP_HOLD);
    }
    let release = send(&[key(f, true)]);
    press.and(release)
}

struct Spear {
    events: [Input; 6],
    delays: [Duration; 5],
    cooldown: Duration,
    next: Option<(usize, Instant)>,
    cooldown_until: Option<Instant>,
}

impl Spear {
    fn new(settings: &Settings) -> io::Result<Self> {
        let swap = scan(settings.swap)?;
        let weapon = scan(settings.weapon)?;
        let attack = match (
            mouse_button(settings.attack, false),
            mouse_button(settings.attack, true),
        ) {
            (Some(down), Some(up)) => [down, up],
            _ => {
                let scan_code = scan(settings.attack)?;
                [key(scan_code, false), key(scan_code, true)]
            }
        };
        let [attack_down, attack_up] = attack;
        Ok(Self {
            events: [
                attack_down,
                attack_up,
                key(swap, false),
                key(swap, true),
                key(weapon, false),
                key(weapon, true),
            ],
            delays: settings
                .delays
                .map(|ms| Duration::from_millis(u64::from(ms))),
            cooldown: Duration::from_millis(u64::from(settings.spear_cooldown_ms)),
            next: None,
            cooldown_until: None,
        })
    }

    fn update(&mut self, trigger: bool, now: Instant) -> io::Result<()> {
        if self.cooldown_until.is_some_and(|until| now >= until) {
            self.cooldown_until = None;
        }
        if trigger && self.next.is_none() && self.cooldown_until.is_none() {
            self.next = Some((0, now));
        }
        if let Some((index, due)) = self.next {
            if now >= due {
                send(&self.events[index..=index])?;
                if let Some(delay) = self.delays.get(index) {
                    self.next = Some((index + 1, Instant::now() + *delay));
                } else {
                    self.next = None;
                    self.cooldown_until = Some(Instant::now() + self.cooldown);
                }
            }
        }
        Ok(())
    }

    fn active(&self) -> bool {
        self.next.is_some() || self.cooldown_until.is_some()
    }

    fn cancel(&mut self) -> io::Result<()> {
        if let Some((index, _)) = self.next.take() {
            if matches!(index, 1 | 3 | 5) {
                send(&self.events[index..=index])?;
            }
            self.cooldown_until = None;
        }
        Ok(())
    }
}

impl Drop for Spear {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

pub fn run(
    settings: Settings,
    stop: &AtomicBool,
    enabled: &AtomicUsize,
    cooldown: &AtomicUsize,
    ready: impl FnOnce(),
) -> io::Result<()> {
    let f = scan(0x46)?;
    let pickup_interval = Duration::from_millis(u64::from(settings.pickup_interval_ms));
    let mut spear = Spear::new(&settings)?;
    let mut quick = QuickFlight::new(
        settings.jet_slot,
        settings.jet_equip_delay_ms,
        settings.jet_delay_ms,
    )?;
    let mut whip = DualWhip::new(
        settings.whip_slots,
        settings.whip_equip_ms,
        settings.whip_charge_ms,
        settings.whip_wait_ms,
    )?;
    let mut whip_switch = WhipSwitch::new(
        settings.whip_switch_slots[0],
        settings.whip_switch_slots[1],
        settings.whip_switch_charge_ms,
        settings.whip_switch_recovery_ms,
        settings.whip_switch_cooldown_ms,
    )?;
    let mut side = SideFlight::new(settings.side_random_max_ms)?;
    let hotkeys = Hotkeys::install([
        settings.pickup,
        settings.spear,
        settings.quick,
        settings.side,
        settings.whip,
        settings.whip_switch,
        settings.spin,
    ])?;
    let mut focus = ForegroundTarget::new(settings.target_path);
    let mut previous = 0;
    let mut scroll_next = true;
    let mut repeating = false;
    let mut side_toggled = false;
    let mut quick_started_at = None;
    let mut next_pickup = Instant::now();
    ready();

    while !stop.load(Ordering::Relaxed) {
        let target_pid = focus.matching_pid();
        let enabled = if target_pid != 0 {
            enabled.load(Ordering::Relaxed)
        } else {
            0
        };
        let down = hotkeys.read(enabled, target_pid);
        let focused = target_pid != 0 && processes::foreground_pid() == target_pid;
        if target_pid != 0 && !focused {
            hotkeys.suspend();
        }
        let (enabled, down) = if focused { (enabled, down) } else { (0, 0) };
        let pressed = (down & !previous) | hotkeys.take_pressed();
        previous = down;
        let left_pressed = hotkeys.take_left_press();
        let left_down = hotkeys.left_down();
        let instant_locked = spear.active() || whip_switch.active();
        let quick_start =
            pressed & QUICK_ENABLED != 0 && !left_pressed && !left_down && !instant_locked;
        if quick_start {
            spear.cancel()?;
            whip.cancel()?;
            whip_switch.cancel()?;
        }
        let now = Instant::now();
        quick.update(
            enabled & QUICK_ENABLED != 0,
            quick_start,
            hotkeys.space_down(),
            left_pressed,
            left_down,
            now,
        )?;
        if quick_start && quick.active() {
            quick_started_at = Some(now);
        }
        if !quick.active() {
            quick_started_at = None;
        }
        let side_enabled = enabled & SIDE_ENABLED != 0;
        let side_down = down & SIDE_ENABLED != 0;
        if !side_enabled || !focused {
            side_toggled = false;
        } else if settings.side_toggle_mode && pressed & SIDE_ENABLED != 0 {
            side_toggled = !side_toggled;
        }
        let side_hotkey_active = if settings.side_toggle_mode {
            side_toggled
        } else {
            side_down
        };
        let side_active =
            side_enabled && side_hotkey_active || settings.quick_auto_side && quick.active();
        let side_interval_ms = quick_started_at
            .filter(|_| settings.quick_auto_side && quick.active())
            .map(|started| {
                now.duration_since(started)
                    .as_millis()
                    .min(u128::from(u32::MAX)) as u32
            })
            .and_then(|elapsed| {
                settings
                    .side_segments
                    .iter()
                    .filter(|segment| segment.after_ms <= elapsed)
                    .max_by_key(|segment| segment.after_ms)
                    .map(|segment| segment.duration_ms.max(1))
            })
            .unwrap_or(settings.side_ms);
        side.update(side_active, side_interval_ms, now)?;
        let hold_whip = down & WHIP_ENABLED != 0 && !quick.active() && !instant_locked;
        if hold_whip {
            spear.cancel()?;
            whip_switch.cancel()?;
        }
        whip.update(hold_whip, Instant::now())?;
        if !focused || enabled & WHIP_SWITCH_ENABLED == 0 {
            whip_switch.cancel()?;
        } else {
            let hold_switch = down & WHIP_SWITCH_ENABLED != 0
                && !quick.active()
                && !whip.active()
                && !spear.active();
            if hold_switch {
                spear.cancel()?;
            }
            // Once a cycle begins, releasing the hotkey stops future cycles but
            // lets this one finish its return to the weapon slot.
            whip_switch.update(hold_switch, Instant::now())?;
        }
        if enabled & SPEAR_ENABLED != 0
            && !quick.active()
            && !whip.active()
            && !whip_switch.active()
        {
            spear.update(pressed & SPEAR_ENABLED != 0, Instant::now())?;
        } else {
            spear.cancel()?;
        }
        let cooling = (usize::from(spear.active()) * SPEAR_ENABLED)
            | (usize::from(whip_switch.active()) * WHIP_SWITCH_ENABLED);
        cooldown.store(cooling, Ordering::Relaxed);

        if down & SPIN_ENABLED != 0 {
            send(&[mouse_move(
                settings.spin_pixels.min(i32::MAX as u32) as i32,
                0,
            )])?;
        }

        let now = Instant::now();
        if pressed & PICKUP_ENABLED != 0 {
            repeating = scroll_next;
            scroll_next = !scroll_next;
            pickup(f, repeating)?;
            next_pickup = now + pickup_interval;
        } else if down & PICKUP_ENABLED != 0 && repeating && now >= next_pickup {
            pickup(f, true)?;
            next_pickup = now + pickup_interval;
        }

        let now = Instant::now();
        let mut delay = POLL_INTERVAL;
        if let Some((_, due)) = spear.next {
            delay = delay.min(due.saturating_duration_since(now));
        }
        if down & PICKUP_ENABLED != 0 && repeating {
            delay = delay.min(next_pickup.saturating_duration_since(now));
        }
        if !delay.is_zero() {
            thread::sleep(delay);
        }
    }
    Ok(())
}
