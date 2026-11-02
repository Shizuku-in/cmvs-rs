//! Host input shared by mobile platforms, without window-system dependencies.
//! Hosts translate native touches to physical surface pixels and sample once per
//! engine frame. Gesture recognition and action bindings remain in cmvs-core.
use cmvs_core::input::{ActionInput, CodeInput, KeyBindings};
use std::collections::BTreeSet;

/// Shared fixed-point display geometry for a logical game canvas.
#[derive(Debug, Clone, Copy)]
pub struct Viewport(cmvs_core::viewport::Viewport);
impl Viewport {
    pub fn new(size: [u32; 2]) -> Option<Self> {
        Self::with_mode(size, 1)
    }
    pub fn with_mode(size: [u32; 2], mode: u32) -> Option<Self> {
        Self::for_canvas([1280, 720], size, mode)
    }
    pub fn for_canvas(canvas: [u32; 2], size: [u32; 2], mode: u32) -> Option<Self> {
        cmvs_core::viewport::Viewport::new(canvas, size, mode)
            .ok()
            .map(Self)
    }
    pub fn to_script(self, point: [f64; 2]) -> Option<[i32; 2]> {
        self.0.to_source(point)
    }
    pub fn to_window(self, point: [i32; 2]) -> [f64; 2] {
        self.0.to_target(point)
    }
}

/// Platform touch identity, stable until that contact ends or is cancelled.
pub type TouchId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// Coordinates use the same physical surface pixels as the presentation viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchEvent {
    pub id: TouchId,
    pub phase: TouchPhase,
    pub position: [f64; 2],
}

/// Single-primary-contact mouse adapter. Secondary contacts never take over;
/// a new gesture can start only after every contact from the old one has ended.
/// A contact starting outside the viewport is ignored for its entire gesture.
///
/// This adapter owns the supplied ActionInput, just like the desktop adapter.
/// Do not sample multiple adapters into the same ActionInput. Hosts must route
/// only game-area events here, cancel on resize/menu entry, and call focus(false)
/// on suspension. No platform navigation or synthetic long-press policy lives here.
#[derive(Debug, Default)]
pub struct TouchInput {
    contacts: BTreeSet<TouchId>,
    primary: Option<TouchId>,
    pointer: Option<[f64; 2]>,
    pulse: bool,
    focused: bool,
    history: CodeInput,
}

impl TouchInput {
    /// Initially unfocused. Losing focus cancels rather than generating a release.
    pub fn focus(&mut self, focused: bool, input: &mut ActionInput) {
        self.focused = focused;
        if !focused {
            self.cancel(input);
        }
    }

    /// Cancels all contacts and queued presses without producing a click edge.
    /// Focus is unchanged; subsequent new contacts may begin a gesture.
    pub fn cancel(&mut self, input: &mut ActionInput) {
        self.contacts.clear();
        self.primary = None;
        self.pointer = None;
        self.pulse = false;
        self.history.clear(input);
    }

    /// Use a viewport matching the current renderer for both events and sampling.
    /// A native cancellation of the primary contact cancels its pending click;
    /// other contacts remain blocked until their own end/cancellation arrives.
    pub fn event(&mut self, event: TouchEvent, viewport: Viewport, input: &mut ActionInput) {
        if !self.focused {
            return;
        }
        match event.phase {
            TouchPhase::Started => {
                let first = self.contacts.is_empty();
                if self.contacts.insert(event.id)
                    && first
                    && viewport.to_script(event.position).is_some()
                {
                    self.primary = Some(event.id);
                    self.pointer = Some(event.position);
                    self.pulse = true;
                }
            }
            TouchPhase::Moved => {
                if self.primary == Some(event.id) {
                    self.pointer = Some(event.position);
                }
            }
            TouchPhase::Ended => {
                self.contacts.remove(&event.id);
                if self.primary == Some(event.id) {
                    self.pointer = Some(event.position);
                    self.primary = None;
                }
            }
            TouchPhase::Cancelled => {
                self.contacts.remove(&event.id);
                if self.primary == Some(event.id) {
                    self.primary = None;
                    self.pointer = None;
                    self.pulse = false;
                    self.history.clear(input);
                }
            }
        }
    }

    /// Exactly once at the host's input boundary, never per VM budget slice.
    /// A complete short tap between samples produces a down frame then a release.
    /// Multiple taps between samples coalesce, as with desktop button pulses.
    /// Keep the final position through release for the core drag recognizer.
    pub fn frame(&mut self, bindings: &KeyBindings, viewport: Viewport, input: &mut ActionInput) {
        let point = self.pointer.and_then(|point| viewport.to_script(point));
        let mut codes = [false; 128];
        codes[1] = self.primary.is_some() || self.pulse;
        self.pulse = false;
        self.history.update(bindings, &codes, input);
        let [x, y] = point.unwrap_or([-1, -1]);
        input.set_pointer(x, y);
    }
}
