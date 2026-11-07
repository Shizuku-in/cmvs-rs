//! Probe group IDs 0..=5 are ordinary groups; 6..=13 are list groups 0..=7.
//! List hit rectangles follow the bound object, then intersect the screen-space hit area.
use cmvs_core::{
    buttons::{BUTTON_GROUPS, Button, ButtonGroup},
    runner::Startup,
};

pub const GROUPS: usize = BUTTON_GROUPS + 8;

#[derive(Clone, Copy)]
pub struct Group<'a> {
    pub buttons: &'a [Button],
    pub current: Option<usize>,
    offset: [i32; 2],
    clip: Option<[i32; 4]>,
    /// Ordinary groups map script points to device pointer positions (0x221b scale).
    pointer: Option<&'a ButtonGroup>,
}

impl<'a> Group<'a> {
    pub fn rect(&self, index: usize) -> Option<[i32; 4]> {
        let button = self.buttons.get(index)?;
        let [x, y, w, h] = button.rect;
        if w <= 0 || h <= 0 {
            return None;
        }
        let x = i64::from(x.wrapping_add(self.offset[0]));
        let y = i64::from(y.wrapping_add(self.offset[1]));
        let (mut left, mut top, mut right, mut bottom) = (x, y, x + i64::from(w), y + i64::from(h));
        if let Some([cx, cy, cw, ch]) = self.clip {
            if cw <= 0 || ch <= 0 {
                return None;
            }
            left = left.max(i64::from(cx));
            top = top.max(i64::from(cy));
            right = right.min(i64::from(cx) + i64::from(cw));
            bottom = bottom.min(i64::from(cy) + i64::from(ch));
        }
        if right <= left || bottom <= top {
            return None;
        }
        Some([
            i32::try_from(left).ok()?,
            i32::try_from(top).ok()?,
            i32::try_from(right - left).ok()?,
            i32::try_from(bottom - top).ok()?,
        ])
    }
    pub fn enabled(&self, index: usize) -> bool {
        self.buttons.get(index).is_some_and(|button| button.enabled) && self.rect(index).is_some()
    }
    /// The device pointer position at the centre of a button's rectangle.
    pub fn point(&self, index: usize) -> Option<[i32; 2]> {
        let [x, y, w, h] = self.rect(index)?;
        let centre = [x.checked_add(w / 2)?, y.checked_add(h / 2)?];
        Some(
            self.pointer
                .map_or(centre, |group| group.to_pointer(centre)),
        )
    }
    pub fn scroll_point(&self) -> Option<[i32; 2]> {
        let [x, y, w, h] = self.clip?;
        if w <= 0 || h <= 0 {
            return None;
        }
        Some([x.checked_add(w / 2)?, y.checked_add(h / 2)?])
    }
}

pub fn group<'a>(run: &'a Startup<'_>, slot: usize) -> Option<Group<'a>> {
    if slot < BUTTON_GROUPS {
        let group = run.buttons().group(slot)?;
        return Some(Group {
            buttons: &group.buttons,
            current: group.current,
            offset: [0, 0],
            clip: None,
            pointer: Some(group),
        });
    }
    let group = run.buttons().list_group(slot.checked_sub(BUTTON_GROUPS)?)?;
    let object = run.scene().object(group.object)?;
    if !object.shown {
        return None;
    }
    let draw = object.draw.as_ref()?;
    let offset = match draw.words[0] {
        0 => [draw.words[8], draw.words[9]].map(|v| v as i32),
        5 => {
            let values = [draw.words[11], draw.words[12]].map(f32::from_bits);
            if values
                .iter()
                .any(|v| !(-2147483648.0..2147483648.0).contains(v))
            {
                return None;
            }
            values.map(|v| v as i32)
        }
        _ => return None,
    };
    Some(Group {
        buttons: &group.buttons,
        current: group.current,
        offset,
        clip: Some(group.hit_area),
        pointer: None,
    })
}

pub fn enabled(run: &Startup<'_>) -> Vec<(usize, usize)> {
    (0..GROUPS)
        .filter_map(|slot| group(run, slot).map(|group| (slot, group)))
        .flat_map(|(slot, group)| {
            (0..group.buttons.len())
                .filter(move |&index| group.enabled(index))
                .map(move |index| (slot, index))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::buttons::{BUTTON_STATES, ButtonImage};

    #[test]
    fn scaled_group_points_are_device_pointer_positions() {
        let group = |pointer_scale| ButtonGroup {
            object: 0,
            buttons: vec![Button {
                id: 1,
                enabled: true,
                rect: [640, 480, 100, 100],
                links: [-1; 4],
                images: [ButtonImage::default(); BUTTON_STATES],
            }],
            current: None,
            pressed_visual: false,
            press_target: None,
            pointer_scale,
        };
        let probe = |group: &ButtonGroup| {
            Group {
                buttons: &group.buttons,
                current: None,
                offset: [0, 0],
                clip: None,
                pointer: Some(group),
            }
            .point(0)
        };
        // 800x600 device over 1280x960 script space: centre (690, 530) is (431, 331).
        assert_eq!(probe(&group(Some([800, 600, 1280, 960]))), Some([431, 331]));
        assert_eq!(probe(&group(None)), Some([690, 530]));
    }
}
