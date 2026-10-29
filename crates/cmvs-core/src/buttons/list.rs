//! Engine +4552 list groups, distinct from the six ordinary button groups.
use super::{BUTTON_LIMIT, BUTTON_STATES, Button, ButtonHost, ButtonImage, apply_images};
use crate::{
    bytecode::DecodeError, evaluate::error, machine::Machine, scene::SceneHost, vm::SystemResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListGroup {
    pub object: u8,
    pub buttons: Vec<Button>,
    pub screen: [u32; 6],
    pub active: bool,
    pub current: Option<usize>,
    pub pressed_visual: bool,
    pub press_target: Option<usize>,
    pub keep_input: bool,
    pub disable_navigation: bool,
    pub hit_area: [i32; 4],
    /// +96/+100 are first initialized by 0x2377, not the constructor.
    pub hit_point: Option<[i32; 2]>,
}

impl ButtonHost {
    pub fn list_group(&self, slot: usize) -> Option<&ListGroup> {
        self.lists.get(slot)?.as_ref()
    }

    pub(super) fn try_handle_list(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
    ) -> Result<bool, DecodeError> {
        let Some((pc, command @ (0x370..=0x376 | 0x378..=0x379 | 0x37c..=0x37f))) =
            vm.pending_system()
        else {
            return Ok(false);
        };
        let op = command + 0x2000;
        let fail = |why| error(pc, op, why);
        let count = match command {
            0x370 | 0x372 | 0x37c | 0x37e => 2,
            0x373 | 0x379 => 6,
            0x374 => 10,
            0x37d => 5,
            _ => 1,
        };
        let mut args = [0; 10];
        for (i, arg) in args.iter_mut().take(count).enumerate() {
            *arg = vm.stack().peek_u32((i + 1) * 4).map_err(|e| fail(e.0))?;
        }
        let slot = args[0] as usize;
        if slot >= 8 {
            return Err(fail("auxiliary list group slot outside 0..7"));
        }
        if command == 0x370 {
            let object =
                u8::try_from(args[1]).map_err(|_| fail("auxiliary list object outside 0..255"))?;
            if scene.object(object).is_none() {
                return Err(fail("auxiliary list object does not exist"));
            }
            vm.complete_system(SystemResult(0x4008))?;
            self.lists[slot] = Some(ListGroup {
                object,
                buttons: Vec::new(),
                screen: [1280, 720, 1280, 720, 1280, 720],
                active: true,
                current: None,
                pressed_visual: false,
                press_target: None,
                keep_input: false,
                disable_navigation: false,
                hit_area: [0, 0, 1280, 720],
                hit_point: None,
            });
            return Ok(true);
        }
        if command == 0x371 {
            vm.complete_system(SystemResult(0x4004))?;
            self.lists[slot] = None;
            return Ok(true);
        }
        let group = self.lists[slot]
            .as_ref()
            .ok_or_else(|| fail("auxiliary list group does not exist"))?;
        if command == 0x372 && group.buttons.len() >= BUTTON_LIMIT {
            return Err(fail("auxiliary list research limit exceeded"));
        }
        let hit_point = if command == 0x37f {
            Some(
                group
                    .hit_point
                    .ok_or_else(|| fail("auxiliary list hit point is not initialized"))?,
            )
        } else {
            None
        };
        if command == 0x375 {
            let images: Vec<_> = group
                .buttons
                .iter()
                .map(|b| b.images[if b.enabled { 0 } else { 3 }])
                .collect();
            apply_images(scene, group.object, &images, true).map_err(fail)?;
        }
        vm.complete_system(SystemResult(0x4000 | (count as u16 * 4)))?;
        let group = self.lists[slot].as_mut().expect("validated");
        match command {
            0x372 => {
                let previous = group.buttons.last().map_or(-1, |b| b.id as i32);
                if let Some(last) = group.buttons.last_mut() {
                    last.links[1] = args[1] as i32;
                }
                group.buttons.push(Button {
                    id: args[1],
                    enabled: true,
                    rect: [0; 4],
                    links: [previous, -1, -1, -1],
                    images: [ButtonImage::default(); BUTTON_STATES],
                });
                vm.state_mut()
                    .set_system_integer(0, 1)
                    .expect("result exists");
            }
            0x373 | 0x379 => {
                if let Some(button) = group.buttons.iter_mut().find(|b| b.id == args[1]) {
                    let values = [args[2], args[3], args[4], args[5]].map(|v| v as i32);
                    if command == 0x373 {
                        button.rect = values;
                    } else {
                        button.links = values;
                    }
                }
            }
            0x374 => {
                if let Some(button) = group.buttons.iter_mut().find(|b| b.id == args[1])
                    && args[2] < 4
                {
                    button.images[args[2] as usize] = ButtonImage {
                        child: args[3] as i32,
                        crop: [args[4], args[5], args[6], args[7]].map(|v| v as i16),
                        origin: [args[8], args[9]].map(|v| v as i16),
                    };
                }
            }
            0x375 => group.current = None,
            0x376 => {
                let result = group.current.map_or(u32::MAX, |i| group.buttons[i].id);
                vm.state_mut()
                    .set_system_integer(0, result)
                    .expect("result exists");
            }
            0x378 => group.active = false,
            0x37c => group.keep_input = args[1] != 0,
            0x37d => group.hit_area = [args[1], args[2], args[3], args[4]].map(|v| v as i32),
            0x37e => group.disable_navigation = args[1] != 0,
            0x37f => {
                for (slot, value) in [1, 2].into_iter().zip(hit_point.expect("validated")) {
                    vm.state_mut()
                        .set_system_integer(slot, value as u32)
                        .expect("result exists");
                }
            }
            _ => unreachable!(),
        }
        Ok(true)
    }
}
