//! Engine +4504: six button groups bound to scene objects (0x2210..0x221b); addresses refer
//! to the documented binary hash. Data layer only: groups, buttons, hit rectangles,
//! navigation links and per-state images, plus the initial display of 0x2215. Input
//! hit-testing (0x2217/0x221a) and the remaining commands stay unhandled.
use crate::{
    bytecode::DecodeError, evaluate::error, machine::Machine, scene::SceneHost, vm::SystemResult,
};

pub const BUTTON_GROUPS: usize = 6;
pub const BUTTON_STATES: usize = 4;
/// Research limit; the original allocates list nodes without a bound.
const BUTTON_LIMIT: usize = 256;

/// Node +48 + 16 * state: what 0x2215 applies to the child object of the group's object.
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
    /// Node +8, set at creation; 0x2215 shows image 3 instead of 0 when cleared.
    pub enabled: bool,
    /// Node +16..+28 from 0x2213.
    pub rect: [i32; 4],
    /// Node +32..+44: previous and next ids in insertion order, then -1, -1.
    pub links: [i32; 4],
    pub images: [ButtonImage; BUTTON_STATES],
}

/// A 0x48-byte group from 0x14005ef80. The original also copies six screen metrics at
/// creation for input hit-testing; they are not modeled until that path is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonGroup {
    /// Scene slot whose object pointer the original stores at +48.
    pub object: u8,
    pub buttons: Vec<Button>,
}

#[derive(Debug, Default)]
pub struct ButtonHost {
    groups: [Option<ButtonGroup>; BUTTON_GROUPS],
    /// dword_140101448 (zero-filled .data tail), set only by 0x2087.
    merged_states: bool,
}

impl ButtonHost {
    pub fn group(&self, slot: usize) -> Option<&ButtonGroup> {
        self.groups.get(slot).and_then(Option::as_ref)
    }
    pub fn merged_states(&self) -> bool {
        self.merged_states
    }
    pub fn try_handle(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
    ) -> Result<bool, DecodeError> {
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
        // Handlers compare the slot as a signed int against 6; larger values only set
        // +12056 bit 0x100000 (unmodeled) and negative values index before the table.
        let slot = usize::try_from(args[0] as i32)
            .ok()
            .filter(|slot| *slot < BUTTON_GROUPS)
            .ok_or_else(|| fail("button group slot outside 0..5"))?;
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
            });
            return Ok(true);
        }
        if command == 0x211 {
            vm.complete_system(SystemResult(0x4004))?;
            self.groups[slot] = None;
            return Ok(true);
        }
        let group = self.groups[slot]
            .as_ref()
            .ok_or_else(|| fail("button group slot is empty"))?;
        if command == 0x215 {
            return Self::show(group, vm, scene, pc);
        }
        let id = args[1];
        if command == 0x212 && group.buttons.len() >= BUTTON_LIMIT {
            return Err(fail("button count exceeds research limit"));
        }
        vm.complete_system(SystemResult(0x4000 | (count * 4) as u16))?;
        let buttons = &mut self.groups[slot].as_mut().expect("checked").buttons;
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

    /// 0x2215 / 0x14005f3c0: for every button, apply image 0 (image 3 when disabled) to the
    /// child of the group's object with the 0x2044/0x2046 setters, then set child +6244.
    /// The original dereferences a missing child or draw state; every target is checked
    /// before anything is written. Clearing group +16 (current item) is not modeled yet.
    fn show(
        group: &ButtonGroup,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        pc: usize,
    ) -> Result<bool, DecodeError> {
        let fail = |why| error(pc, 0x2215, why);
        let parent = scene
            .object(group.object)
            .ok_or_else(|| fail("button group object no longer exists"))?;
        let images = group
            .buttons
            .iter()
            .map(|button| button.images[if button.enabled { 0 } else { 3 }])
            .collect::<Vec<_>>();
        for image in &images {
            let child = u16::try_from(image.child)
                .ok()
                .and_then(|child| parent.children.get(&child))
                .ok_or_else(|| fail("button image child does not exist"))?;
            if child.draw.is_none() {
                return Err(fail("button image child has no draw state"));
            }
        }
        vm.complete_system(SystemResult(0x4004))?;
        let parent = scene.object_mut(group.object).expect("checked");
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
            child.shown = true;
        }
        Ok(true)
    }
}
