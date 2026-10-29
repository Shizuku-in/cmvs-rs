//! Mode 1 / 0x1400541a0: region-gated left-button drag and release measurements.
use super::{HELD, LEFT, PRESSED, PointerGesture, RELEASED};
use crate::input::ActionInput;

impl PointerGesture {
    pub(super) fn frame_drag(&mut self, input: &mut ActionInput, now: u32) {
        self.set(80, 0);
        self.set(88, -1);
        if [0, 8, 12, 16, 20].into_iter().any(|at| self.get(at) == 0) {
            self.idle();
            self.set(172, -1);
            return;
        }
        let [x, y] = input.pointer();
        if self.get(48) == 0 {
            if !input.mouse(LEFT)[PRESSED] {
                return;
            }
            for (at, value) in [(56, x), (60, y), (100, x), (104, y)] {
                self.set(at, value);
            }
            let mut hit = None;
            for slot in 0..6 {
                let base = 188 + slot * 20;
                let [left, top, w, h] = [4, 8, 12, 16].map(|d| self.get(base + d));
                if self.get(base) != 0
                    && x >= left
                    && y >= top
                    && x < left.wrapping_add(w)
                    && y < top.wrapping_add(h)
                {
                    hit = Some(slot);
                }
            }
            if let Some(slot) = hit {
                self.set(48, 1);
                self.set(44, -1);
                self.set(172, -1);
                self.set(176, slot as i32);
            }
            return;
        }
        if self.get(120) != 0 {
            self.set(124, x.wrapping_sub(self.get(100)));
            self.set(128, y.wrapping_sub(self.get(104)));
        } else {
            let difference = if self.get(116) != 0 {
                y.wrapping_sub(self.get(104))
            } else {
                x.wrapping_sub(self.get(100))
            };
            if difference.wrapping_abs() >= 4 {
                for (at, value) in [
                    (132, x),
                    (136, y),
                    (140, x),
                    (144, y),
                    (148, x),
                    (152, y),
                    (156, now as i32),
                    (160, now as i32),
                    (100, x),
                    (104, y),
                ] {
                    self.set(at, value);
                }
                self.set(172, -1);
                self.set(120, 1);
            }
            self.set(124, 0);
            self.set(128, 0);
        }
        if self.get(120) != 0 {
            let near = x.wrapping_sub(self.get(140)).wrapping_abs() < 4
                && y.wrapping_sub(self.get(144)).wrapping_abs() < 4;
            if !near || now.wrapping_sub(self.get(160) as u32) >= 100 {
                if near {
                    self.set(132, x);
                    self.set(136, y);
                    self.set(156, now as i32);
                }
                self.set(140, x);
                self.set(144, y);
                self.set(160, now as i32);
            }
        }
        let flags = input.mouse(LEFT);
        if !flags[RELEASED] && flags[HELD] {
            return;
        }
        if self.get(120) != 0 {
            let dx = x.wrapping_sub(self.get(132));
            let dy = y.wrapping_sub(self.get(136));
            let (distance, axis) = if dx.wrapping_abs() < dy.wrapping_abs() {
                (dy, 1)
            } else {
                (dx, 0)
            };
            self.set(164, now.wrapping_sub(self.get(156) as u32) as i32);
            self.set(168, distance);
            self.set(
                172,
                if distance.wrapping_abs() < 4 {
                    -1
                } else {
                    axis
                },
            );
        }
        if !flags[RELEASED] {
            self.set(44, -1);
        }
        self.set(48, 0);
        if self.get(120) != 0 {
            input.clear_all_edges();
        }
        self.set(120, 0);
    }
}
