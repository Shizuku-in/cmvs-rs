//! Cooperative VM slices share one input sample and one frame completion.
use super::{Runtime, RuntimeEvent};
use crate::input::ActionInput;
use std::{
    error::Error,
    time::{Duration, Instant},
};

/// A host slice limit, retained across device-service retries within that slice.
pub struct FrameBudget {
    remaining: usize,
    deadline: Option<Instant>,
}
impl FrameBudget {
    pub fn steps(steps: usize) -> Self {
        Self {
            remaining: steps,
            deadline: None,
        }
    }
    pub fn time(duration: Duration) -> Self {
        Self {
            remaining: usize::MAX,
            deadline: Some(Instant::now() + duration),
        }
    }
    fn take(&mut self) -> bool {
        if self.remaining == 0 || self.deadline.is_some_and(|end| Instant::now() >= end) {
            return false;
        }
        self.remaining -= 1;
        true
    }
}

/// A yield still needs [`FrameDriver::finish_frame`] before scene composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    BudgetExhausted,
    Yield,
    Inactive,
    Stopped(RuntimeEvent),
}

/// Window-independent frame sequencing. Hosts retain clocks and device services.
#[derive(Default)]
pub struct FrameDriver {
    sampled: bool,
    yielded: bool,
    ready: bool,
    delta: Option<u32>,
}
impl FrameDriver {
    pub fn needs_input(&self) -> bool {
        !self.sampled && !self.yielded
    }
    /// Only completed live state may be composed again, including on resize.
    pub fn ready(&self) -> bool {
        self.ready
    }
    pub fn sample_input(&mut self, input: &mut ActionInput, sample: impl FnOnce(&mut ActionInput)) {
        if self.needs_input() {
            sample(input);
            self.sampled = true;
        }
    }
    /// Save an embedding host's frame delta once, across all budget slices.
    /// Hosts measuring elapsed time at yield can instead pass it to `finish_frame`.
    pub fn latch_delta(&mut self, delta_ms: u32) {
        self.delta.get_or_insert(delta_ms);
    }
    /// Discard paused time and allow fresh input after a focus/lifecycle transition.
    /// This does not make partially executed scene state safe to draw.
    pub fn suspend(&mut self) {
        self.sampled = false;
        self.delta = None;
    }
    /// Run until yield, inactivity, a stop (including [`RuntimeEvent::DeviceFramePending`]),
    /// or the host budget. The step hook may service devices and diagnostics; both
    /// `Stopped(RuntimeEvent::DeviceFramePending)` and errors retain the current input and
    /// delta so an asynchronous device request can resume the pending instruction.
    pub fn run_slice<E>(
        &mut self,
        runtime: &mut Runtime,
        budget: &mut FrameBudget,
        mut step: impl FnMut(&mut Runtime) -> Result<RuntimeEvent, E>,
    ) -> Result<FrameOutcome, E> {
        if self.yielded {
            return Ok(FrameOutcome::Yield);
        }
        while budget.take() {
            let was_ready = self.ready;
            self.ready = false;
            match step(runtime)? {
                RuntimeEvent::Progress | RuntimeEvent::SnapshotCaptured => {}
                RuntimeEvent::Yield => {
                    self.yielded = true;
                    return Ok(FrameOutcome::Yield);
                }
                RuntimeEvent::Inactive => {
                    self.ready = was_ready;
                    self.suspend();
                    return Ok(FrameOutcome::Inactive);
                }
                event => return Ok(FrameOutcome::Stopped(event)),
            }
        }
        Ok(FrameOutcome::BudgetExhausted)
    }
    /// Advance playback exactly once after yield. A latched delta takes precedence
    /// over the host's elapsed time. Returns whether a new frame was completed.
    pub fn finish_frame(
        &mut self,
        runtime: &mut Runtime,
        delta_ms: u32,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        if !self.yielded {
            return Ok(false);
        }
        // A failed update must not be retried against partially advanced state.
        self.yielded = false;
        runtime.advance_playback(self.delta.take().unwrap_or(delta_ms))?;
        self.sampled = false;
        self.ready = true;
        Ok(true)
    }
    /// A modal host can discard a yielded frame while keeping its previous canvas.
    pub fn skip_frame(&mut self) {
        self.yielded = false;
        self.ready = false;
        self.suspend();
    }
}
