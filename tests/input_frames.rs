use cmvs_rs::input::{ActionInput, CodeInput, KeyBindings};

#[test]
fn bindings_are_bounded_and_only_the_first_bank_is_loaded() {
    let b = KeyBindings::parse("[key_function_01]\nkey_data_01=22\nKEY_DATA_02=128\nKEY_DATA_03=-1\n[KEY_FUNCTION_51]\nKEY_DATA_01=3\n[KEY_FUNCTION_24]\nKEY_DATA_05=127").unwrap();
    assert_eq!(b.codes()[0], [22, 0, 0, 0, 0]);
    assert_eq!(b.codes()[23], [0, 0, 0, 0, 127]);
    assert!(KeyBindings::parse("[KEY_FUNCTION_01]\nKEY_DATA_01=oops").is_err());
    assert!(KeyBindings::parse(&" ".repeat(65537)).is_err());
}

#[test]
fn individual_binding_edges_survive_consumption_and_other_held_bindings() {
    let b = KeyBindings::parse("[KEY_FUNCTION_01]\nKEY_DATA_01=1\nKEY_DATA_02=22").unwrap();
    let mut history = CodeInput::default();
    let mut input = ActionInput::default();
    let mut codes = [false; 128];
    codes[1] = true;
    history.update(&b, &codes, &mut input);
    assert!(input.pressed(0) && input.held(0) && !input.released(0));
    input.consume(0);
    assert!(!input.pressed(0) && input.held(0));
    history.update(&b, &codes, &mut input);
    assert!(!input.pressed(0));
    codes[22] = true;
    history.update(&b, &codes, &mut input);
    assert!(input.pressed(0));
    codes[1] = false;
    history.update(&b, &codes, &mut input);
    assert!(input.released(0) && input.held(0));
    history.clear(&mut input);
    history.update(&b, &[false; 128], &mut input);
    assert!(!input.released(0) && !input.held(0));
}
