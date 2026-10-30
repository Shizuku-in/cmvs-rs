//! Voice-triggered BGM gain state machine (0x140087df0), independent of devices.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceDuckingSettings {
    pub percent: u32,
    pub fade_ms: u32,
    pub wait_ms: u32,
    /// Count fading-out voices too; this is not a master enable switch.
    pub include_fading: bool,
}

impl Default for VoiceDuckingSettings {
    fn default() -> Self {
        Self {
            percent: 128,
            fade_ms: 500,
            wait_ms: 1000,
            include_fading: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VoiceActivity {
    pub playing: bool,
    pub fading_out: bool,
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Idle,
    Lowering,
    Lowered,
    Waiting,
    Restoring,
}

#[derive(Debug)]
pub struct VoiceDucking {
    active: VoiceDuckingSettings,
    pending: Option<VoiceDuckingSettings>,
    phase: Phase,
    started: u32,
    from: u32,
    gain: u32,
}

impl Default for VoiceDucking {
    fn default() -> Self {
        Self {
            active: VoiceDuckingSettings::default(),
            pending: None,
            phase: Phase::Idle,
            started: 0,
            from: 256,
            gain: 256,
        }
    }
}

impl VoiceDucking {
    /// 0x140087d60: pending values replace earlier pending values, not active ones.
    pub fn configure(&mut self, mut settings: VoiceDuckingSettings) {
        settings.percent = settings.percent.min(100);
        self.pending = Some(settings);
    }

    pub fn gain(&self) -> u32 {
        self.gain
    }

    /// One mixer iteration, with a wrapping millisecond clock. Like the original,
    /// transitions do not consume leftover time in another state within this tick.
    pub fn update(&mut self, now: u32, voices: &[VoiceActivity]) -> u32 {
        // Detection precedes committing pending parameters in the idle branch.
        let voice = voices
            .iter()
            .any(|v| v.playing && (self.active.include_fading || !v.fading_out));
        let mut gain = self.active.percent.wrapping_shl(8) / 100;
        let elapsed = now.wrapping_sub(self.started);
        match self.phase {
            Phase::Idle => {
                if let Some(settings) = self.pending.take() {
                    self.active = settings;
                }
                gain = 256;
                if self.active.percent < 100 && voice {
                    self.phase = Phase::Lowering;
                    self.started = now;
                    self.from = 256;
                }
            }
            Phase::Lowering => {
                if elapsed < self.active.fade_ms {
                    gain = self.from.wrapping_sub(
                        elapsed.wrapping_mul(self.from.wrapping_sub(gain)) / self.active.fade_ms,
                    );
                } else {
                    self.phase = Phase::Lowered;
                }
            }
            Phase::Lowered => {
                if !voice {
                    self.phase = Phase::Waiting;
                    self.started = now;
                }
            }
            Phase::Waiting => {
                if elapsed >= self.active.wait_ms {
                    self.phase = Phase::Restoring;
                    self.from = gain;
                    self.started = now;
                }
                if voice {
                    self.phase = Phase::Lowered;
                }
            }
            Phase::Restoring => {
                gain = 256;
                if elapsed >= self.active.fade_ms {
                    self.phase = Phase::Idle;
                } else {
                    gain = self.from.wrapping_add(
                        256u32.wrapping_sub(self.from).wrapping_mul(elapsed) / self.active.fade_ms,
                    );
                }
                if voice {
                    self.phase = Phase::Lowering;
                    self.from = gain;
                    self.started = now;
                }
            }
        }
        self.gain = gain;
        gain
    }
}
