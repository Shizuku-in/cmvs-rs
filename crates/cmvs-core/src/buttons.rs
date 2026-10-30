//! Engine +4504: six button groups bound to scene objects (0x2210..0x221b); addresses refer
//! to the documented binary hash. Covers groups, buttons, hit rectangles, navigation links,
//! per-state images, the initial display of 0x2215, the hover/press/selection logic of
//! 0x2216/0x2217 over `ActionInput`, 0x221a (make a button current) and 0x221b (a group with
//! an explicit pointer scale). 0x2218/0x2219 stay unhandled.
//! Auxiliary list groups at engine +4552 are handled in `list`.
use crate::{
    bytecode::DecodeError, evaluate::error, input::ActionInput, machine::Machine, scene::SceneHost,
    vm::SystemResult,
};

mod list;
pub use list::ListGroup;
pub const BUTTON_GROUPS: usize = 6;
pub const BUTTON_STATES: usize = 4;
/// Research limit; the original allocates list nodes without a bound.
const BUTTON_LIMIT: usize = 1024;

/// Node +48 + 16 * state: what 0x2215/0x2217 apply to a child of the group's object.
/// State 0 is normal, 1 hovered, 2 pressed and 3 disabled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ButtonImage {
    pub child: i32,
    /// Source x/y/width/height for the 0x2044 setter 0x14003b070.
    pub crop: [i16; 4],
    /// Origin x/y for the 0x2046 setter 0x14003b0a0.
    pub origin: [i16; 2],
}

/// A 0x70-byte list node from 0x14005f070.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub id: u32,
    /// Node +8, set at creation; 0x2215 shows image 3 instead of 0 when cleared, and the
    /// pointer hit test skips the button.
    pub enabled: bool,
    /// Node +16..+28 from 0x2213: x, y, width, height in script coordinates.
    pub rect: [i32; 4],
    /// Node +32..+44: previous and next ids in insertion order, then -1, -1.
    pub links: [i32; 4],
    pub images: [ButtonImage; BUTTON_STATES],
}

/// A 0x48-byte group from 0x14005ef80. Its six screen metrics come from two values copied
/// twice, so the pointer scaling in 0x14005f700 cancels out and is not modeled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonGroup {
    /// Scene slot whose object pointer the original stores at +48.
    pub object: u8,
    pub buttons: Vec<Button>,
    /// +16: hovered button (index into `buttons`).
    pub current: Option<usize>,
    /// +60: the pressed image (state 2) is shown.
    pub pressed_visual: bool,
    /// +64: button that was current when action 0 went down.
    pub press_target: Option<usize>,
}

/// 0x2217 results other than a selected id.
const DECIDED_ELSEWHERE: u32 = -10i32 as u32;
const CANCELLED: u32 = -2i32 as u32;
const NOTHING: u32 = u32::MAX;

#[derive(Debug, Default, Clone)]
pub struct ButtonHost {
    groups: [Option<ButtonGroup>; BUTTON_GROUPS],
    lists: [Option<ListGroup>; 8],
    /// dword_140101448 (zero-filled .data tail), set only by 0x2087.
    merged_states: bool,
}

/// Writes button images to children of `object` with the 0x2044/0x2046 setters and, for
/// 0x2215, sets +6244. The original dereferences a missing child or draw state, so every
/// target is checked before anything is written.
fn apply_images(
    scene: &mut SceneHost,
    object: u8,
    images: &[ButtonImage],
    show: bool,
) -> Result<(), &'static str> {
    let parent = scene
        .object(object)
        .ok_or("button group object no longer exists")?;
    for image in images {
        let child = u16::try_from(image.child)
            .ok()
            .and_then(|child| parent.children.get(&child))
            .ok_or("button image child does not exist")?;
        if child.draw.is_none() {
            return Err("button image child has no draw state");
        }
    }
    let parent = scene.object_mut(object).expect("checked");
    for image in images {
        let child = parent
            .children
            .get_mut(&(image.child as u16))
            .expect("checked");
        let words = &mut child.draw.as_mut().expect("checked").words;
        for (word, value) in words[2..8]
            .iter_mut()
            .zip(image.crop.iter().chain(&image.origin))
        {
            *word = i32::from(*value) as u32;
        }
        if show {
            child.shown = true;
        }
    }
    Ok(())
}

impl ButtonHost {
    pub fn group(&self, slot: usize) -> Option<&ButtonGroup> {
        self.groups.get(slot).and_then(Option::as_ref)
    }
    pub fn merged_states(&self) -> bool {
        self.merged_states
    }
    /// Handlers compare the slot as a signed int against 6; larger values only set +12056
    /// bit 0x100000 (unmodeled) and negative values index before the table.
    fn slot(raw: u32) -> Option<usize> {
        usize::try_from(raw as i32)
            .ok()
            .filter(|slot| *slot < BUTTON_GROUPS)
    }
    pub fn try_handle(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
    ) -> Result<bool, DecodeError> {
        if self.try_handle_list(vm, scene)? {
            return Ok(true);
        }
        let Some((pc, command @ (0x87 | 0x210..=0x215 | 0x21b))) = vm.pending_system() else {
            return Ok(false);
        };
        let op = command + 0x2000;
        let fail = |why| error(pc, op, why);
        let count = match command {
            0x87 | 0x211 | 0x215 => 1,
            0x210 | 0x212 => 2,
            0x21b => 4,
            0x213 => 6,
            _ => 10,
        };
        let mut args = Vec::with_capacity(count);
        for i in 1..=count {
            args.push(vm.stack().peek_u32(i * 4).map_err(|e| fail(e.0))?);
        }
        if command == 0x87 {
            vm.complete_system(SystemResult(0x4004))?;
            self.merged_states = args[0] != 0;
            return Ok(true);
        }
        let slot = Self::slot(args[0]).ok_or_else(|| fail("button group slot outside 0..5"))?;
        if command == 0x21b {
            // 0x1400794e0: as 0x2210, but the group's pointer scale (+40/+44) is SP-12 /
            // SP-16 over the device size (+24/+28) instead of the device size itself.
            if [args[2], args[3]] != crate::message::RESEARCH_SCREEN {
                return Err(fail("a button group with a pointer scale is not modeled"));
            }
        }
        if matches!(command, 0x210 | 0x21b) {
            // 0x140079370 stores the object pointer, which is null for an empty slot.
            let object = u8::try_from(args[1] as i32)
                .ok()
                .filter(|object| scene.object(*object).is_some())
                .ok_or_else(|| fail("button group needs an existing scene object"))?;
            vm.complete_system(SystemResult(if command == 0x210 { 0x4008 } else { 0x4010 }))?;
            self.groups[slot] = Some(ButtonGroup {
                object,
                buttons: Vec::new(),
                current: None,
                pressed_visual: false,
                press_target: None,
            });
            return Ok(true);
        }
        if command == 0x211 {
            vm.complete_system(SystemResult(0x4004))?;
            self.groups[slot] = None;
            return Ok(true);
        }
        let group = self.groups[slot]
            .as_mut()
            .ok_or_else(|| fail("button group slot is empty"))?;
        if command == 0x215 {
            // 0x14005f3c0: image 0 (image 3 when disabled) for every button, then +16 = null.
            let images = group
                .buttons
                .iter()
                .map(|button| button.images[if button.enabled { 0 } else { 3 }])
                .collect::<Vec<_>>();
            apply_images(scene, group.object, &images, true).map_err(fail)?;
            vm.complete_system(SystemResult(0x4004))
                .expect("arguments were checked");
            group.current = None;
            return Ok(true);
        }
        let id = args[1];
        if command == 0x212 && group.buttons.len() >= BUTTON_LIMIT {
            return Err(fail("button count exceeds research limit"));
        }
        vm.complete_system(SystemResult(0x4000 | (count * 4) as u16))?;
        let buttons = &mut group.buttons;
        if command == 0x212 {
            // 0x14005f070 appends and links the node after the previous tail.
            let previous = buttons.last_mut().map(|tail| {
                tail.links[1] = id as i32;
                tail.id as i32
            });
            buttons.push(Button {
                id,
                enabled: true,
                rect: [0; 4],
                links: [previous.unwrap_or(-1), -1, -1, -1],
                images: Default::default(),
            });
            vm.state_mut()
                .set_system_integer(0, 1)
                .expect("slot 0 exists");
            return Ok(true);
        }
        // Setters update the first node with the id; an unknown id is a silent no-op.
        let Some(button) = buttons.iter_mut().find(|button| button.id == id) else {
            return Ok(true);
        };
        if command == 0x213 {
            for (field, value) in button.rect.iter_mut().zip(&args[2..6]) {
                *field = *value as i32;
            }
            return Ok(true);
        }
        let image = ButtonImage {
            child: args[3] as i32,
            crop: [args[4], args[5], args[6], args[7]].map(|v| v as i16),
            origin: [args[8], args[9]].map(|v| v as i16),
        };
        // 0x14005f180: with dword_140101448 set, state 1 fills records 1 and 2,
        // state 2 fills record 3 and state 3 writes nothing.
        let records: &[usize] = match (self.merged_states, args[2]) {
            (_, 0) => &[0],
            (false, 1) => &[1],
            (false, 2) => &[2],
            (false, 3) => &[3],
            (true, 1) => &[1, 2],
            (true, 2) => &[3],
            _ => &[],
        };
        for &record in records {
            button.images[record] = image;
        }
        Ok(true)
    }

    /// 0x2216 (0x1400798c0 → 0x14005f4a0): system result 0 = hovered id or -1.
    /// 0x2217 (0x140079920 → 0x14005f700): one frame of navigation, hover, press and
    /// selection; system result 0 = selected id, -10 (released elsewhere), -2 (action 1
    /// released) or -1. Both return 0x4004.
    pub fn try_handle_input(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        input: &mut ActionInput,
    ) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x216 | 0x217 | 0x21a))) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, command + 0x2000, why);
        let raw = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let group = Self::slot(raw)
            .and_then(|slot| self.groups[slot].as_mut())
            .ok_or_else(|| fail("button group slot is empty or outside 0..5"))?;
        if command == 0x21a {
            // 0x140079a80 -> 0x14005f4c0: SP-8 id, SP-12 warp flag. A missing or disabled
            // button gives 0. Otherwise a pointer outside its rectangle is moved to the centre
            // when the flag is set (0x140056530, SetCursorPos); a different current button
            // gets image 0 back and the target image 1, and becomes current; result 1.
            let id = vm.stack().peek_u32(8).map_err(|e| fail(e.0))?;
            let warp = vm.stack().peek_u32(12).map_err(|e| fail(e.0))? != 0;
            let target = group
                .buttons
                .iter()
                .position(|b| b.id == id)
                .filter(|&i| group.buttons[i].enabled);
            let mut moved = None;
            if let Some(i) = target {
                let [x, y, w, h] = group.buttons[i].rect;
                let [px, py] = input.pointer();
                let inside = px >= x && py >= y && px < x.wrapping_add(w) && py < y.wrapping_add(h);
                if warp && !inside {
                    moved = Some([x.wrapping_add(w / 2), y.wrapping_add(h / 2)]);
                }
                if group.current != Some(i) {
                    let mut images = Vec::new();
                    if let Some(old) = group.current {
                        images.push(group.buttons[old].images[0]);
                    }
                    images.push(group.buttons[i].images[1]);
                    apply_images(scene, group.object, &images, false).map_err(fail)?;
                }
            }
            vm.complete_system(SystemResult(0x400c))
                .expect("arguments were checked");
            if let Some(i) = target {
                group.current = Some(i);
            }
            if let Some([x, y]) = moved {
                input.warp_pointer(x, y);
            }
            vm.state_mut()
                .set_system_integer(0, u32::from(target.is_some()))
                .expect("slot 0 exists");
            return Ok(true);
        }
        if command == 0x216 {
            let result = group.current.map_or(NOTHING, |i| group.buttons[i].id);
            vm.complete_system(SystemResult(0x4004))?;
            vm.state_mut()
                .set_system_integer(0, result)
                .expect("slot 0 exists");
            return Ok(true);
        }
        let frame = Frame::plan(group, input);
        apply_images(scene, group.object, &frame.images, false).map_err(fail)?;
        vm.complete_system(SystemResult(0x4004))
            .expect("argument was checked");
        vm.state_mut()
            .set_system_integer(0, frame.result)
            .expect("slot 0 exists");
        group.current = frame.current;
        group.pressed_visual = frame.pressed_visual;
        group.press_target = frame.press_target;
        for action in frame.consumed {
            input.consume(action);
        }
        if let Some([x, y]) = frame.warp {
            input.warp_pointer(x, y);
        }
        Ok(true)
    }
}

/// The effects of one 0x14005f700 call, computed before anything is committed.
struct Frame {
    images: Vec<ButtonImage>,
    current: Option<usize>,
    pressed_visual: bool,
    press_target: Option<usize>,
    consumed: Vec<usize>,
    warp: Option<[i32; 2]>,
    result: u32,
}

impl Frame {
    fn plan(group: &ButtonGroup, input: &ActionInput) -> Self {
        let buttons = &group.buttons;
        let find = |id: i32| buttons.iter().position(|b| b.id == id as u32);
        let mut consumed = Vec::new();
        let mut target = None;
        // Action 2 (consumed by 0x140054e60) moves to the previous link, else the last button.
        if input.released(2) {
            consumed.push(2);
            target = match group.current.map(|i| buttons[i].links[0]) {
                Some(link) if link >= 0 => find(link),
                _ => buttons.len().checked_sub(1),
            };
        }
        // Action 3 (consumed by 0x140054e90) moves to the next link, else the first button.
        if input.released(3) {
            consumed.push(3);
            target = match group.current.map(|i| buttons[i].links[1]) {
                Some(link) if link >= 0 => find(link),
                _ => (!buttons.is_empty()).then_some(0),
            };
        }
        // Navigation moves the pointer to the button centre (0x140056530, SetCursorPos).
        let warp = target.map(|i| {
            let [x, y, w, h] = buttons[i].rect;
            [x.wrapping_add(w / 2), y.wrapping_add(h / 2)]
        });
        let [px, py] = warp.unwrap_or_else(|| input.pointer());
        // The last enabled button containing the pointer wins.
        for (i, b) in buttons.iter().enumerate() {
            let [x, y, w, h] = b.rect;
            if b.enabled && px >= x && py >= y && px < x.wrapping_add(w) && py < y.wrapping_add(h) {
                target = Some(i);
            }
        }
        let mut images = Vec::new();
        let mut pressed_visual = group.pressed_visual;
        if target != group.current {
            if let Some(old) = group.current {
                images.push(buttons[old].images[0]);
            }
            if let Some(new) = target {
                images.push(buttons[new].images[1]);
            }
            pressed_visual = false;
        }
        let mut press_target = group.press_target;
        if input.pressed(0) {
            press_target = target;
        }
        let result = if input.released(0) {
            consumed.push(0);
            match target {
                Some(i) if press_target == Some(i) => buttons[i].id,
                _ => DECIDED_ELSEWHERE,
            }
        } else {
            if input.held(0) {
                if let (false, Some(i)) = (pressed_visual, target) {
                    images.push(buttons[i].images[2]);
                    pressed_visual = true;
                }
            } else {
                press_target = None;
                if pressed_visual {
                    if let Some(i) = target {
                        images.push(buttons[i].images[1]);
                    }
                    pressed_visual = false;
                }
            }
            if input.released(1) {
                consumed.push(1);
                CANCELLED
            } else {
                NOTHING
            }
        };
        Self {
            images,
            current: target,
            pressed_visual,
            press_target,
            consumed,
            warp,
            result,
        }
    }
}

/// A node's images as 0x14005f180 / 0x140004cf0 store them: child, crop, origin.
fn image_words(images: &[ButtonImage; BUTTON_STATES], out: &mut Vec<u8>) {
    for image in images {
        out.extend(image.child.to_le_bytes());
        for value in image.crop.iter().chain(&image.origin) {
            out.extend(value.to_le_bytes());
        }
    }
}

/// The header both records share: the u16 object field the writers leave 0 (the dword
/// 0x20000), version 2, then the pointer scale words +24/+28 of the group.
fn record_header(out: &mut Vec<u8>, scale: [u32; 2]) {
    out.extend(0x20000u32.to_le_bytes());
    out.extend(scale[0].to_le_bytes());
    out.extend(scale[1].to_le_bytes());
}

/// Reader side of both records: the object field and the saved scale.
fn record_binding(body: &[u8]) -> Result<(u8, [u32; 2]), &'static str> {
    let word = |at: usize| -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(
            body.get(at..at + 4)
                .ok_or("truncated button record")?
                .try_into()
                .unwrap(),
        ))
    };
    if u16::from_le_bytes([body[0], body[1]]) > 255 || body.get(2..4) != Some(&[2, 0]) {
        return Err("unsupported button record object or version");
    }
    Ok((body[0], [word(4)?, word(8)?]))
}

impl ButtonHost {
    /// Records 0x8c0 (0x14005efd0): header, +8 (the constructor's 1; only the unimplemented
    /// 0x2218/0x2219 touch it), the count, then each node +8..+112 (enabled, id, rect,
    /// links, four images; 104 bytes).
    pub(crate) fn saved_groups(&self) -> Vec<(u16, Vec<u8>)> {
        let screen = crate::message::RESEARCH_SCREEN;
        let mut out = Vec::new();
        for (slot, group) in self.groups.iter().enumerate() {
            let Some(group) = group else { continue };
            let mut body = Vec::new();
            record_header(&mut body, screen);
            body.extend(1u32.to_le_bytes());
            body.extend((group.buttons.len() as u32).to_le_bytes());
            for button in &group.buttons {
                body.extend(u32::from(button.enabled).to_le_bytes());
                body.extend(button.id.to_le_bytes());
                for value in button.rect.iter().chain(&button.links) {
                    body.extend(value.to_le_bytes());
                }
                image_words(&button.images, &mut body);
            }
            out.push((slot as u16, body));
        }
        out
    }
    /// Records 0x8c8 (0x140004b20): header, +8 active, +92 navigation off, +72 keep input,
    /// +76..+88 hit area, the count, then each 0x78-byte node +8..+116 (enabled, a word
    /// the constructor leaves unset and Rust writes as 0, id, rect, links, four images).
    pub(crate) fn saved_lists(&self) -> Vec<(u16, Vec<u8>)> {
        let mut out = Vec::new();
        for (slot, list) in self.lists.iter().enumerate() {
            let Some(list) = list else { continue };
            let mut body = Vec::new();
            record_header(&mut body, [list.screen[0], list.screen[1]]);
            for flag in [list.active, list.disable_navigation, list.keep_input] {
                body.extend(u32::from(flag).to_le_bytes());
            }
            for value in list.hit_area {
                body.extend(value.to_le_bytes());
            }
            body.extend((list.buttons.len() as u32).to_le_bytes());
            for button in &list.buttons {
                body.extend(u32::from(button.enabled).to_le_bytes());
                body.extend(0u32.to_le_bytes());
                body.extend(button.id.to_le_bytes());
                for value in button.rect.iter().chain(&button.links) {
                    body.extend(value.to_le_bytes());
                }
                image_words(&button.images, &mut body);
            }
            out.push((slot as u16, body));
        }
        out
    }
    /// Loader 0x140081960 frees both tables (0x2087's merged-state flag stays); the reader
    /// then builds each saved group or list with its constructor (0x14005ef80 /
    /// 0x140004ab0) bound to the record's object field, with the saved pointer scale, and
    /// passes the record buffer to the writer instead of reading the buttons back. A loaded
    /// group or list is therefore empty, bound to object 0 for records these writers made.
    pub(crate) fn restore(
        &mut self,
        groups: &[(u16, &[u8])],
        lists: &[(u16, &[u8])],
    ) -> Result<(), &'static str> {
        let mut restored_groups: [Option<ButtonGroup>; BUTTON_GROUPS] = Default::default();
        for &(slot, body) in groups {
            let (object, scale) = record_binding(body)?;
            if scale != crate::message::RESEARCH_SCREEN {
                return Err("a restored button group with a pointer scale is not modeled");
            }
            *restored_groups
                .get_mut(usize::from(slot))
                .ok_or("button group record outside 0..5")? = Some(ButtonGroup {
                object,
                buttons: Vec::new(),
                current: None,
                pressed_visual: false,
                press_target: None,
            });
        }
        let mut restored_lists: [Option<ListGroup>; 8] = Default::default();
        for &(slot, body) in lists {
            let (object, scale) = record_binding(body)?;
            let [w, h] = crate::message::RESEARCH_SCREEN;
            *restored_lists
                .get_mut(usize::from(slot))
                .ok_or("list group record outside 0..7")? = Some(ListGroup {
                object,
                buttons: Vec::new(),
                screen: [w, h, w, h, scale[0], scale[1]],
                active: true,
                current: None,
                pressed_visual: false,
                press_target: None,
                keep_input: false,
                disable_navigation: false,
                hit_area: [0, 0, w as i32, h as i32],
                hit_point: None,
            });
        }
        self.groups = restored_groups;
        self.lists = restored_lists;
        Ok(())
    }
}
