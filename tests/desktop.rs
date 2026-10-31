#![cfg(feature = "desktop")]
use cmvs_rs::{
    cursor::CursorBitmap,
    desktop::{DesktopInput, InputEvent, TextEncoding, cursor_rgba, decode_icon},
};

#[path = "support/bytecode.rs"]
mod bytecode;

#[test]
#[ignore = "requires CMVS_FONT_PATH; synthetic scene and external font, no game data"]
fn immediate_device_readback_keeps_the_completed_font_composite() {
    use cmvs_rs::{
        camera::CameraHost, capture::ScreenCapture, desktop::font::FontRenderer, machine::Machine,
        message::MessageHost, message_flags::MessageFlags, platform::RecordingBackend,
        scene::SceneHost,
    };
    let path = std::env::var_os("CMVS_FONT_PATH").expect("CMVS_FONT_PATH");
    let mut font = FontRenderer::read(std::path::Path::new(&path), 0).unwrap();
    let mut code = vec![];
    bytecode::command(&mut code, 0x2020, &[3]);
    bytecode::command(&mut code, 0x215d, &[0, 0, 100, 100, 100, 100, 1280, 720]);
    bytecode::command(&mut code, 0x2159, &[0, 0, 0x8000_0000, 0]);
    bytecode::command(&mut code, 0x203e, &[3, u32::MAX]);
    let script = bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"\x82\xa0").unwrap();
    let mut scene = SceneHost::default();
    let mut messages = MessageHost::default();
    bytecode::pending(&mut vm);
    scene.try_handle(&mut vm).unwrap();
    bytecode::pending(&mut vm);
    messages.try_handle(&mut vm).unwrap();
    messages.set_record_output(true);
    bytecode::pending(&mut vm);
    messages
        .try_record_poll(&mut vm, MessageFlags::default())
        .unwrap();
    messages.advance_frame(10_000);
    let camera = CameraHost::default();
    let frame = font.render(&scene, &camera, &messages).unwrap();
    let windows: Vec<_> = (0..cmvs_rs::message::MESSAGE_WINDOWS)
        .filter_map(|i| messages.window(i))
        .filter(|w| w.visible && !w.hidden)
        .map(|w| &w.object)
        .collect();
    let without_glyphs =
        cmvs_rs::scene_render::render_scene(&scene, Some(&camera), &windows, 1280, 720).unwrap();
    assert_ne!(frame, without_glyphs);
    assert!(frame.pixels().chunks_exact(4).any(|p| p[..3] != [0; 3]));
    let mut backend = RecordingBackend::default();
    backend.present_frame(frame.clone());
    bytecode::pending(&mut vm);
    ScreenCapture::default()
        .try_immediate(&mut vm, &mut scene, &mut backend)
        .unwrap();
    assert_eq!(
        scene.object(3).unwrap().image.as_ref().unwrap().pixels,
        frame
    );
}

#[test]
#[ignore = "requires CMVS_FONT_PATH; synthetic message and external font, no game data"]
fn horizontal_glyph_reveal_crops_the_full_font_image_without_rescaling() {
    use cmvs_rs::{
        camera::CameraHost, desktop::font::FontRenderer, machine::Machine, message::MessageHost,
        message_flags::MessageFlags, scene::SceneHost, settings::TextOutputSettings,
    };
    let font_path = std::env::var_os("CMVS_FONT_PATH").expect("CMVS_FONT_PATH");
    let mut font = FontRenderer::read(std::path::Path::new(&font_path), 0).unwrap();
    let mut code = vec![];
    bytecode::command(&mut code, 0x215d, &[0, 0, 100, 100, 100, 100, 1280, 720]);
    bytecode::command(&mut code, 0x2159, &[0, 0, 0x8000_0000, 0]);
    let script = bytecode::script(&code);
    let mut vm = Machine::new(&script).unwrap();
    vm.state_mut().set_string(0, b"\x82\xa0").unwrap();
    let mut messages = MessageHost::default();
    bytecode::pending(&mut vm);
    assert!(messages.try_handle(&mut vm).unwrap());
    messages.set_record_output(true);
    messages.set_text_settings(TextOutputSettings {
        bold: false,
        speed: 1,
        outline: 0,
        reveal_mode: 2,
        reveal_duration: 1,
    });
    bytecode::pending(&mut vm);
    messages
        .try_record_poll(&mut vm, MessageFlags::default())
        .unwrap();
    let scene = SceneHost::default();
    let camera = CameraHost::default();
    let blank = font.render(&scene, &camera, &messages).unwrap();
    messages.advance_frame(100);
    let partial = font.render(&scene, &camera, &messages).unwrap();
    let crop_end = 98 + messages.text(0).unwrap().glyphs[0].reveal_width as usize;
    messages.advance_frame(100);
    let full = font.render(&scene, &camera, &messages).unwrap();
    assert_ne!(partial.pixels(), full.pixels());
    assert_ne!(partial.pixels(), blank.pixels());
    let mut revealed_later = false;
    for (i, pixel) in partial.pixels().chunks_exact(4).enumerate() {
        let expected = if i % 1280 < crop_end {
            &full.pixels()[i * 4..i * 4 + 4]
        } else {
            &blank.pixels()[i * 4..i * 4 + 4]
        };
        assert_eq!(pixel, expected);
        revealed_later |= i % 1280 >= crop_end
            && full.pixels()[i * 4..i * 4 + 4] != blank.pixels()[i * 4..i * 4 + 4];
    }
    assert!(revealed_later);
}

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn icon() -> Vec<u8> {
    let mut b = vec![0; 74];
    b[2] = 1;
    b[4] = 1;
    b[6] = 2;
    b[7] = 1;
    b[10] = 1;
    b[12] = 32;
    put(&mut b, 14, 52);
    put(&mut b, 18, 22);
    put(&mut b, 22, 40);
    put(&mut b, 26, 2);
    put(&mut b, 30, 2);
    b[34] = 1;
    b[36] = 32;
    b[62..70].copy_from_slice(&[30, 20, 10, 255, 60, 50, 40, 128]);
    b
}
fn cursor(rgb: [u8; 3], mask: bool) -> CursorBitmap {
    let mut b = vec![0; 74];
    b[2] = 2;
    b[4] = 1;
    b[6] = 1;
    b[7] = 1;
    b[8] = 1;
    put(&mut b, 14, 52);
    put(&mut b, 18, 22);
    put(&mut b, 22, 40);
    put(&mut b, 26, 1);
    put(&mut b, 30, 2);
    b[34] = 1;
    b[36] = 8;
    put(&mut b, 54, 1);
    b[62..66].copy_from_slice(&[rgb[2], rgb[1], rgb[0], 0]);
    if mask {
        b[70] = 128;
    }
    CursorBitmap::decode(&b).unwrap()
}
#[test]
fn title_encoding_is_explicit_and_invalid_bytes_are_not_replaced() {
    assert_eq!(
        TextEncoding::ShiftJis
            .decode(&[0x83, 0x41, 0x83, 0x7d, 0x83, 0x63, 0x83, 0x63, 0x83, 0x7e])
            .unwrap(),
        "アマツツミ"
    );
    assert_eq!(
        TextEncoding::Gbk.decode(&[0xd6, 0xd0, 0xce, 0xc4]).unwrap(),
        "中文"
    );
    assert_eq!(
        TextEncoding::Utf8.decode("标题".as_bytes()).unwrap(),
        "标题"
    );
    for (encoding, bytes) in [
        (TextEncoding::ShiftJis, &[0x82][..]),
        (TextEncoding::Gbk, &[0x81][..]),
        (TextEncoding::Utf8, &[0xff][..]),
    ] {
        assert!(encoding.decode(bytes).is_err());
    }
    assert!(TextEncoding::parse("automatic").is_err());
}
#[test]
fn icon_bgra_and_alpha_decode_with_embedded_dimension_preflight() {
    let image = decode_icon(&icon()).unwrap();
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.rgba, [10, 20, 30, 255, 40, 50, 60, 128]);
    for (at, value) in [(4, 0), (18, u32::MAX), (26, u32::MAX), (30, u32::MAX)] {
        let mut b = icon();
        put(&mut b, at, value);
        assert!(decode_icon(&b).is_err());
    }
    for end in 0..74 {
        assert!(decode_icon(&icon()[..end]).is_err());
    }
}
#[test]
fn native_cursor_preserves_transparency_and_rejects_inverting_pixels() {
    assert_eq!(
        cursor_rgba(&cursor([1, 2, 3], false)).unwrap(),
        [1, 2, 3, 255]
    );
    assert_eq!(cursor_rgba(&cursor([0, 0, 0], true)).unwrap(), [0, 0, 0, 0]);
    assert!(cursor_rgba(&cursor([255, 255, 255], true)).is_err());
}
#[test]
fn focus_loss_clears_held_keys_and_buttons_and_history_is_bounded() {
    let mut input = DesktopInput::default();
    input.push(InputEvent::Key {
        physical: "KeyA".into(),
        pressed: true,
        repeat: false,
    });
    input.push(InputEvent::Button {
        button: "Left".into(),
        pressed: true,
    });
    assert!(input.held_keys().contains("KeyA"));
    assert!(input.held_buttons().contains("Left"));
    input.push(InputEvent::Focus(false));
    assert!(input.held_keys().is_empty());
    assert!(input.held_buttons().is_empty());
    for i in 0..300 {
        input.push(InputEvent::Pointer {
            x: f64::from(i),
            y: 0.0,
        });
    }
    assert_eq!(input.events().len(), 256);
    assert_eq!(input.dropped(), 47);
    assert_eq!(input.counts().pointer, 300);
    assert_eq!(input.counts().keys, 1);
    assert_eq!(input.counts().buttons, 1);
    assert_eq!(input.counts().focus, 1);
}
#[test]
fn key_repeat_and_release_do_not_leave_stuck_host_state() {
    let mut input = DesktopInput::default();
    for repeat in [false, true] {
        input.push(InputEvent::Key {
            physical: "KeyA".into(),
            pressed: true,
            repeat,
        });
    }
    assert_eq!(input.held_keys().len(), 1);
    input.push(InputEvent::Key {
        physical: "KeyA".into(),
        pressed: false,
        repeat: false,
    });
    assert!(input.held_keys().is_empty());
    input.push(InputEvent::Button {
        button: "Right".into(),
        pressed: true,
    });
    input.push(InputEvent::Button {
        button: "Right".into(),
        pressed: false,
    });
    assert!(input.held_buttons().is_empty());
}
