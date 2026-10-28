#[path = "support/script_validation.rs"]
mod script_validation;
#[allow(dead_code)]
#[path = "support/bytecode.rs"]
mod synthetic;

use cmvs_rs::{MAX_PAYLOAD, cpz::Cpz6Index, ps3::Script};
use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::Path,
};

#[test]
fn inventory_distinguishes_literal_opcode_bytes_from_instructions() {
    let script = synthetic::script(&[
        0x00, 0x02, 0x00, 0x01, 0xff, 0x27, 0x00, 0x20, 0x0f, 0x02, // integer literal
        0xff, 0x27, 0xff, 0x27, 0x00, 0x20,
    ]);
    let checked = script_validation::validate(&script).unwrap();
    assert_eq!(checked.code_bytes, 16);
    assert_eq!(checked.label_count, 1);
    assert_eq!(checked.instructions.len(), 4);
    assert_eq!(checked.system_commands.get(&0x27ff), Some(&2));
    assert_eq!(checked.system_commands.get(&0x2000), Some(&1));
    // A structural pass must not turn an unknown system command into runtime support.
    assert_eq!(checked.system_commands.len(), 2);
}

#[test]
fn structure_rejects_truncation_and_jump_into_operand() {
    for code in [&[0x00, 0x04][..], &[0x00, 0x04, 0x02, 0x00, 0x00, 0x00][..]] {
        assert!(script_validation::validate(&synthetic::script(code)).is_err());
    }
}

fn inspect(name: &str, raw: &[u8], output: &mut impl Write) -> Result<(), Box<dyn Error>> {
    let checked = script_validation::validate(&Script::decode(raw)?)?;
    writeln!(
        output,
        "script\t{name}\tok\t{}\t{}\t{}\t{}",
        checked.code_bytes,
        checked.instructions.len(),
        checked.label_count,
        checked.system_commands.len()
    )?;
    for (opcode, count) in checked.system_commands {
        writeln!(output, "system\t{name}\t0x{opcode:04x}\t{count}")?;
    }
    for instruction in checked.instructions {
        writeln!(
            output,
            "site\t{name}\t{:x}\t0x{:04x}",
            instruction.pc, instruction.opcode
        )?;
    }
    Ok(())
}

fn record(
    name: &str,
    raw: Result<Vec<u8>, Box<dyn Error>>,
    output: &mut impl Write,
    failed: &mut usize,
) -> io::Result<()> {
    if let Err(error) = raw.and_then(|raw| inspect(name, &raw, output)) {
        *failed += 1;
        eprintln!("Script structure failure {name}: {error}");
        writeln!(
            output,
            "failure\t{name}\t{}",
            error.to_string().escape_debug()
        )?;
    }
    Ok(())
}

#[test]
fn failed_sample_does_not_hide_a_later_valid_sample() {
    let mut output = Vec::new();
    let mut failed = 0;
    record("bad", Ok(vec![]), &mut output, &mut failed).unwrap();
    // Synthetic uncompressed PS2A, no labels, one yield instruction.
    let mut raw = b"PS2A".to_vec();
    for field in [48u32, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0] {
        raw.extend(field.to_le_bytes());
    }
    raw.extend([124, 188]);
    record("good", Ok(raw), &mut output, &mut failed).unwrap();
    assert_eq!(failed, 1);
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("failure\tbad\t"));
    assert!(output.contains("script\tgood\tok\t2\t1\t0\t1\n"));
    assert!(output.contains("site\tgood\t0\t0x2000\n"));
}

fn check_input(
    path: &Path,
    output: &mut impl Write,
    checked: &mut usize,
    failed: &mut usize,
) -> Result<(), Box<dyn Error>> {
    let mut file = File::open(path)?;
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cpz"))
    {
        let index = Cpz6Index::read(&mut file)?;
        for (i, entry) in index.entries().iter().enumerate() {
            let name = entry.name.escape_ascii().to_string();
            if !entry.name.to_ascii_lowercase().ends_with(b".ps3") {
                writeln!(output, "skip\t{name}\tnot-ps3")?;
                continue;
            }
            *checked += 1;
            record(
                &name,
                index.read_entry_verified(&mut file, i).map_err(Into::into),
                output,
                failed,
            )?;
        }
    } else {
        let mut raw = Vec::new();
        file.take(MAX_PAYLOAD as u64 + 49).read_to_end(&mut raw)?;
        if raw.len() > MAX_PAYLOAD + 48 {
            return Err("input exceeds research allocation limit".into());
        }
        *checked += 1;
        let name = format!("standalone:{}", path.to_string_lossy().escape_debug());
        record(&name, Ok(raw), output, failed)?;
    }
    Ok(())
}

/// External files are never fixtures and are never opened by the default test suite.
#[test]
#[ignore = "requires explicit CMVS_SCRIPT_TEST_INPUTS pointing to external read-only samples"]
fn external_script_structure() -> Result<(), Box<dyn Error>> {
    let inputs = std::env::var_os("CMVS_SCRIPT_TEST_INPUTS")
        .ok_or("set CMVS_SCRIPT_TEST_INPUTS with std::env::join_paths-compatible paths")?;
    let paths: Vec<_> = std::env::split_paths(&inputs).collect();
    if paths.is_empty() || paths.iter().any(|p| p.as_os_str().is_empty()) {
        return Err("at least one nonempty sample path is required".into());
    }
    let sink: Box<dyn Write> = match std::env::var_os("CMVS_SCRIPT_TEST_REPORT") {
        Some(path) => Box::new(OpenOptions::new().write(true).create_new(true).open(path)?),
        None => Box::new(io::sink()),
    };
    let mut output = BufWriter::new(sink);
    let (mut checked, mut failed) = (0, 0);
    for path in paths {
        if let Err(error) = check_input(&path, &mut output, &mut checked, &mut failed) {
            failed += 1;
            eprintln!("Script input failure {path:?}: {error}");
            writeln!(
                output,
                "failure\t{:?}\t{}",
                path,
                error.to_string().escape_debug()
            )?;
        }
    }
    writeln!(output, "summary\tchecked\t{checked}\tfailed\t{failed}")?;
    output.flush()?;
    eprintln!("Script structure: checked={checked}, failed={failed}; not execution coverage");
    if checked == 0 || failed != 0 {
        return Err("script structure checks failed".into());
    }
    Ok(())
}
