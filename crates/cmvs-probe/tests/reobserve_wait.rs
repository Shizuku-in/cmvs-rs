use cmvs_core::runner::load_start;
use cmvs_probe::{
    observe::PageSignature,
    scenario::{Event, Scenario},
    session::{Options, Session},
    target::Target,
};

#[test]
fn observing_a_message_wait_twice_does_not_require_an_input_edge() {
    let root = std::env::temp_dir().join(format!(
        "probe-reobserve-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("data/pack")).unwrap();
    let mut code = Vec::new();
    for (op, args) in [
        (0x2140u16, vec![0u32, 0, 0, 0, 800, 600]),
        (0x2153, vec![0, 0, u32::MAX]),
        (0x2153, vec![0, 0, u32::MAX]),
    ] {
        for arg in args.into_iter().rev() {
            code.extend(0x440u16.to_le_bytes());
            code.extend(4u16.to_le_bytes());
            code.extend(arg.to_le_bytes());
        }
        code.extend(op.to_le_bytes());
    }
    let mut raw = b"PS2A".to_vec();
    for word in [
        48,
        0,
        0,
        0,
        code.len() as u32,
        0,
        0,
        0,
        0,
        code.len() as u32,
        0,
    ] {
        raw.extend(word.to_le_bytes());
    }
    raw.extend(code.iter().map(|b| b.rotate_left(1).wrapping_add(124)));
    std::fs::write(root.join("data/pack/start.ps3"), raw).unwrap();
    let archive: Vec<u8> = include_str!("../../../tests/fixtures/cpz6-script.hex")
        .trim()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect();
    std::fs::write(root.join("data/pack/script.cpz"), archive).unwrap();
    std::fs::write(root.join("key.cfg"), "[KEY_FUNCTION_01]\nKEY_DATA_01=1\n").unwrap();
    let target = Target::open(&root, Some("cmvs-20160201"));
    let script = load_start(&root, &target.game.as_ref().unwrap().files).unwrap();
    let user = root.with_extension("user");
    std::fs::create_dir(&user).unwrap();
    let options = Options {
        capture_snapshots: false,
        max_steps: 10000,
        ..Default::default()
    };
    let mut session = Session::start(&target, &script, &user, &options, None).unwrap();
    let start = PageSignature::of(session.startup());
    let mut scenario = Scenario::resume(&mut session, start, false);
    let Some(Event::Wait(first)) = scenario.next_within(&mut session, 8).unwrap() else {
        panic!("first message wait missing")
    };
    let Some(Event::Wait(second)) = scenario.next_within(&mut session, 8).unwrap() else {
        panic!("re-observed wait missing without an advance")
    };
    assert_eq!((first.script, first.pc), (second.script, second.pc));
    assert!(second.repeat > first.repeat);
    scenario.advance(&mut session).unwrap();
    let Some(Event::Wait(third)) = scenario.next_within(&mut session, 8).unwrap() else {
        panic!("next message wait missing after an advance")
    };
    assert_ne!(
        second.pc, third.pc,
        "released input must be consumed before reporting another wait"
    );
    drop(session);
    std::fs::remove_dir_all(user).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
