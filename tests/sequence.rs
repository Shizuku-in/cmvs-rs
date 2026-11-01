#[path = "support/shared_media.rs"]
mod shared_media;
use shared_media::SharedSequence;
mod support;
use cmvs_rs::{
    machine::Machine,
    sequence::{SequenceHeader, SequenceHost},
};
use support::bytecode::{command, pending, script};

#[test]
fn reset_zeroes_the_header_and_pops_nothing() {
    // A word left by earlier code stays on the stack: 0x2280 returns 0x4000.
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(5u32.to_le_bytes());
    code.extend(0x2280u16.to_le_bytes());
    code.extend(0x228bu16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 3).unwrap();
    let mut sequence = SequenceHost::default();
    pending(&mut vm);
    assert!(sequence.shared_sequence(&mut vm).unwrap());
    assert_eq!(sequence.header(), SequenceHeader::default());
    assert_eq!(vm.stack().len_bytes(), 4);
    assert_eq!(vm.state().system_integer(0), Some(3));
    // Other commands of the family stay pending.
    pending(&mut vm);
    assert!(!sequence.shared_sequence(&mut vm).unwrap());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x28b));
}

#[test]
fn count_query_preserves_stack_header_and_other_results() {
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0xdeadbeefu32.to_le_bytes());
    code.extend(0x2285u16.to_le_bytes());
    code.extend(0x2285u16.to_le_bytes());
    code.extend(0x228bu16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_system_integer(0, 99).unwrap();
    vm.state_mut().set_system_integer(4, 7).unwrap();
    let mut sequence = SequenceHost::default();
    assert!(!sequence.shared_sequence(&mut vm).unwrap());
    for _ in 0..2 {
        pending(&mut vm);
        let pc = vm.pc();
        assert!(sequence.shared_sequence(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.state().system_integer(0), Some(0));
        assert_eq!(vm.state().system_integer(4), Some(7));
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0xdeadbeef);
        assert_eq!(sequence.header(), SequenceHeader::default());
    }
    pending(&mut vm);
    assert!(!sequence.shared_sequence(&mut vm).unwrap());
    assert_eq!(vm.pending_system().map(|p| p.1), Some(0x28b));
}

#[test]
fn current_index_uses_activity_not_count_and_preserves_raw_index_bits() {
    for (active, count, index, expected) in [
        (0, 0, 0, u32::MAX),
        (0, 7, 3, u32::MAX),
        (1, 7, 3, 3),
        (2, 0, 0x80000000, 0x80000000),
        (u32::MAX, 1, u32::MAX, u32::MAX),
    ] {
        let header = SequenceHeader {
            active,
            count,
            index,
            start: 77,
            waited: 88,
        };
        assert_eq!(header.current_index(), expected);
        assert_eq!(header.start, 77);
        assert_eq!(header.waited, 88);
    }
}

#[test]
fn current_index_query_returns_minus_one_when_inactive_and_does_not_consume_parameters() {
    let mut code = vec![];
    code.extend(0x440u16.to_le_bytes());
    code.extend(4u16.to_le_bytes());
    code.extend(0xdeadbeefu32.to_le_bytes());
    code.extend(0x2281u16.to_le_bytes());
    code.extend(0x2281u16.to_le_bytes());
    let script = script(&code);
    let mut vm = Machine::new(&script).unwrap();
    let mut sequence = SequenceHost::default();
    for slot in 0..7 {
        vm.state_mut().set_system_integer(slot, slot + 10).unwrap();
    }
    for _ in 0..2 {
        pending(&mut vm);
        let pc = vm.pc();
        let header = sequence.header();
        assert!(sequence.shared_sequence(&mut vm).unwrap());
        assert_eq!(vm.pc(), pc + 2);
        assert_eq!(vm.pending_system(), None);
        assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
        for slot in 1..7 {
            assert_eq!(vm.state().system_integer(slot), Some(slot + 10));
        }
        assert_eq!(vm.stack().len_bytes(), 4);
        assert_eq!(vm.stack().peek_u32(4).unwrap(), 0xdeadbeef);
        assert_eq!(sequence.header(), header);
    }
}

fn run(vm: &mut Machine<'_>, sequence: &mut SequenceHost) {
    pending(vm);
    assert!(sequence.shared_sequence(vm).unwrap());
}

#[test]
fn wait_append_caps_at_sixteen_preserves_results_and_reset_keeps_inactive_entries() {
    use cmvs_rs::sequence::SequenceEntry;
    let mut code = vec![];
    for duration in 0..17 {
        command(&mut code, 0x2286, &[duration]);
    }
    command(&mut code, 0x2285, &[]);
    command(&mut code, 0x2280, &[]);
    command(&mut code, 0x2286, &[999]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::default();
    vm.state_mut().set_system_integer(0, 123).unwrap();
    for count in 1..=17 {
        run(&mut vm, &mut host);
        assert_eq!(host.header().count, count.min(16));
        assert_eq!(vm.state().system_integer(0), Some(123));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    for i in 0..16 {
        assert_eq!(
            host.entries()[i],
            Some(SequenceEntry::Wait { duration: i as u32 })
        );
    }
    run(&mut vm, &mut host);
    assert_eq!(vm.state().system_integer(0), Some(16));
    let entries = host.entries().clone();
    host.advance_frame(42).unwrap();
    run(&mut vm, &mut host);
    assert_eq!(host.header(), SequenceHeader::default());
    assert_eq!(host.entries(), &entries);
    assert_eq!(host.clock(), 42);
    run(&mut vm, &mut host);
    assert_eq!(
        host.entries()[0],
        Some(SequenceEntry::Wait { duration: 999 })
    );
    assert_eq!(&host.entries()[1..], &entries[1..]);
    assert_eq!(host.header().count, 1);
}

#[test]
fn wait_sequence_start_stop_restart_and_queries_follow_header_lifecycle() {
    let mut code = vec![];
    for (op, args) in [
        (0x2282, vec![]),
        (0x2286, vec![10]),
        (0x2286, vec![20]),
        (0x2282, vec![]),
        (0x2281, vec![]),
        (0x2282, vec![]),
        (0x2283, vec![]),
        (0x2282, vec![]),
        (0x2281, vec![]),
    ] {
        command(&mut code, op, &args);
    }
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::with_tick(100);
    run(&mut vm, &mut host);
    assert_eq!(host.header(), SequenceHeader::default());
    for _ in 0..3 {
        run(&mut vm, &mut host);
    }
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 1,
            count: 2,
            index: 0,
            start: 100,
            waited: 0
        }
    );
    host.advance_frame(15).unwrap();
    assert_eq!(host.header().index, 1);
    assert_eq!(host.header().waited, 10);
    run(&mut vm, &mut host);
    assert_eq!(vm.state().system_integer(0), Some(1));
    let active = host.header();
    run(&mut vm, &mut host);
    assert_eq!(host.header(), active, "start while active is a no-op");
    run(&mut vm, &mut host);
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 0,
            ..active
        }
    );
    host.advance_frame(100).unwrap();
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 0,
            ..active
        }
    );
    run(&mut vm, &mut host);
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 1,
            count: 2,
            index: 0,
            start: 215,
            waited: 0
        }
    );
    host.advance_frame(30).unwrap();
    assert_eq!(
        host.header(),
        SequenceHeader {
            active: 0,
            count: 2,
            index: 2,
            start: 215,
            waited: 30
        }
    );
    run(&mut vm, &mut host);
    assert_eq!(vm.state().system_integer(0), Some(u32::MAX));
    assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
}

#[test]
fn waits_use_wrapping_absolute_time_and_carry_leftover_time_without_frame_clamp() {
    for start in [0, u32::MAX - 15] {
        let mut code = vec![];
        for duration in [0, 20, 40, 0] {
            command(&mut code, 0x2286, &[duration]);
        }
        command(&mut code, 0x2282, &[]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = SequenceHost::with_tick(start);
        for _ in 0..5 {
            run(&mut vm, &mut host);
        }
        host.advance_frame(0).unwrap();
        assert_eq!(host.header().index, 1);
        host.advance_frame(25).unwrap();
        assert_eq!(
            (
                host.header().index,
                host.header().waited,
                host.header().active
            ),
            (2, 20, 1)
        );
        assert_eq!(host.clock(), start.wrapping_add(25));
        host.advance_frame(34).unwrap();
        assert_eq!(host.header().active, 1);
        host.advance_frame(1).unwrap();
        assert_eq!(
            (
                host.header().index,
                host.header().waited,
                host.header().active
            ),
            (4, 60, 0)
        );
        let finished = host.header();
        host.advance_frame(1000).unwrap();
        assert_eq!(host.header(), finished);
    }
    let mut code = vec![];
    command(&mut code, 0x2286, &[300]);
    command(&mut code, 0x2282, &[]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::default();
    run(&mut vm, &mut host);
    run(&mut vm, &mut host);
    host.advance_frame(300).unwrap();
    assert_eq!(
        host.header().active,
        0,
        "sequence time is not clamped to 200ms"
    );
}

#[test]
fn finishing_waits_clears_header_keeps_entries_clock_stack_and_results() {
    for active in [false, true] {
        let mut code = vec![];
        command(&mut code, 0x2286, &[u32::MAX]);
        if active {
            command(&mut code, 0x2282, &[]);
        }
        command(&mut code, 0x2284, &[]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = SequenceHost::default();
        run(&mut vm, &mut host);
        if active {
            run(&mut vm, &mut host);
        }
        host.advance_frame(10).unwrap();
        let entries = host.entries().clone();
        vm.state_mut().set_system_integer(0, 123).unwrap();
        run(&mut vm, &mut host);
        assert_eq!(host.header(), SequenceHeader::default());
        assert_eq!(host.entries(), &entries);
        assert_eq!(host.clock(), 10);
        assert_eq!(vm.state().system_integer(0), Some(123));
        assert_eq!((vm.pc(), vm.stack().len_bytes()), (code.len(), 0));
    }
}

#[test]
fn failed_wait_append_preserves_active_sequence_and_unknown_actions_remain_pending() {
    let mut code = vec![];
    command(&mut code, 0x2286, &[100]);
    command(&mut code, 0x2282, &[]);
    command(&mut code, 0x2286, &[]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::default();
    run(&mut vm, &mut host);
    run(&mut vm, &mut host);
    host.advance_frame(25).unwrap();
    let header = host.header();
    let entries = host.entries().clone();
    vm.state_mut().set_system_integer(0, 123).unwrap();
    pending(&mut vm);
    for _ in 0..2 {
        assert!(host.shared_sequence(&mut vm).is_err());
        assert_eq!(host.header(), header);
        assert_eq!(host.entries(), &entries);
        assert_eq!(host.clock(), 25);
        assert_eq!(vm.state().system_integer(0), Some(123));
        assert_eq!(vm.pending_system(), Some((code.len() - 2, 0x286)));
        assert_eq!(vm.stack().len_bytes(), 0);
    }
    for opcode in [0x228bu16, 0x228d] {
        let program = script(&opcode.to_le_bytes());
        let mut vm = Machine::new(&program).unwrap();
        pending(&mut vm);
        assert!(!host.shared_sequence(&mut vm).unwrap());
        assert_eq!(vm.pending_system(), Some((0, opcode - 0x2000)));
        assert_eq!(host.header(), header);
        assert_eq!(host.entries(), &entries);
    }
}

#[test]
fn camera_sequence_starts_motion_at_execution_and_requires_the_camera_host() {
    let mut code = vec![];
    let xyz = [10f32, 20.0, 30.0].map(f32::to_bits);
    command(&mut code, 0x2287, &[0, xyz[0], xyz[1], xyz[2], 100]);
    command(&mut code, 0x2282, &[]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::default();
    let mut scene = motion_scene();
    let mut camera = cmvs_rs::camera::CameraHost::default();
    for _ in 0..2 {
        run(&mut vm, &mut host);
    }
    assert!(
        host.advance_frame_with_images(0, &mut scene, &mut NoImages)
            .is_err()
    );
    assert_eq!(host.header().index, 0);
    host.advance_frame_with_hosts(0, &mut scene, &mut NoImages, &mut camera)
        .unwrap();
    assert_eq!(host.header().index, 1);
    assert!(camera.moving());
    camera.advance_frame(100);
    assert_eq!(&camera.camera(0).unwrap().words[1..4], &xyz);
    assert!(!camera.moving());
    assert_eq!(vm.stack().len_bytes(), 0);
}

#[test]
fn camera_sequence_immediate_paths_quantize_xyz_without_cancelling_old_motion() {
    for finish in [false, true] {
        let mut old = vec![];
        command(
            &mut old,
            0x206c,
            &[0, 90f32.to_bits(), 80f32.to_bits(), 70f32.to_bits(), 500],
        );
        let old = script(&old);
        let mut old_vm = Machine::new(&old).unwrap();
        let mut camera = cmvs_rs::camera::CameraHost::default();
        pending(&mut old_vm);
        camera.try_handle(&mut old_vm).unwrap();
        let old_words = camera.camera(0).unwrap().words;
        let mut code = vec![];
        command(
            &mut code,
            0x2287,
            &[
                0xffff,
                1.239f32.to_bits(),
                2.349f32.to_bits(),
                3.459f32.to_bits(),
                if finish { 500 } else { 0 },
            ],
        );
        command(&mut code, 0x2282, &[]);
        if finish {
            command(&mut code, 0x2284, &[]);
        }
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = SequenceHost::default();
        let mut scene = motion_scene();
        for _ in 0..2 {
            run(&mut vm, &mut host);
        }
        if finish {
            pending(&mut vm);
            host.shared_sequence_hosts(&mut vm, &mut scene, &mut NoImages, &mut camera)
                .unwrap();
        } else {
            host.advance_frame_with_hosts(0, &mut scene, &mut NoImages, &mut camera)
                .unwrap();
        }
        let words = camera.camera(0).unwrap().words;
        assert_eq!(&words[1..4], &[1.23f32, 2.34, 3.45].map(f32::to_bits));
        assert_eq!(&words[21..37], &old_words[21..37]);
    }
}

struct NoImages;
impl cmvs_rs::image::ImageSource for NoImages {
    fn read(&mut self, _: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        panic!("world motion must not read image resources")
    }
}
fn motion_scene() -> cmvs_rs::scene::SceneHost {
    let mut code = vec![];
    command(&mut code, 0x2020, &[20]);
    command(&mut code, 0x2040, &[20]);
    for (i, value) in [10f32, 20.0, 30.0].into_iter().enumerate() {
        command(
            &mut code,
            0x2072 + i as u16,
            &[20, u32::MAX, value.to_bits()],
        );
    }
    command(&mut code, 0x21f9, &[20, u32::MAX]);
    command(
        &mut code,
        0x21fa,
        &[20, u32::MAX, 7, u32::MAX, 0, 999, 0, 0, 0],
    );
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut scene = cmvs_rs::scene::SceneHost::default();
    for _ in 0..7 {
        pending(&mut vm);
        assert!(scene.try_handle(&mut vm).unwrap());
    }
    scene
}
#[test]
fn motion_sequence_defers_rebuild_captures_execution_base_and_does_not_wait_for_motion() {
    let mut code = vec![];
    command(&mut code, 0x2286, &[30]);
    command(
        &mut code,
        0x228a,
        &[
            20,
            1f32.to_bits(),
            2f32.to_bits(),
            3f32.to_bits(),
            0x203e0039,
            400,
        ],
    );
    command(&mut code, 0x2286, &[50]);
    command(&mut code, 0x2282, &[]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::default();
    let mut scene = motion_scene();
    let before = scene.object(20).unwrap().clone();
    vm.state_mut().set_system_integer(0, 123).unwrap();
    for _ in 0..4 {
        run(&mut vm, &mut host);
    }
    assert_eq!(vm.stack().len_bytes(), 0);
    assert_eq!(vm.state().system_integer(0), Some(123));
    assert_eq!(scene.object(20), Some(&before));
    // Change a coordinate after append/start: the animation must capture it at execution.
    let mut setter = vec![];
    command(&mut setter, 0x2072, &[20, u32::MAX, 11f32.to_bits()]);
    let setter = script(&setter);
    let mut setter_vm = Machine::new(&setter).unwrap();
    pending(&mut setter_vm);
    scene.try_handle(&mut setter_vm).unwrap();
    let execution_words = scene.object(20).unwrap().draw.as_ref().unwrap().words;
    host.advance_frame_with_images(29, &mut scene, &mut NoImages)
        .unwrap();
    assert_eq!(host.header().index, 0);
    host.advance_frame_with_images(1, &mut scene, &mut NoImages)
        .unwrap();
    assert_eq!(host.header().index, 2);
    let draw = scene.object(20).unwrap().draw.as_ref().unwrap();
    assert_eq!(draw.words, execution_words);
    let motion = draw.world_motion.as_ref().unwrap();
    assert_eq!(
        motion.keys[7],
        Default::default(),
        "rebuild discards old keys"
    );
    assert_eq!(
        motion.playback.as_ref().unwrap().base,
        [11f32, 20.0, 30.0].map(f32::to_bits)
    );
    assert_eq!(motion.playback.as_ref().unwrap().elapsed, 0);
    scene.advance_frame(100);
    assert_eq!(
        scene.object(20).unwrap().draw.as_ref().unwrap().words[12],
        20.62f32.to_bits()
    );
    host.advance_frame_with_images(50, &mut scene, &mut NoImages)
        .unwrap();
    assert_eq!(
        host.header().active,
        0,
        "sequence waits are independent of animation lifetime"
    );
    scene.advance_frame(200);
    scene.advance_frame(100);
    let draw = scene.object(20).unwrap().draw.as_ref().unwrap();
    assert_eq!(&draw.words[11..14], &[1f32, 2.0, 3.0].map(f32::to_bits));
    assert!(draw.world_motion.is_none());
}
#[test]
fn motion_sequence_fast_finish_skips_without_changing_existing_track_or_validating_mode() {
    for with_scene in [false, true] {
        let mut code = vec![];
        command(&mut code, 0x228a, &[20, 0, 0, 0, 0xffff, 500]);
        command(&mut code, 0x2282, &[]);
        command(&mut code, 0x2284, &[]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = SequenceHost::default();
        let mut scene = motion_scene();
        let before = scene.object(20).unwrap().clone();
        run(&mut vm, &mut host);
        run(&mut vm, &mut host);
        pending(&mut vm);
        if with_scene {
            assert!(
                host.shared_sequence_images(&mut vm, &mut scene, &mut NoImages)
                    .unwrap()
            );
        } else {
            assert!(host.shared_sequence(&mut vm).unwrap());
        }
        assert_eq!(scene.object(20), Some(&before));
        assert_eq!(host.header(), SequenceHeader::default());
        assert!(host.entries()[0].is_some());
    }
}

#[test]
fn subtype_one_motion_runs_normally_but_fast_finish_snaps_and_removes_the_old_track() {
    for finish in [false, true] {
        let xyz = if finish {
            [0x8000_0000, 0x7fc0_1234, 0x7f80_0000]
        } else {
            [1f32, 2.0, 3.0].map(f32::to_bits)
        };
        let mut code = vec![];
        command(
            &mut code,
            0x2289,
            &[
                20,
                xyz[0],
                xyz[1],
                xyz[2],
                if finish { 0xffff } else { 0 },
                100,
            ],
        );
        command(&mut code, 0x2282, &[]);
        if finish {
            command(&mut code, 0x2284, &[]);
        }
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = SequenceHost::default();
        let mut scene = motion_scene();
        vm.state_mut().set_system_integer(0, 123).unwrap();
        run(&mut vm, &mut host);
        run(&mut vm, &mut host);
        if finish {
            pending(&mut vm);
            let before = host.header();
            assert!(host.shared_sequence(&mut vm).is_err());
            assert_eq!(host.header(), before);
            host.shared_sequence_images(&mut vm, &mut scene, &mut NoImages)
                .unwrap();
        } else {
            host.advance_frame_with_images(0, &mut scene, &mut NoImages)
                .unwrap();
            let motion = scene
                .object(20)
                .unwrap()
                .draw
                .as_ref()
                .unwrap()
                .world_motion
                .as_ref()
                .unwrap();
            assert_eq!(motion.keys[0].xyz, xyz);
            assert_eq!(motion.keys[7], Default::default());
            scene.advance_frame(100);
        }
        let draw = scene.object(20).unwrap().draw.as_ref().unwrap();
        assert_eq!(&draw.words[11..14], &xyz);
        assert!(draw.world_motion.is_none());
        assert_eq!(vm.stack().len_bytes(), 0);
        assert_eq!(vm.state().system_integer(0), Some(123));
    }
}
#[test]
fn motion_sequence_execution_errors_preserve_entry_and_target_but_missing_slots_are_skipped() {
    for (slot, mode, no_draw, succeeds) in [
        (20, 16, false, false),
        (256, 0, false, false),
        (21, 16, false, true),
        (20, 0, true, false),
    ] {
        let mut code = vec![];
        command(&mut code, 0x228a, &[slot, 0, 0, 0, mode, 100]);
        command(&mut code, 0x2282, &[]);
        let program = script(&code);
        let mut vm = Machine::new(&program).unwrap();
        let mut host = SequenceHost::default();
        let mut scene = motion_scene();
        if no_draw {
            let program = script(&{
                let mut c = vec![];
                command(&mut c, 0x2020, &[20]);
                c
            });
            let mut vm = Machine::new(&program).unwrap();
            pending(&mut vm);
            scene.try_handle(&mut vm).unwrap();
        }
        let before = scene.object(20).unwrap().clone();
        run(&mut vm, &mut host);
        run(&mut vm, &mut host);
        for _ in 0..2 {
            assert_eq!(
                host.advance_frame_with_images(16, &mut scene, &mut NoImages)
                    .is_ok(),
                succeeds
            );
            assert_eq!(host.header().index, u32::from(succeeds));
            assert_eq!(scene.object(20), Some(&before));
        }
    }
}
#[test]
fn motion_append_preserves_raw_bits_caps_at_sixteen_and_missing_args_are_transactional() {
    let mut code = vec![];
    for slot in 0..17 {
        command(
            &mut code,
            0x228a,
            &[slot, 0x7fc01234, 0x80000000, 0x7f800000, 0xffff, u32::MAX],
        );
    }
    command(&mut code, 0x228a, &[20, 0, 0, 0, 0]);
    let program = script(&code);
    let mut vm = Machine::new(&program).unwrap();
    let mut host = SequenceHost::default();
    for count in 1..=17 {
        run(&mut vm, &mut host);
        assert_eq!(host.header().count, count.min(16));
    }
    assert!(matches!(
        host.entries()[0],
        Some(cmvs_rs::sequence::SequenceEntry::WorldMotion {
            xyz: [0x7fc01234, 0x80000000, 0x7f800000],
            mode: 0xffff,
            duration: u32::MAX,
            ..
        })
    ));
    let header = host.header();
    let entries = host.entries().clone();
    pending(&mut vm);
    for _ in 0..2 {
        assert!(host.shared_sequence(&mut vm).is_err());
        assert_eq!(host.header(), header);
        assert_eq!(host.entries(), &entries);
        assert_eq!(vm.stack().len_bytes(), 20);
    }
}
