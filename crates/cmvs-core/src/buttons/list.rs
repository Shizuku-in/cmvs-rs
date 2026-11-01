//! Engine +4552 list groups, distinct from the six ordinary button groups.
use super::{BUTTON_LIMIT, BUTTON_STATES, Button, ButtonHost, ButtonImage, apply_images};
use crate::{
    bytecode::DecodeError, dispatch::Call, evaluate::error, machine::Machine, scene::SceneHost,
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

    /// 0x2377 / 0x14007a0e0 -> 0x140005140. Hit rectangles are relative to
    /// the bound object's current position; the group hit area stays in screen space.
    pub fn try_handle_list_input(
        &mut self,
        vm: &mut Machine<'_>,
        scene: &mut SceneHost,
        input: &mut crate::input::ActionInput,
        gesture: &crate::gesture::PointerGesture,
    ) -> Result<bool, DecodeError> {
        let Some((pc, 0x377)) = vm.pending_system() else {
            return Ok(false);
        };
        self.handle_list_input(&mut Call::new(vm, pc, 0x377), scene, input, gesture)?;
        Ok(true)
    }

    pub(crate) fn handle_list_input(
        &mut self,
        call: &mut Call<'_, '_>,
        scene: &mut SceneHost,
        input: &mut crate::input::ActionInput,
        gesture: &crate::gesture::PointerGesture,
    ) -> Result<(), DecodeError> {
        let pc = call.pc;
        let fail = |why| error(pc, 0x2377, why);
        let slot = call.argument(4, super::argument_names(call.command)[0])? as usize;
        let group = self
            .lists
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or_else(|| fail("auxiliary list group does not exist or outside 0..7"))?;
        let draw = scene
            .object(group.object)
            .and_then(|o| o.draw.as_ref())
            .ok_or_else(|| fail("auxiliary list object has no draw state"))?;
        let position = match draw.words[0] {
            0 => [draw.words[8], draw.words[9]].map(|v| v as i32),
            5 => [draw.words[11], draw.words[12]].map(|v| {
                let f = f32::from_bits(v);
                if !(-2147483648.0..2147483648.0).contains(&f) {
                    i32::MIN
                } else {
                    f as i32
                }
            }),
            _ => {
                return Err(fail(
                    "auxiliary list hit testing needs 2D or font coordinates",
                ));
            }
        };
        let mut view = super::ButtonGroup {
            object: group.object,
            buttons: group.buttons.clone(),
            current: group.current,
            pressed_visual: group.pressed_visual,
            press_target: group.press_target,
        };
        for button in &mut view.buttons {
            for (coordinate, offset) in button.rect[..2].iter_mut().zip(position) {
                *coordinate = coordinate.wrapping_add(offset);
            }
        }
        let dragging = gesture.field(4) == Some(1) && gesture.field(120).is_some_and(|v| v != 0);
        let mut local = input.clone();
        if group.disable_navigation || dragging {
            local.consume(2);
            local.consume(3);
        }
        let planned = super::Frame::plan(&view, &local);
        let [px, py] = planned.warp.unwrap_or_else(|| input.pointer());
        let [x, y, w, h] = group.hit_area;
        let inside =
            px >= x && py >= y && px < x.wrapping_add(w) && py < y.wrapping_add(h) && !dragging;
        if !inside {
            for button in &mut view.buttons {
                button.enabled = false;
            }
        }
        let mut frame = super::Frame::plan(&view, &local);
        let mut hit = [-1, -1];
        if inside && let Some(index) = frame.current {
            let button = &view.buttons[index];
            let [x, y, w, h] = button.rect;
            if button.enabled
                && px >= x
                && py >= y
                && px < x.wrapping_add(w)
                && py < y.wrapping_add(h)
            {
                hit = [px.wrapping_sub(x), py.wrapping_sub(y)];
            }
        }
        if group.keep_input {
            frame.consumed.retain(|action| !matches!(action, 0 | 1));
        }
        apply_images(scene, group.object, &frame.images, false).map_err(fail)?;
        call.complete(0x4004)?;
        call.vm
            .state_mut()
            .set_system_integer(0, frame.result)
            .expect("result exists");
        let group = self.lists[slot].as_mut().expect("validated");
        group.current = frame.current;
        group.press_target = frame.press_target;
        group.pressed_visual = frame.pressed_visual;
        group.hit_point = Some(hit);
        for action in frame.consumed {
            input.consume(action);
        }
        if let Some([x, y]) = frame.warp {
            input.warp_pointer(x, y);
        }
        Ok(())
    }

    pub(super) fn handle_list(
        &mut self,
        call: &mut Call<'_, '_>,
        scene: &mut SceneHost,
    ) -> Result<(), DecodeError> {
        let (pc, command) = (call.pc, call.command);
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
            *arg = call.argument((i + 1) * 4, super::argument_names(call.command)[i])?;
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
            call.complete(0x4008)?;
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
            return Ok(());
        }
        if command == 0x371 {
            call.complete(0x4004)?;
            self.lists[slot] = None;
            return Ok(());
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
        call.complete(0x4000 | (count as u16 * 4))?;
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
                call.vm
                    .state_mut()
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
                call.vm
                    .state_mut()
                    .set_system_integer(0, result)
                    .expect("result exists");
            }
            0x378 => group.active = false,
            0x37c => group.keep_input = args[1] != 0,
            0x37d => group.hit_area = [args[1], args[2], args[3], args[4]].map(|v| v as i32),
            0x37e => group.disable_navigation = args[1] != 0,
            0x37f => {
                for (slot, value) in [1, 2].into_iter().zip(hit_point.expect("validated")) {
                    call.vm
                        .state_mut()
                        .set_system_integer(slot, value as u32)
                        .expect("result exists");
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
