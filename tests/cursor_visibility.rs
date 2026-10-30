use cmvs_rs::cursor::CursorVisibility;

#[test]
fn movement_deadlines_and_disabled_or_inactive_state_follow_engine_policy() {
    for mode in 1..=4 {
        let mut cursor = CursorVisibility::default();
        assert!(cursor.update(100, mode, true, [10, 20]));
        assert!(cursor.update(99 + mode * 5000, mode, true, [10, 20]));
        assert!(!cursor.update(100 + mode * 5000, mode, true, [10, 20]));
        assert!(!cursor.update(200 + mode * 5000, 0, true, [10, 20]));
        assert!(!cursor.update(200 + mode * 5000, mode, false, [11, 20]));
        assert!(cursor.update(201 + mode * 5000, mode, true, [11, 20]));
    }
    let mut cursor = CursorVisibility::default();
    assert!(!cursor.update(30_000, 1, true, [0, 0]));
    // Disabled mode retains hidden state even when the pointer moves.
    assert!(!cursor.update(30_001, 0, true, [1, 0]));
    // A mode change uses the old movement time, not a fresh delay.
    assert!(cursor.update(30_002, 4, true, [1, 0]));
    assert!(!cursor.update(u32::MAX - 1, 1, true, [2, 0]));
    assert!(cursor.update(0, 1, true, [2, 0]));
}
