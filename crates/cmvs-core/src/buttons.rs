//! Engine +4504: six button groups bound to scene objects (0x2210..0x221b); addresses refer
//! to the documented binary hash. Covers groups, buttons, hit rectangles, navigation links,
//! per-state images, the initial display of 0x2215 and the hover/press/selection logic of
//! 0x2216/0x2217 over `ActionInput`. 0x2218/0x2219/0x221a/0x221b stay unhandled.
//! 0x2371 only handles the initially empty auxiliary list slots at engine +4552;
//! their creation commands remain barriers and must gain owned state before support.
use crate::{
    bytecode::DecodeError, evaluate::error, input::ActionInput, machine::Machine, scene::SceneHost,
    vm::SystemResult,
};

pub const BUTTON_GROUPS: usize = 6;
pub const BUTTON_STATES: usize = 4;
/// Research limit; the original allocates list nodes without a bound.
const BUTTON_LIMIT: usize = 256;

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

#[derive(Debug, Default)]
pub struct ButtonHost {
    groups: [Option<ButtonGroup>; BUTTON_GROUPS],
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
        if let Some((pc, 0x371)) = vm.pending_system() {
            // Engine +4552 has eight auxiliary list groups, separate from +4504.
            // 0x140014f8a..0x140014fbb initializes them all to null. Their creation
            // commands (0x2370/0x237b) remain unhandled, so only the verified empty
            // branch of 0x140079e10 can be reached. When adding creation, store the
            // owned groups and implement their destruction here before enabling it.
            let slot = vm.stack().peek_u32(4).map_err(|e| error(pc, 0x2371, e.0))?;
            if slot >= 8 {
                return Err(error(pc, 0x2371, "auxiliary list group slot outside 0..7"));
            }
            vm.complete_system(SystemResult(0x4004))?;
            return Ok(true);
        }
        let Some((pc, command @ (0x87 | 0x210..=0x215))) = vm.pending_system() else {
            return Ok(false);
        };
        let op = command + 0x2000;
        let fail = |why| error(pc, op, why);
        let count = match command {
            0x87 | 0x211 | 0x215 => 1,
            0x210 | 0x212 => 2,
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
        if command == 0x210 {
            // 0x140079370 stores the object pointer, which is null for an empty slot.
            let object = u8::try_from(args[1] as i32)
                .ok()
                .filter(|object| scene.object(*object).is_some())
                .ok_or_else(|| fail("button group needs an existing scene object"))?;
            vm.complete_system(SystemResult(0x4008))?;
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
        let Some((pc, command @ (0x216 | 0x217))) = vm.pending_system() else {
            return Ok(false);
        };
        let fail = |why| error(pc, command + 0x2000, why);
        let raw = vm.stack().peek_u32(4).map_err(|e| fail(e.0))?;
        let group = Self::slot(raw)
            .and_then(|slot| self.groups[slot].as_mut())
            .ok_or_else(|| fail("button group slot is empty or outside 0..5"))?;
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
