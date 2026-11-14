use cmvs_rs::{
    buttons::ButtonHost,
    dispatch::{self, Capability, Flow, Hosts, InputHosts},
    gesture::PointerGesture,
    input::ActionInput,
    machine::Machine,
    profile::Semantics,
    scene::SceneHost,
};
mod support;
use support::bytecode::{command, pending, script};

#[derive(Default)]
struct Controls {
    scene: SceneHost,
    buttons: ButtonHost,
    input: ActionInput,
    gesture: PointerGesture,
}
impl Controls {
    fn hosts(&mut self, capability: Capability) -> Hosts<'_> {
        let button = matches!(
            capability,
            Capability::Buttons | Capability::ButtonInput | Capability::ListInput
        );
        let input = capability != Capability::Buttons;
        let gesture = matches!(capability, Capability::ListInput | Capability::DragPress);
        Hosts {
            scene: button.then_some(&mut self.scene),
            buttons: button.then_some(&mut self.buttons),
            input: input.then_some(InputHosts {
                input: &mut self.input,
                skip: false,
                clock: (capability == Capability::DragPress).then_some(123),
            }),
            gesture: gesture.then_some(&mut self.gesture),
            list_input: capability == Capability::ListInput,
            ..Default::default()
        }
    }
    fn snapshot(&self) -> String {
        format!(
            "{:?}{:?}{:?}{:?}",
            self.scene, self.buttons, self.input, self.gesture
        )
    }
    fn setup(&mut self) {
        let mut code = vec![];
        for (op, args) in [
            (0x2020, vec![3]),
            (0x2040, vec![3]),
            (0x2022, vec![3, 0]),
            (0x2050, vec![3, 0]),
            (0x2210, vec![0, 3]),
            (0x2370, vec![0, 3]),
            (0x2377, vec![0]),
        ] {
            command(&mut code, op, &args);
        }
        let script = script(&code);
        let mut vm = Machine::new(&script).unwrap();
        for i in 0..7 {
            pending(&mut vm);
            let capability = if i < 4 {
                Capability::Scene
            } else if i < 6 {
                Capability::Buttons
            } else {
                Capability::ListInput
            };
            if capability == Capability::Scene {
                assert!(self.scene.try_handle(&mut vm).unwrap());
            } else {
                assert_eq!(
                    dispatch::run(Semantics::Root, &mut vm, &mut self.hosts(capability)).unwrap(),
                    Some(Flow::Done)
                );
            }
        }
    }
}

#[test]
fn partial_button_and_gesture_contexts_cannot_run_or_mutate_input() {
    for (opcode, args, capability, parts) in [
        (0x2210, vec![0, 3], Capability::Buttons, 2),
        (0x2217, vec![0], Capability::ButtonInput, 3),
        (0x2377, vec![0], Capability::ListInput, 5),
        (0x2369, vec![1], Capability::DragPress, 3),
    ] {
        for missing in 0..parts {
            let mut controls = Controls::default();
            controls.setup();
            let mut code = vec![];
            command(&mut code, opcode, &args);
            let script = script(&code);
            let mut vm = Machine::new(&script).unwrap();
            pending(&mut vm);
            let before = (vm.pc(), vm.pending_system(), vm.stack().len_bytes());
            let state = controls.snapshot();
            let mut hosts = controls.hosts(capability);
            if capability == Capability::DragPress {
                match missing {
                    0 => hosts.gesture = None,
                    1 => hosts.input = None,
                    _ => hosts.input.as_mut().unwrap().clock = None,
                }
            } else {
                match missing {
                    0 => hosts.buttons = None,
                    1 => hosts.scene = None,
                    2 => hosts.input = None,
                    3 => hosts.gesture = None,
                    _ => hosts.list_input = false,
                }
            }
            assert_eq!(
                dispatch::run(Semantics::Root, &mut vm, &mut hosts).unwrap(),
                Some(Flow::Unavailable)
            );
            assert_eq!(
                (vm.pc(), vm.pending_system(), vm.stack().len_bytes()),
                before
            );
            assert_eq!(controls.snapshot(), state);
        }
    }
}
