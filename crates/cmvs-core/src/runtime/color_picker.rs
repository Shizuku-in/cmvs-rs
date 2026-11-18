//! CVNS color dialog drawn in the logical game canvas.
use super::Runtime;
use crate::{
    FormatError,
    dispatch::{RuntimeCall, store_color_result},
    input::ActionInput,
    raster::RgbaImage,
    scene::ObjectImage,
};
use std::error::Error;

const WIDTH: usize = 400;
const HEIGHT: usize = 234;
const TRACK_LEFT: usize = 48;
const TRACK_WIDTH: usize = 256;
const INK: [u8; 3] = [238, 242, 249];
const MUTED: [u8; 3] = [163, 178, 199];
const FOCUS: [u8; 3] = [127, 187, 255];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Red,
    Green,
    Blue,
    First,
    Second,
    Cancel,
    Accept,
}

impl Control {
    const ALL: [Self; 7] = [
        Self::Red,
        Self::Green,
        Self::Blue,
        Self::First,
        Self::Second,
        Self::Cancel,
        Self::Accept,
    ];

    fn rect(self) -> [usize; 4] {
        match self {
            Self::Red => [TRACK_LEFT, 65, TRACK_WIDTH, 23],
            Self::Green => [TRACK_LEFT, 103, TRACK_WIDTH, 23],
            Self::Blue => [TRACK_LEFT, 141, TRACK_WIDTH, 23],
            Self::First => [18, 184, 39, 32],
            Self::Second => [65, 184, 39, 32],
            Self::Cancel => [204, 184, 86, 32],
            Self::Accept => [302, 184, 80, 32],
        }
    }

    fn at([x, y]: [usize; 2]) -> Option<Self> {
        Self::ALL.into_iter().find(|&control| {
            let [left, top, w, h] = control.rect();
            (left..left + w).contains(&x) && (top..top + h).contains(&y)
        })
    }

    fn channel(self) -> Option<usize> {
        (self as usize <= Self::Blue as usize).then_some(self as usize)
    }

    fn next(self, direction: isize) -> Self {
        Self::ALL[(self as isize + direction).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

enum Answer {
    Open,
    Closed(Option<u32>),
}

pub(super) struct ColorPicker {
    request: RuntimeCall,
    swatches: [[u8; 3]; 2],
    color: [u8; 3],
    focus: Control,
    pressed: Option<Control>,
    panel: [i64; 4],
}

impl ColorPicker {
    fn new(request: RuntimeCall, swatches: [u32; 2], screen: [u32; 2]) -> Self {
        let [sw, sh] = screen.map(i64::from);
        let mut w = sw.min(WIDTH as i64);
        let mut h = sh.min(HEIGHT as i64);
        if w * (HEIGHT as i64) < h * (WIDTH as i64) {
            h = (w * HEIGHT as i64 / WIDTH as i64).max(1);
        } else {
            w = (h * WIDTH as i64 / HEIGHT as i64).max(1);
        }
        Self {
            request,
            swatches: swatches.map(crate::scene::cvns::unpack_rgb),
            color: [0; 3],
            focus: Control::Red,
            pressed: None,
            panel: [(sw - w) / 2, (sh - h) / 2, w, h],
        }
    }

    fn input(&mut self, input: &mut ActionInput) -> Answer {
        let [x, y, w, h] = self.panel;
        let [px, py] = input.pointer().map(i64::from);
        let local = [
            ((px - x) * WIDTH as i64 / w).clamp(0, WIDTH as i64 - 1) as usize,
            ((py - y) * HEIGHT as i64 / h).clamp(0, HEIGHT as i64 - 1) as usize,
        ];
        let over = ((x..x + w).contains(&px) && (y..y + h).contains(&py))
            .then(|| Control::at(local))
            .flatten();
        let [released, held, pressed] = input.mouse(0);
        if pressed {
            self.pressed = over;
            if let Some(control) = over {
                self.focus = control;
            }
        }
        if (held || released)
            && let Some(channel) = self.pressed.and_then(Control::channel)
            && px >= 0
        {
            self.color[channel] = local[0].saturating_sub(TRACK_LEFT).min(255) as u8;
        }
        if released {
            let clicked = self
                .pressed
                .take()
                .filter(|control| Some(*control) == over && control.channel().is_none());
            if let Some(control) = clicked {
                return self.activate(control);
            }
        } else if !held && !pressed {
            // Focus loss clears the buttons without a release edge.
            self.pressed = None;
        }

        if input.mouse(1)[0] || input.cvns_wait_interrupt(1 << 4, true) {
            return Answer::Closed(None);
        }
        let vertical = i8::from(input.cvns_wait_interrupt(1 << 9, true))
            - i8::from(input.cvns_wait_interrupt(1 << 8, true));
        if vertical != 0 {
            self.focus = self.focus.next(isize::from(vertical));
        }
        let horizontal = i8::from(input.cvns_wait_interrupt(1 << 11, true))
            - i8::from(input.cvns_wait_interrupt(1 << 10, true));
        if let Some(channel) = self.focus.channel() {
            let delta = horizontal + input.repeat_horizontal();
            self.color[channel] = self.color[channel].saturating_add_signed(delta);
        } else if horizontal != 0 {
            self.focus = self.focus.next(isize::from(horizontal));
        }
        if input.cvns_wait_interrupt((1 << 5) | (1 << 6), true) {
            return self.activate(self.focus);
        }
        Answer::Open
    }

    fn activate(&mut self, control: Control) -> Answer {
        match control {
            Control::Cancel => Answer::Closed(None),
            Control::First => {
                self.color = self.swatches[0];
                Answer::Open
            }
            Control::Second => {
                self.color = self.swatches[1];
                Answer::Open
            }
            _ => {
                let [r, g, b] = self.color;
                Answer::Closed(Some(u32::from_be_bytes([0, r, g, b])))
            }
        }
    }

    fn render(&self) -> Result<([i64; 4], ObjectImage), FormatError> {
        let mut panel = Panel::new();
        panel.fill([0, 0, WIDTH, 3], FOCUS);
        panel.text([18, 20], b"COLOR", INK);
        panel.fill([245, 15, 30, 24], self.color);
        panel.outline([245, 15, 30, 24], MUTED);
        let [r, g, b] = self.color;
        panel.text([290, 20], format!("#{r:02X}{g:02X}{b:02X}").as_bytes(), INK);
        panel.fill([18, 50, 364, 1], [63, 78, 100]);

        for (channel, control) in Control::ALL[..3].iter().copied().enumerate() {
            let [x, y, w, h] = control.rect();
            panel.text([22, y + 4], &b"RGB"[channel..=channel], INK);
            for value in 0..w {
                let mut color = self.color;
                color[channel] = value as u8;
                panel.fill([x + value, y, 1, h], color);
            }
            let mark = [x + usize::from(self.color[channel]) - 2, y - 2, 5, h + 4];
            panel.outline(mark, INK);
            panel.text(
                [326, y + 4],
                format!("{:03}", self.color[channel]).as_bytes(),
                MUTED,
            );
        }
        for (control, color) in [
            (Control::First, self.swatches[0]),
            (Control::Second, self.swatches[1]),
        ] {
            panel.fill(control.rect(), color);
            panel.outline(control.rect(), MUTED);
        }
        for (control, label, color, ink) in [
            (Control::Cancel, b"CANCEL".as_slice(), [41, 53, 72], INK),
            (
                Control::Accept,
                b"OK".as_slice(),
                [150, 199, 255],
                [17, 29, 55],
            ),
        ] {
            let [x, y, w, h] = control.rect();
            panel.fill([x, y, w, h], color);
            panel.text([x + (w - label.len() * 12 + 2) / 2, y + 9], label, ink);
        }
        let [x, y, w, h] = self.focus.rect();
        panel.outline([x - 3, y - 3, w + 6, h + 6], FOCUS);
        Ok((
            self.panel,
            ObjectImage {
                pixels: RgbaImage::new(WIDTH as u32, HEIGHT as u32, panel.0)?,
                filter: None,
                name: Vec::new(),
                alpha: false,
                surface: false,
            },
        ))
    }
}

impl Runtime {
    /// Enable the shared color modal for a host that presents and supplies input for each frame.
    pub fn enable_in_game_color_picker(&mut self) {
        self.in_game_color_picker = true;
    }

    pub(super) fn open_color_picker(
        &mut self,
        request: RuntimeCall,
        swatches: [u32; 2],
    ) -> Result<(), FormatError> {
        let picker = ColorPicker::new(request, swatches, self.screen_size());
        self.scene.color_picker_panel = Some(picker.render()?);
        self.color_picker = Some(picker);
        self.input.clear_all_edges();
        self.frame_delta = 0;
        Ok(())
    }

    pub(super) fn advance_color_picker(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let picker = self.color_picker.as_mut().expect("open color picker");
        let previous = (picker.color, picker.focus);
        match picker.input(&mut self.input) {
            Answer::Closed(answer) => {
                picker.request.complete(&mut self.machine, 0x4008)?;
                store_color_result(self.machine.state_mut(), answer);
                self.color_picker = None;
                self.scene.color_picker_panel = None;
                self.input.clear_all_edges();
                self.input.cvns_wait_interrupt(3, true);
                self.resume_run = true;
            }
            Answer::Open if previous != (picker.color, picker.focus) => {
                self.scene.color_picker_panel = Some(picker.render()?);
            }
            Answer::Open => {}
        }
        Ok(())
    }
}

struct Panel(Vec<u8>);
impl Panel {
    fn new() -> Self {
        Self([23, 32, 48, 255].repeat(WIDTH * HEIGHT))
    }

    fn fill(&mut self, [x, y, w, h]: [usize; 4], [r, g, b]: [u8; 3]) {
        for row in y..y + h {
            let start = (row * WIDTH + x) * 4;
            for pixel in self.0[start..start + w * 4].as_chunks_mut::<4>().0 {
                *pixel = [r, g, b, 255];
            }
        }
    }

    fn outline(&mut self, [x, y, w, h]: [usize; 4], color: [u8; 3]) {
        self.fill([x, y, w, 1], color);
        self.fill([x, y + h - 1, w, 1], color);
        self.fill([x, y, 1, h], color);
        self.fill([x + w - 1, y, 1, h], color);
    }

    fn text(&mut self, [x, y]: [usize; 2], bytes: &[u8], color: [u8; 3]) {
        for (index, &code) in bytes.iter().enumerate() {
            for (row, bits) in label_glyph(code).into_iter().enumerate() {
                for column in 0..5 {
                    if bits & (16 >> column) != 0 {
                        self.fill([x + index * 12 + column * 2, y + row * 2, 2, 2], color);
                    }
                }
            }
        }
    }
}

// The few monochrome glyphs needed for the picker are independent of game fonts and encodings.
fn label_glyph(code: u8) -> [u8; 7] {
    match code {
        b'#' => [10, 10, 31, 10, 31, 10, 10],
        b'0' => [14, 17, 19, 21, 25, 17, 14],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 16, 30, 1, 1, 30],
        b'6' => [14, 16, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 1, 14],
        b'A' => [14, 17, 17, 31, 17, 17, 17],
        b'B' => [30, 17, 17, 30, 17, 17, 30],
        b'C' => [14, 17, 16, 16, 16, 17, 14],
        b'D' => [30, 17, 17, 17, 17, 17, 30],
        b'E' => [31, 16, 16, 30, 16, 16, 31],
        b'F' => [31, 16, 16, 30, 16, 16, 16],
        b'G' => [14, 17, 16, 23, 17, 17, 15],
        b'K' => [17, 18, 20, 24, 20, 18, 17],
        b'L' => [16, 16, 16, 16, 16, 16, 31],
        b'N' => [17, 25, 21, 19, 17, 17, 17],
        b'O' => [14, 17, 17, 17, 17, 17, 14],
        b'R' => [30, 17, 17, 30, 20, 18, 17],
        _ => [0; 7],
    }
}
