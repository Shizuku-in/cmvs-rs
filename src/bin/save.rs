use cmvs_rs::{
    MAX_PAYLOAD,
    save::{Csv2Layout, Csv2Save, HEADER_SIZE},
    snapshot,
};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
};

const USAGE: &str = "\
Usage:
  cmvs-save <save.dat>                                      print the header and sections
  cmvs-save --section <0|1|2> <save.dat> <new-output-file>  export one decoded section
  cmvs-save --records <save.dat>                            list the snapshot record framing
  cmvs-save --vm-state <save.dat>                           validate the typed VM fields
  cmvs-save --visual-state <save.dat>                       validate text, windows and scene

Sections: 0 snapshot, 1 auxiliary, 2 script memory image. Only section 0 has the
original additive checksum, which does not authenticate the whole file.
--visual-state covers the supported subset and reads no resources.
Read only: these commands never install engine state.";

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{USAGE}");
        return Ok(());
    }
    let records = args.len() == 2 && args[0] == "--records";
    let vm_state = args.len() == 2 && args[0] == "--vm-state";
    let visual_state = args.len() == 2 && args[0] == "--visual-state";
    let section = if args.len() == 4 && args[0] == "--section" {
        let index = args[1]
            .to_str()
            .ok_or("section must be 0, 1 or 2")?
            .parse::<usize>()?;
        if index > 2 {
            return Err("section must be 0, 1 or 2".into());
        }
        Some(index)
    } else if records || vm_state || visual_state || (args.len() == 1 && args[0] != "--section") {
        None
    } else {
        return Err("wrong argument count; use --help".into());
    };
    let input = if section.is_some() {
        &args[2]
    } else if records || vm_state || visual_state {
        &args[1]
    } else {
        &args[0]
    };
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
        let bytes = [&save.snapshot, &save.auxiliary, &save.script_image][index];
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[3])?;
        output.write_all(bytes)?;
        println!("Wrote {} decoded bytes to {:?}", bytes.len(), args[3]);
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
