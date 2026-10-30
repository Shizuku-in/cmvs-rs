//! Shared display rectangle for software presentation and pointer conversion.
use crate::{FormatError, MAX_PAYLOAD};

/// Software desktop policy for the independent fullscreen scaling enable switch.
/// A disabled switch keeps script pixels unscaled when they fit the desktop;
/// smaller borderless desktops retain mode zero's existing contain fallback.
pub fn presentation_mode(fullscreen: bool, scaling: bool, aspect_mode: u32) -> u32 {
    if !fullscreen {
        1
    } else if scaling {
        aspect_mode
    } else {
        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub left: i64,
    pub top: i64,
    pub width: u32,
    pub height: u32,
    source: [u32; 2],
    target: [u32; 2],
}
impl Viewport {
    /// 0x14001a360 fullscreen branch: 13-bit scale, unsigned contain and signed
    /// cover centering. Mode zero keeps native size when it fits; invalid modes
    /// must be normalized by the command/backend before constructing a viewport.
    pub fn new(source: [u32; 2], target: [u32; 2], mode: u32) -> Result<Self, FormatError> {
        if source.contains(&0)
            || target.contains(&0)
            || mode > 3
            || u64::from(source[0]) * u64::from(source[1]) > MAX_PAYLOAD as u64 / 4
            || u64::from(target[0]) * u64::from(target[1]) > MAX_PAYLOAD as u64 / 4
        {
            return Err(FormatError("invalid display viewport"));
        }
        let [sw, sh] = source.map(u64::from);
        let [tw, th] = target.map(u64::from);
        let mode = if mode == 0 && (tw < sw || th < sh) {
            1
        } else {
            mode
        };
        let (w, h) = match mode {
            0 => (sw, sh),
            3 => (tw, th),
            _ => {
                let rx = (tw << 13) / sw;
                let ry = (th << 13) / sh;
                if rx == ry {
                    (tw, th)
                } else if (mode == 1 && rx < ry) || (mode == 2 && rx > ry) {
                    (tw, (rx * sh) >> 13)
                } else {
                    ((ry * sw) >> 13, th)
                }
            }
        };
        if w == 0 || h == 0 || w > i32::MAX as u64 || h > i32::MAX as u64 {
            return Err(FormatError("display viewport exceeds supported dimensions"));
        }
        Ok(Self {
            left: (tw as i64 - w as i64) / 2,
            top: (th as i64 - h as i64) / 2,
            width: w as u32,
            height: h as u32,
            source,
            target,
        })
    }
    pub fn to_source(self, [x, y]: [f64; 2]) -> Option<[i32; 2]> {
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= f64::from(self.target[0])
            || y >= f64::from(self.target[1])
        {
            return None;
        }
        let x = x - self.left as f64;
        let y = y - self.top as f64;
        if x < 0.0 || y < 0.0 || x >= f64::from(self.width) || y >= f64::from(self.height) {
            return None;
        }
        Some([
            (x * f64::from(self.source[0]) / f64::from(self.width)).floor() as i32,
            (y * f64::from(self.source[1]) / f64::from(self.height)).floor() as i32,
        ])
    }
    pub fn to_target(self, [x, y]: [i32; 2]) -> [f64; 2] {
        [
            self.left as f64
                + (f64::from(x) + 0.5) * f64::from(self.width) / f64::from(self.source[0]),
            self.top as f64
                + (f64::from(y) + 0.5) * f64::from(self.height) / f64::from(self.source[1]),
        ]
    }
}
