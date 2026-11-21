//! Runtime group IDs 0..=5 are ordinary groups; 6..=13 are list groups 0..=7.
//! List hit rectangles follow the bound object, then intersect the screen-space hit area.
use super::{BUTTON_GROUPS, Button, ButtonGroup, Runtime};

pub const GROUPS: usize = BUTTON_GROUPS + 8;

#[derive(Clone)]
pub enum GroupButtons<'a> {
    Cmvs(&'a [Button]),
    Rects(Vec<[i32; 4]>),
}

#[derive(Clone)]
pub struct Group<'a> {
    buttons: GroupButtons<'a>,
    pub current: Option<usize>,
    offset: [i32; 2],
    clip: Option<[i32; 4]>,
    /// Ordinary groups map script points to device pointer positions (0x221b scale).
    pointer: Option<&'a ButtonGroup>,
}

impl<'a> Group<'a> {
    pub fn len(&self) -> usize {
        match &self.buttons {
            GroupButtons::Cmvs(b) => b.len(),
            GroupButtons::Rects(r) => r.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn raw_rect(&self, index: usize) -> Option<[i32; 4]> {
        match &self.buttons {
            GroupButtons::Cmvs(b) => b.get(index).map(|button| button.rect),
            GroupButtons::Rects(r) => r.get(index).copied(),
        }
    }
    pub fn rect(&self, index: usize) -> Option<[i32; 4]> {
        let [x, y, w, h] = self.raw_rect(index)?;
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
        let flag = match &self.buttons {
            GroupButtons::Cmvs(b) => b.get(index).is_some_and(|button| button.enabled),
            GroupButtons::Rects(r) => index < r.len(),
        };
        flag && self.rect(index).is_some()
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
    /// Hit bounds in device pointer coordinates, after list clipping and script scaling.
    pub fn pointer_rect(&self, index: usize) -> Option<[i32; 4]> {
        let [x, y, w, h] = self.rect(index)?;
        let Some(group) = self.pointer else {
            return Some([x, y, w, h]);
        };
        let [left, top] = group.to_pointer([x, y]);
        let [right, bottom] = group.to_pointer([x.checked_add(w)?, y.checked_add(h)?]);
        Some([
            left,
            top,
            right.checked_sub(left)?,
            bottom.checked_sub(top)?,
        ])
    }
    pub fn scroll_point(&self) -> Option<[i32; 2]> {
        let [x, y, w, h] = self.clip?;
        if w <= 0 || h <= 0 {
            return None;
        }
        Some([x.checked_add(w / 2)?, y.checked_add(h / 2)?])
    }
}

pub fn group<'a>(run: &'a Runtime, slot: usize) -> Option<Group<'a>> {
    if slot < BUTTON_GROUPS {
        if let Some(group) = run.buttons().group(slot) {
            return Some(Group {
                buttons: GroupButtons::Cmvs(&group.buttons),
                current: group.current,
                offset: [0, 0],
                clip: None,
                pointer: Some(group),
            });
        }
        if run.scene().cvns().is_some() {
            if slot == 0 {
                let rects = run.input().hit_rects();
                if !rects.is_empty() {
                    return Some(Group {
                        buttons: GroupButtons::Rects(rects.to_vec()),
                        current: None,
                        offset: [0, 0],
                        clip: None,
                        pointer: None,
                    });
                }
            } else if slot == 1
                && let Some(cvns) = run.messages().cvns()
            {
                let rects: Vec<[i32; 4]> = cvns
                    .messages
                    .iter()
                    .flatten()
                    .filter(|m| m.visible)
                    .flat_map(|m| m.buttons.iter())
                    .filter(|b| b.enabled && b.hit[2] > 0 && b.hit[3] > 0)
                    .map(|b| b.hit)
                    .collect();
                if !rects.is_empty() {
                    return Some(Group {
                        buttons: GroupButtons::Rects(rects),
                        current: None,
                        offset: [0, 0],
                        clip: None,
                        pointer: None,
                    });
                }
            }
        }
        return None;
    }
    let group = run.buttons().list_group(slot.checked_sub(BUTTON_GROUPS)?)?;
    let object = run.scene().object(group.object)?;
    if !object.shown {
        return None;
    }
    let draw = object.draw.as_ref()?;
    let offset = draw.screen_offset()?;
    Some(Group {
        buttons: GroupButtons::Cmvs(&group.buttons),
        current: group.current,
        offset,
        clip: Some(group.hit_area),
        pointer: None,
    })
}

pub fn enabled(run: &Runtime) -> Vec<(usize, usize)> {
    (0..GROUPS)
        .filter_map(|slot| group(run, slot).map(|group| (slot, group)))
        .flat_map(|(slot, group)| {
            (0..group.len())
                .filter(move |&index| group.enabled(index))
                .map(move |index| (slot, index))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{BUTTON_STATES, ButtonImage};

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
                buttons: GroupButtons::Cmvs(&group.buttons),
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
