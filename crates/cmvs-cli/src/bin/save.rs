use cmvs_core::{
    MAX_PAYLOAD, format::save::Csv2Layout, format::save::Csv2Save, format::save::HEADER_SIZE,
    format::snapshot,
};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
};

const USAGE: &str = "\
Usage:
  cmvs-save <save.dat> [--records | --vm-state | --visual-state]
  cmvs-save --section <0|1|2> <save.dat> <new-file>

Decode a CMVS CSV2 gameplay save and print its header and section sizes.

Options:
  --records       list snapshot records
  --vm-state      inspect VM state; PC validation is limited to the main script
  --visual-state  inspect supported text, window and scene records
  --section       export one section to a new file
  -h, --help      show this help

Section 0 is the snapshot and carries the additive checksum. Section 1 is
auxiliary data; section 2 is the script memory image.

Example: cmvs-save --section 0 save/save000.dat snapshot.bin";

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let raw_args: Vec<_> = env::args_os().skip(1).collect();
    if cmvs_cli::wants_help(&raw_args) {
        println!("{USAGE}");
        return Ok(());
    }
    let mut records = false;
    let mut vm_state = false;
    let mut visual_state = false;
    let mut section = None;
    let mut positionals: Vec<std::ffi::OsString> = Vec::new();
    let mut positional_only = false;
    let mut iter = raw_args.into_iter();
    while let Some(arg) = iter.next() {
        if !positional_only && arg == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && arg.to_string_lossy().starts_with('-') {
            let has_mode = records || vm_state || visual_state || section.is_some();
            if arg == "--records" {
                if has_mode {
                    return Err("wrong argument count; use --help".into());
                }
                records = true;
            } else if arg == "--vm-state" {
                if has_mode {
                    return Err("wrong argument count; use --help".into());
                }
                vm_state = true;
            } else if arg == "--visual-state" {
                if has_mode {
                    return Err("wrong argument count; use --help".into());
                }
                visual_state = true;
            } else if arg == "--section" {
                if has_mode {
                    return Err("wrong argument count; use --help".into());
                }
                let index = iter
                    .next()
                    .ok_or("wrong argument count; use --help")?
                    .to_str()
                    .ok_or("section must be 0, 1 or 2")?
                    .parse::<usize>()?;
                if index > 2 {
                    return Err("section must be 0, 1 or 2".into());
                }
                section = Some(index);
            } else {
                return Err(format!("unknown option {}", arg.to_string_lossy()).into());
            }
            continue;
        }
        positionals.push(arg);
    }
    let expected_positionals = if section.is_some() { 2 } else { 1 };
    if positionals.len() != expected_positionals {
        return Err("wrong argument count; use --help".into());
    }
    let input = &positionals[0];
    let section_output = positionals.get(1);
    let mut file = File::open(input)?;
    let length = file.metadata()?.len();
    let mut header = [0; HEADER_SIZE];
    file.read_exact(&mut header)?;
    let layout = Csv2Layout::parse(&header, length)?;
    let mut raw = header.to_vec();
    file.take(MAX_PAYLOAD as u64 + 1 - HEADER_SIZE as u64)
        .read_to_end(&mut raw)?;
    if raw.len() as u64 != length {
        return Err("CSV2 file changed length while reading".into());
    }
    let save = Csv2Save::decode(&raw)?;
    println!(
        "CSV2 label={} key=0x{:08x}",
        layout.header.label.escape_ascii(),
        layout.header.ordering_key
    );
    for (i, range) in layout.sections.iter().enumerate() {
        println!(
            "section[{i}] offset={} packed={} decoded={}",
            range.start,
            range.len(),
            layout.decoded_sizes[i]
        );
    }
    println!(
        "# snapshot additive checksum verified; other sections are not checksummed; no engine restoration"
    );
    if records {
        let list = snapshot::parse_records(&save.snapshot)?;
        println!("snapshot records={}", list.len());
        for record in list {
            println!(
                "record offset={} tag=0x{:03x} index={:?} len={}",
                record.offset,
                record.tag,
                record.index,
                record.body.len()
            );
        }
    }
    if visual_state {
        let visual = snapshot::VisualSnapshot::decode(&save)?;
        println!(
            "visual clear={:?} texts={} windows={} objects={}",
            visual.clear(),
            visual.texts().iter().flatten().count(),
            visual.windows().iter().flatten().count(),
            visual.objects().len()
        );
        for (&slot, scene) in visual.objects() {
            println!(
                "scene[{slot}] shown={} children={} image={}",
                scene.shown,
                scene.children.len(),
                scene
                    .image
                    .as_ref()
                    .map_or_else(|| "<none>".into(), |i| i.name.escape_ascii().to_string())
            );
        }
        for (slot, window) in visual.windows().iter().enumerate() {
            if let Some(window) = window {
                println!(
                    "window[{slot}] children={} button_words={:?} switch_state={}",
                    window.scene.children.len(),
                    window.button_words,
                    window.switch_state
                );
            }
        }
        println!(
            "# visual subset only; {} other records uninterpreted; resources not loaded; no engine restoration",
            visual.uninterpreted.len()
        );
    }
    if vm_state {
        let vm = snapshot::VmSnapshot::decode(&save)?;
        println!(
            "vm slot={} script={} pc=0x{:x} pc_verified={} repeat={} sp={} depth={}",
            vm.active_slot,
            vm.script_names[usize::from(vm.active_slot)].escape_ascii(),
            vm.pc,
            vm.active_pc_verified,
            vm.repeat_count,
            vm.stack_pointer,
            vm.frame_depth
        );
        println!(
            "locals integers={} floats={} bits={} strings={} saved_system_results={}",
            vm.variables.integers().len(),
            vm.variables.float_bits().len(),
            vm.variables.packed_bits().len() * 8,
            vm.variables.strings().len(),
            vm.system_results.len()
        );
        println!(
            "# uninterpreted records={} (not restored)",
            vm.uninterpreted.len()
        );
        for record in &vm.uninterpreted {
            println!(
                "uninterpreted tag=0x{:03x} index={:?} len={}",
                record.tag,
                record.index,
                record.body.len()
            );
        }
        if !vm.active_pc_verified {
            println!("# active common script is not supplied; its PC has not been validated");
        }
    }
    if let Some(index) = section {
        let out_path = section_output.expect("validated positionals length");
        let bytes = [&save.snapshot, &save.auxiliary, &save.script_image][index];
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out_path)?;
        output.write_all(bytes)?;
        println!("Wrote {} decoded bytes to {out_path:?}", bytes.len());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-save --help' for usage.");
        std::process::exit(1);
    }
}
