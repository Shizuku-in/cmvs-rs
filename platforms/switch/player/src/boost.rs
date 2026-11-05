//! When to request CPU boost (1785 MHz). The system throttles the GPU to its minimum
//! while boosted, so only VM time counts as load: rendering and presentation never keep
//! the boost on, and a GPU slowed by the boost cannot feed back into it.
use std::time::{Duration, Instant};

/// VM time without a frame that counts as loading.
const ENTER: Duration = Duration::from_millis(50);
/// A frame whose VM time stays within this is an ordinary frame.
const QUIET_FRAME: Duration = Duration::from_millis(16);
/// Ordinary frames for this long end the boost.
const HOLD: Duration = Duration::from_millis(500);

pub struct LoadBoost {
    boosted: bool,
    /// VM time since the last frame.
    busy: Duration,
    quiet_since: Option<Instant>,
}

impl LoadBoost {
    /// Starts boosted: everything before the first frames is loading.
    pub fn starting() -> Self {
        Self {
            boosted: true,
            busy: Duration::ZERO,
            quiet_since: None,
        }
    }
    /// The boost the host should apply now.
    pub fn wanted(&self, focused: bool) -> bool {
        self.boosted && focused
    }
    /// One VM tick took `elapsed`.
    pub fn ran(&mut self, elapsed: Duration) {
        self.busy += elapsed;
        if self.busy >= ENTER {
            self.boosted = true;
            self.quiet_since = None;
        }
    }
    /// The VM yielded a frame.
    pub fn frame(&mut self, now: Instant) {
        if self.busy > QUIET_FRAME {
            self.quiet_since = None;
        } else if self.boosted {
            let since = *self.quiet_since.get_or_insert(now);
            if now.duration_since(since) >= HOLD {
                self.boosted = false;
                self.quiet_since = None;
            }
        }
        self.busy = Duration::ZERO;
    }
    /// Time that is not VM work (idle sleep, a system applet) breaks the measurement.
    pub fn interrupted(&mut self) {
        self.busy = Duration::ZERO;
    }
}

/// Applies boost decisions; restores normal clocks on drop. A refused request (applet
/// mode, old firmware) disables further attempts.
#[derive(Default)]
pub struct CpuBoost {
    applied: bool,
    unavailable: bool,
}
impl CpuBoost {
    pub fn apply(&mut self, wanted: bool) {
        if wanted == self.applied || self.unavailable {
            return;
        }
        if crate::sys::set_cpu_boost(wanted) {
            self.applied = wanted;
        } else {
            self.unavailable = true;
        }
    }
}
impl Drop for CpuBoost {
    fn drop(&mut self) {
        if self.applied {
            crate::sys::set_cpu_boost(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn startup_boost_ends_after_quiet_frames_and_returns_for_long_vm_work() {
        let start = Instant::now();
        let mut boost = LoadBoost::starting();
        assert!(boost.wanted(true));
        assert!(!boost.wanted(false));
        for frame in 0..=30u32 {
            boost.ran(4 * MS);
            boost.frame(start + frame * 17 * MS);
        }
        assert!(!boost.wanted(true));

        // Several shorter ticks add up before any frame arrives.
        for _ in 0..6 {
            boost.ran(9 * MS);
        }
        assert!(boost.wanted(true));
        boost.frame(start + 600 * MS);
        boost.ran(4 * MS);
        boost.frame(start + 700 * MS);
        // A busy frame restarts the hold period.
        boost.ran(30 * MS);
        boost.frame(start + 1000 * MS);
        boost.ran(4 * MS);
        boost.frame(start + 1100 * MS);
        assert!(boost.wanted(true));
        boost.frame(start + 1600 * MS);
        assert!(!boost.wanted(true));
    }

    #[test]
    fn interrupted_time_is_not_load() {
        let mut boost = LoadBoost::starting();
        let start = Instant::now();
        boost.frame(start);
        boost.frame(start + HOLD);
        assert!(!boost.wanted(true));
        boost.ran(40 * MS);
        boost.interrupted();
        boost.ran(40 * MS);
        assert!(!boost.wanted(true));
    }
}
