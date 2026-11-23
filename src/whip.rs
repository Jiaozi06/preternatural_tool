use crate::windows::{key, mouse_button, scan, send, Input};
use std::io;
use std::time::{Duration, Instant};

pub const DEFAULT_CHARGE_MS: u32 = 500;
pub const DEFAULT_WAIT_MS: u32 = 100;
pub const DEFAULT_RECOVERY_MS: u32 = 420;
pub const DEFAULT_DUAL_EQUIP_MS: u32 = 80;
const SLOT_HOLD: Duration = Duration::from_millis(20);
const SLOT_SETTLE: Duration = Duration::from_millis(30);
const SWITCH_STEP: Duration = Duration::from_millis(20);

enum Step {
    SlotHeld,
    Equipping,
    Charging,
    Waiting,
}

struct Active {
    slot: usize,
    step: Step,
    due: Instant,
}

pub struct DualWhip {
    slots: [u16; 2],
    equip_delay: Duration,
    charge: Duration,
    wait: Duration,
    stage: Option<Active>,
}

impl DualWhip {
    pub fn new(slots: [u32; 2], equip_ms: u32, charge_ms: u32, wait_ms: u32) -> io::Result<Self> {
        Ok(Self {
            slots: [scan(slots[0])?, scan(slots[1])?],
            equip_delay: Duration::from_millis(equip_ms.into()),
            charge: Duration::from_millis(charge_ms.max(1).into()),
            wait: Duration::from_millis(wait_ms.into()),
            stage: None,
        })
    }

    pub fn active(&self) -> bool {
        self.stage.is_some()
    }

    pub fn update(&mut self, held: bool, now: Instant) -> io::Result<()> {
        if !held {
            return self.cancel();
        }
        let Some(active) = &self.stage else {
            return self.select(0);
        };
        if now < active.due {
            return Ok(());
        }

        let slot = active.slot;
        match active.step {
            Step::SlotHeld => {
                send(&[key(self.slots[slot], true)])?;
                self.advance(slot, Step::Equipping, self.equip_delay);
            }
            Step::Equipping => {
                send(&[attack(false)])?;
                self.advance(slot, Step::Charging, self.charge);
            }
            Step::Charging => {
                send(&[attack(true)])?;
                self.advance(slot, Step::Waiting, self.wait);
            }
            Step::Waiting => self.select(1 - slot)?,
        }
        Ok(())
    }

    fn advance(&mut self, slot: usize, step: Step, delay: Duration) {
        self.stage = Some(Active {
            slot,
            step,
            due: Instant::now() + delay,
        });
    }

    fn select(&mut self, slot: usize) -> io::Result<()> {
        send(&[key(self.slots[slot], false)])?;
        self.advance(slot, Step::SlotHeld, SLOT_HOLD);
        Ok(())
    }

    pub fn cancel(&mut self) -> io::Result<()> {
        if let Some(active) = &self.stage {
            match active.step {
                Step::SlotHeld => send(&[key(self.slots[active.slot], true)])?,
                Step::Charging => send(&[attack(true)])?,
                Step::Equipping | Step::Waiting => {}
            }
        }
        self.stage = None;
        Ok(())
    }
}

impl Drop for DualWhip {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

pub struct WhipSwitch {
    events: [Input; 6],
    delays: [Duration; 6],
    cycle_cooldown: Duration,
    next: Option<(usize, Instant)>,
    cooldown_until: Option<Instant>,
}

impl WhipSwitch {
    pub fn new(
        weapon: u32,
        item: u32,
        charge_ms: u32,
        recovery_ms: u32,
        cooldown_ms: u32,
    ) -> io::Result<Self> {
        let weapon = scan(weapon)?;
        let item = scan(item)?;
        Ok(Self {
            events: [
                attack(false),
                attack(true),
                key(item, false),
                key(item, true),
                key(weapon, false),
                key(weapon, true),
            ],
            delays: [
                Duration::from_millis(charge_ms.max(1).into()),
                Duration::from_millis(recovery_ms.into()),
                SWITCH_STEP,
                SWITCH_STEP,
                SWITCH_STEP,
                SLOT_SETTLE,
            ],
            cycle_cooldown: Duration::from_millis(cooldown_ms.into()),
            next: None,
            cooldown_until: None,
        })
    }

    pub fn active(&self) -> bool {
        self.next.is_some() || self.cooldown_until.is_some()
    }

    pub fn update(&mut self, held: bool, now: Instant) -> io::Result<()> {
        if self.cooldown_until.is_some_and(|until| now >= until) {
            self.cooldown_until = None;
        }
        if self.next.is_none() {
            if !held || self.cooldown_until.is_some() {
                return Ok(());
            }
            self.next = Some((0, now));
        }
        let (index, due) = self.next.expect("active switch step");
        if now >= due {
            send(&self.events[index..=index])?;
            if index + 1 == self.events.len() {
                self.next = None;
                self.cooldown_until = Some(Instant::now() + self.cycle_cooldown);
            } else {
                self.next = Some((index + 1, Instant::now() + self.delays[index]));
            }
        }
        Ok(())
    }

    pub fn cancel(&mut self) -> io::Result<()> {
        if let Some((index @ (1 | 3 | 5), _)) = self.next {
            send(&self.events[index..=index])?;
        }
        if self.next.take().is_some() {
            self.cooldown_until = None;
        }
        Ok(())
    }
}

impl Drop for WhipSwitch {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

fn attack(up: bool) -> Input {
    mouse_button(1, up).expect("left mouse button")
}
