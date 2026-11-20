use cmvs_assembler::script::{Encoding, import_text};
use cmvs_core::format::ps3::Script;
use cmvs_disassembler::export_text;
use std::collections::BTreeMap;

fn fixture(code: &[u8], text: &[u8]) -> Script {
    let data = b"\x13\x37key\0";
    let mut bytes = b"PSS0".to_vec();
    for word in [
        48,
        0,
        0,
        1,
        code.len() as u32,
        data.len() as u32,
        text.len() as u32,
        0,
        0,
        (4 + code.len() + data.len() + text.len()) as u32,
        0,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(code);
    bytes.extend(data);
    bytes.extend(text);
    Script::decode(&bytes).unwrap()
}

fn table(script: &Script) -> String {
    let mut out = Vec::new();
    export_text(script, Encoding::ShiftJis, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn translate(table: &str, edits: &[(&str, &str)]) -> String {
    let edits: BTreeMap<_, _> = edits.iter().copied().collect();
    table
        .lines()
        .map(|row| match edits.get(row) {
            Some(value) => format!("{row}\t{value}\n"),
            None => format!("{row}\n"),
        })
        .collect()
}

#[test]
fn exported_rows_bind_to_the_script_and_relocate_shared_suffixes() {
    let mut code = Vec::new();
    for (tag, offset) in [(0x120u16, 2u32), (0x120, 7), (0x125, 2)] {
        code.extend(0x201u16.to_le_bytes());
        code.extend(tag.to_le_bytes());
        code.extend(offset.to_le_bytes());
        code.extend(0x20fu16.to_le_bytes());
    }
    let script = fixture(&code, b"prefix\0next\0\x81\0");
    let original = table(&script);
    assert!(original.contains("text\tc\t\\x81\n"));
    assert_eq!(
        import_text(&script, original.as_bytes(), Encoding::ShiftJis)
            .unwrap()
            .payload(),
        script.payload()
    );

    let parent = ("text\t0\tprefix", "longer");
    let ambiguous = translate(&original, &[parent]);
    let err = import_text(&script, ambiguous.as_bytes(), Encoding::ShiftJis).unwrap_err();
    assert!(err.to_string().contains("shared suffix text+0x2"));
    let edited = translate(
        &original,
        &[parent, ("text\t2\tefix", "alias"), ("data\t2\tkey", "new")],
    );
    let out = import_text(&script, edited.as_bytes(), Encoding::ShiftJis).unwrap();
    assert_eq!(out.text(), b"longer\0alias\0next\0\x81\0");
    assert_eq!(out.data(), b"\x13\x37new\0");
    let mut expected = code.clone();
    expected[4..8].copy_from_slice(&7u32.to_le_bytes());
    expected[14..18].copy_from_slice(&13u32.to_le_bytes());
    assert_eq!(out.code(), expected);
    assert_eq!(out.labels(), script.labels());
    let other = fixture(&code, b"PREFIX\0next\0\x81\0");
    assert!(
        import_text(&other, edited.as_bytes(), Encoding::ShiftJis)
            .unwrap_err()
            .to_string()
            .contains("SHA-256")
    );
}

#[test]
fn target_byte_limit_applies_to_concatenation_and_not_to_plain_references() {
    let mut code = vec![1, 2, 0x20, 1, 0, 0, 0, 0];
    let mut plain = code.clone();
    plain.extend(0x20fu16.to_le_bytes());
    code.extend([0x20, 1, 2, 0, 0, 0]);
    code.extend(0x160u16.to_le_bytes());
    code.extend(0x20fu16.to_le_bytes());
    let concat = fixture(&code, b"x\0y\0");
    let translated = "字".repeat(341);
    let edited = translate(&table(&concat), &[("text\t0\tx", &translated)]);
    assert!(
        import_text(&concat, edited.as_bytes(), Encoding::Utf8)
            .unwrap_err()
            .to_string()
            .contains("1023")
    );
    let standalone = fixture(&plain, b"x\0y\0");
    let translated = translated + "a";
    let edited = translate(&table(&standalone), &[("text\t0\tx", &translated)]);
    assert!(
        import_text(&standalone, edited.as_bytes(), Encoding::Utf8)
            .unwrap()
            .text()
            .starts_with(translated.as_bytes())
    );
}
