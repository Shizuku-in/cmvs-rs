use cmvs_rs::{
    save_menu::profile_int,
    settings::{Configuration, DisplayPreferences, MAX_CONFIGURATION},
};

#[test]
fn ini_updates_preserve_unknown_bytes_and_comments_and_replace_duplicate_keys() {
    let raw = b"; keep\r\n[MESSAGE_INFO]\r\ntext_wait = 2\r\nUNKNOWN=\x82\xa0\r\nTEXT_WAIT=3\r\n[Other]\nX=raw\n[message_info]\nTEXT_WAIT=4\n";
    let mut configuration = Configuration::new(raw.to_vec()).unwrap();
    configuration
        .set_integer("MESSAGE_INFO", "TEXT_WAIT", u32::MAX)
        .unwrap();
    assert_eq!(
        profile_int(configuration.bytes(), "MESSAGE_INFO", "TEXT_WAIT", 0),
        -1
    );
    assert!(
        configuration
            .bytes()
            .windows(b"UNKNOWN=\x82\xa0\r\n".len())
            .any(|w| w == b"UNKNOWN=\x82\xa0\r\n")
    );
    assert!(
        configuration
            .bytes()
            .windows(b"[Other]\nX=raw\n".len())
            .any(|w| w == b"[Other]\nX=raw\n")
    );
    assert_eq!(
        configuration
            .bytes()
            .windows(b"TEXT_WAIT=-1".len())
            .filter(|w| *w == b"TEXT_WAIT=-1")
            .count(),
        3
    );
    assert!(configuration.bytes().starts_with(b"; keep\r\n"));
}

#[test]
fn missing_keys_are_inserted_inside_their_section_and_missing_sections_appended() {
    let mut configuration = Configuration::new(b"[S]\nX=1\n[T]\nY=2".to_vec()).unwrap();
    configuration.set_integer("S", "NEW", 9).unwrap();
    assert_eq!(configuration.bytes(), b"[S]\nX=1\nNEW=9\r\n[T]\nY=2");
    configuration.set_integer("U", "Z", 7).unwrap();
    assert!(configuration.bytes().ends_with(b"Y=2\r\n[U]\r\nZ=7\r\n"));
    assert_eq!(profile_int(configuration.bytes(), "U", "Z", 0), 7);
    let mut empty = Configuration::new(vec![]).unwrap();
    empty.set_integer("S", "X", 1).unwrap();
    assert_eq!(empty.bytes(), b"[S]\r\nX=1\r\n");
    assert!(Configuration::new(vec![0]).is_err());
    assert!(Configuration::new(vec![b'x'; MAX_CONFIGURATION + 1]).is_err());
    let mut full = Configuration::new(vec![b'x'; MAX_CONFIGURATION]).unwrap();
    let before = full.clone();
    assert!(full.set_integer("S", "X", 1).is_err());
    assert_eq!(full, before);
    assert!(full.set_integer("bad\nsection", "key", 1).is_err());
}

#[test]
fn desktop_preferences_normalize_only_the_supported_aspect_range() {
    assert_eq!(
        DisplayPreferences::from_ini(b""),
        DisplayPreferences::default()
    );
    assert_eq!(
        DisplayPreferences::from_ini(b"[CMVS_RS_DESKTOP]\nFULLSCREEN=1\nASPECT_MODE=2\n"),
        DisplayPreferences {
            fullscreen: true,
            aspect_mode: 2,
            aspect_scaling: true,
            render_mode: 1
        }
    );
    assert_eq!(
        DisplayPreferences::from_ini(b"[CMVS_RS_DESKTOP]\nASPECT_MODE=-1\n").aspect_mode,
        1
    );
}

#[test]
fn desktop_scaling_preference_is_independent_and_nonzero_enables_it() {
    for (word, enabled) in [("0", false), ("1", true), ("2", true), ("-1", true)] {
        let ini = format!("[CMVS_RS_DESKTOP]\nASPECT_SCALING={word}\nASPECT_MODE=3\n");
        let prefs = DisplayPreferences::from_ini(ini.as_bytes());
        assert_eq!(prefs.aspect_scaling, enabled);
        assert_eq!(prefs.aspect_mode, 3);
    }
}

#[test]
fn graphic_filter_preference_swaps_point_and_linear_like_the_original_word() {
    let mode = |raw: &str| {
        let text = format!("[GRAPHIC_CONFIG]\r\nASPECT_RATIO_FILTER={raw}\r\n");
        DisplayPreferences::from_ini(text.as_bytes()).render_mode
    };
    for (raw, expected) in [("0", 1), ("1", 0), ("2", 2), ("3", 3), ("4", 0), ("-1", 0)] {
        assert_eq!(mode(raw), expected, "raw={raw}");
    }
    assert_eq!(DisplayPreferences::default().render_mode, 1);
}
