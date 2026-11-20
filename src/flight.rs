use crate::windows::{key, mouse_button, scan, send, Input};
use std::io;
use std::time::{Duration, Instant};
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};

const SLOT_HOLD: Duration = Duration::from_millis(10);
const JUMP_HOLD: Duration = Duration::from_millis(20);
const SPACE_TAKEOVER: Duration = Duration::from_millis(300);

pub const DEFAULT_JET_DELAY_MS: u32 = 100;
pub const DEFAULT_SIDE_MS: u32 = 1000;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideSegment {
    pub after_ms: u32,
    pub duration_ms: u32,
}

pub fn default_side_segments() -> Vec<SideSegment> {
    vec![
        SideSegment {
            after_ms: 1000,
            duration_ms: 200,
        },
        SideSegment {
            after_ms: 2000,
            duration_ms: 300,
        },
    ]
}

pub const fn default_equip_delay_ms() -> u32 {
    120
}

pub struct QuickFlight {
    slot: [Input; 2],
    jump: [Input; 2],
    jet: [Input; 2],
    equip_delay: Duration,
    jet_delay: Duration,
    stage: Option<Stage>,
    manual_left: bool,
    wait_for_space_release: bool,
    space_since: Option<Instant>,
    space_control: bool,
}

enum Stage {
    Step(Step, Instant),
    HoldingJet,
}

enum Step {
    SelectSlot,
    ReleaseSlot,
    PressJet,
    Jump,
    ReleaseJump,
}

impl QuickFlight {
    pub fn new(slot: u32, equip_delay_ms: u32, delay_ms: u32) -> io::Result<Self> {
        let slot = scan(slot)?;
        let space = scan(0x20)?;
        let left_down = mouse_button(1, false).expect("left mouse button");
        let left_up = mouse_button(1, true).expect("left mouse button");
        Ok(Self {
            slot: [key(slot, false), key(slot, true)],
            jump: [key(space, false), key(space, true)],
            jet: [left_down, left_up],
            equip_delay: Duration::from_millis(equip_delay_ms.into()),
            jet_delay: Duration::from_millis(delay_ms.into()),
            stage: None,
            manual_left: false,
            wait_for_space_release: false,
            space_since: None,
            space_control: false,
        })
    }

    pub fn active(&self) -> bool {
        self.stage.is_some()
    }

    fn jet_started(&self) -> bool {
        matches!(
            self.stage,
            Some(Stage::HoldingJet | Stage::Step(Step::Jump | Step::ReleaseJump, _))
        )
    }

    pub fn update(
        &mut self,
        enabled: bool,
        trigger: bool,
        space_down: bool,
        left_pressed: bool,
        left_down: bool,
        now: Instant,
    ) -> io::Result<()> {
        if space_down && matches!(self.stage, Some(Stage::Step(Step::ReleaseJump, _))) {
            self.stage = Some(Stage::HoldingJet);
        }
        if left_pressed && self.jet_started() {
            // The real mouse now owns the held button and will send its release.
            self.manual_left = true;
        } else if left_pressed && matches!(self.stage, Some(Stage::Step(_, _))) {
            return self.cancel();
        }
        if !enabled || self.manual_left && !left_down {
            return self.cancel();
        }
        if trigger && !self.active() {
            self.wait_for_space_release = space_down;
            self.space_since = None;
            self.stage = Some(Stage::Step(Step::SelectSlot, now));
        }
        if !self.active() {
            return Ok(());
        }

        if !space_down && !self.manual_left {
            if self.space_control {
                return self.cancel();
            }
            self.wait_for_space_release = false;
            self.space_since = None;
        } else if space_down && !self.manual_left && !self.wait_for_space_release {
            let since = *self.space_since.get_or_insert(now);
            if now.duration_since(since) >= SPACE_TAKEOVER {
                self.space_control = true;
            }
        }

        if let Some(Stage::Step(ref step, due)) = self.stage {
            if now >= due {
                if matches!(step, Step::Jump) && space_down {
                    self.stage = Some(Stage::HoldingJet);
                    return Ok(());
                }
                let (event, following) = match step {
                    Step::SelectSlot => (&self.slot[0], Some((Step::ReleaseSlot, SLOT_HOLD))),
                    Step::ReleaseSlot => (&self.slot[1], Some((Step::PressJet, self.equip_delay))),
                    Step::PressJet => (&self.jet[0], Some((Step::Jump, self.jet_delay))),
                    Step::Jump => (&self.jump[0], Some((Step::ReleaseJump, JUMP_HOLD))),
                    Step::ReleaseJump => (&self.jump[1], None),
                };
                send(std::slice::from_ref(event))?;
                self.stage = Some(match following {
                    Some((step, delay)) => Stage::Step(step, Instant::now() + delay),
                    None => Stage::HoldingJet,
                });
            }
        }
        Ok(())
    }

    fn cancel(&mut self) -> io::Result<()> {
        let key_release = match self.stage {
            Some(Stage::Step(Step::ReleaseSlot, _)) => send(&self.slot[1..]),
            Some(Stage::Step(Step::ReleaseJump, _)) => send(&self.jump[1..]),
            _ => Ok(()),
        };
        if self.jet_started() && !self.manual_left {
            key_release.and(send(&self.jet[1..]))?;
        } else {
            key_release?;
        }
        self.stage = None;
        self.manual_left = false;
        self.space_since = None;
        self.space_control = false;
        Ok(())
    }
}

impl Drop for QuickFlight {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

pub struct SideFlight {
    keys: [u16; 2],
    random_max_ms: u32,
    held: Option<(usize, Instant)>,
}

impl SideFlight {
    pub fn new(random_max_ms: u32) -> io::Result<Self> {
        Ok(Self {
            keys: [scan(u32::from(b'A'))?, scan(u32::from(b'D'))?],
            random_max_ms,
            held: None,
        })
    }

    pub fn update(&mut self, down: bool, interval_ms: u32, now: Instant) -> io::Result<()> {
        if !down {
            return self.release();
        }
        let next = match self.held {
            Some((_, until)) if now < until => return Ok(()),
            Some((direction, _)) => {
                self.release()?;
                1 - direction
            }
            None => 0,
        };
        let extra = random_extra(self.random_max_ms)?;
        let duration = Duration::from_millis(u64::from(interval_ms.max(1)) + u64::from(extra));
        send(&[key(self.keys[next], false)])?;
        self.held = Some((next, Instant::now() + duration));
        Ok(())
    }

    fn release(&mut self) -> io::Result<()> {
        if let Some((direction, _)) = self.held {
            send(&[key(self.keys[direction], true)])?;
            self.held = None;
        }
        Ok(())
    }
}

impl Drop for SideFlight {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

fn random_extra(max: u32) -> io::Result<u32> {
    if max == 0 {
        return Ok(0);
    }
    let span = u64::from(max) + 1;
    let limit = (1u64 << 32) / span * span;
    loop {
        let mut bytes = [0; 4];
        let result = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                4,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if result < 0 {
            return Err(io::Error::other(format!(
                "无法获取随机延迟（NTSTATUS 0x{result:08X}）"
            )));
        }
        let sample = u64::from(u32::from_ne_bytes(bytes));
        if sample < limit {
            return Ok((sample % span) as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_jitter_draws_fresh_values_within_the_configured_range() {
        assert_eq!(random_extra(0).unwrap(), 0);
        let samples: Vec<_> = (0..64).map(|_| random_extra(30).unwrap()).collect();
        assert!(samples.iter().all(|&value| value <= 30));
        assert!(samples.iter().any(|value| *value != samples[0]));
        assert!(random_extra(u32::MAX).is_ok());
    }
}
