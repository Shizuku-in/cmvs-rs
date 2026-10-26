use cmvs_rs::{
    MAX_PAYLOAD,
    audio::{AudioHost, AudioRequest, RecordingAudio},
    bootstrap::Bootstrap,
    cpz::Cpz6Index,
    gesture::PointerGesture,
    image::ImageArchives,
    machine::{Machine, Step},
    platform::{DirectoryAssets, PlatformHost, PlatformRequest, RecordingBackend},
    ps3::Script,
    random::CrtRandom,
    save_menu::{DirectorySaves, SaveMenuHost, SlotStatus},
    scene::SceneHost,
    script_control::{ArchiveScripts, ControlEvent, ScriptControl},
    snapshot::{SaveImage, SaveTitle},
    system_save::{FileSystemSave, SystemSaveHost},
    text,
};
use std::{collections::BTreeMap, env, error::Error, fs::File, io::Read};

fn number(raw: &std::ffi::OsStr) -> Result<u32, Box<dyn Error>> {
    let text = raw.to_str().ok_or("expected numeric argument")?;
    Ok(if let Some(hex) = text.strip_prefix("0x") {
        u32::from_str_radix(hex, 16)?
    } else {
        text.parse()?
    })
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: cmvs-trace <script-or-archive> [--script <ASCII-basename>] [--entry <pc>] [--steps <budget>] [--bootstrap]")?;
    if path == "--help" {
        println!(
            "Usage: cmvs-trace <script-or-archive> [--script <ASCII-basename>] [--entry <pc>] [--steps <budget>] [--bootstrap] [--script-archive <archive>] [--virtual-tick <tick>] [--frame-ms <delta>] [--save-dir <directory>] [--system-save <file>] [--record-platform <game-root>] [--continue-without-snapshot] [--rand-seed <seed>]\n--frame-ms supplies a positive engine-frame delta on each supported wait/yield; without it time stays frozen. Callback ticks remain separate. --script loads an isolated initial script. --script-archive supplies common/main scripts and enables explicit control handling with virtual time (default tick 0), without automatic scheduling. --bootstrap applies verified startup configuration. --save-dir enables read-only saveNNN.dat header scanning and menu state, without native UI or save restoration. --system-save explicitly enables CSS1 persistent-state loading for 0x2128; it does not restore a gameplay snapshot. --record-platform enables a recording-only backend with read-only cursor/icon assets from the explicit game root, plus verified state such as the clear color, the save title and pointer-gesture configuration, records SE playback requests without audio output, and decodes 0x2030 object images read-only from the 0x2011 entries; no native window, drawing, or input polling. Main transfer stops at the engine save snapshot unless --continue-without-snapshot is given; that applies only the verified live-state effect, builds no save image, and leaves saving unavailable. --rand-seed enables 0x20d4 with the CRT rand() generator from an explicit seed; the original seeds from timeGetTime and draws every frame, so values are not reproducible."
        );
        return Ok(());
    }
    let mut entry = None;
    let mut budget = 10000u32;
    let mut bootstrap = None;
    let mut archive_script = None;
    let mut script_archive = None;
    let mut virtual_tick = 0;
    let mut frame_ms = None;
    let mut save_dir = None;
    let mut system_save_path = None;
    let mut platform_root = None;
    let mut continue_without_snapshot = false;
    let mut random = None;
    while let Some(flag) = args.next() {
        if flag == "--bootstrap" {
            bootstrap = Some(Bootstrap::default());
            continue;
        }
        if flag == "--continue-without-snapshot" {
            continue_without_snapshot = true;
            continue;
        }
        let value = args.next().ok_or("missing option value")?;
        if flag == "--entry" {
            entry = Some(number(&value)?);
        } else if flag == "--steps" {
            budget = number(&value)?;
        } else if flag == "--script" {
            archive_script = Some(value);
        } else if flag == "--script-archive" {
            script_archive = Some(value);
        } else if flag == "--virtual-tick" {
            virtual_tick = number(&value)?;
        } else if flag == "--frame-ms" {
            let value = number(&value)?;
            if value == 0 {
                return Err("frame-ms must be positive".into());
            }
            frame_ms = Some(value);
        } else if flag == "--save-dir" {
            save_dir = Some(value);
        } else if flag == "--system-save" {
            system_save_path = Some(value);
        } else if flag == "--rand-seed" {
            random = Some(CrtRandom::new(number(&value)?));
        } else if flag == "--record-platform" {
            platform_root = Some(value);
        } else {
            return Err("unknown option".into());
        }
    }
    if budget == 0 || budget > 1000000 {
        return Err("step budget must be 1..1000000".into());
    }
    let mut file = File::open(&path)?;
    let mut initial_name = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("entry.ps3")
        .as_bytes()
        .to_vec();
    let raw = if let Some(name) = archive_script {
        let name = name
            .to_str()
            .filter(|s| s.is_ascii())
            .ok_or("archive script basename must be ASCII")?;
        let index = Cpz6Index::read(&mut file)?;
        initial_name = name.as_bytes().to_vec();
        index.read_entry(&mut file, index.find(name.as_bytes())?)?
    } else {
        let mut raw = Vec::new();
        file.take(MAX_PAYLOAD as u64 + 49).read_to_end(&mut raw)?;
        raw
    };
    if raw.len() > MAX_PAYLOAD + 48 {
        return Err("input exceeds research allocation limit".into());
    }
    let script = Script::decode(&raw)?;
    let mut machine = Machine::at(&script, entry.unwrap_or(script.header.entry_pc))?;
    machine.set_current_name(initial_name);
    let mut script_source = script_archive
        .map(|path| -> Result<_, Box<dyn Error>> { ArchiveScripts::new(File::open(path)?) })
        .transpose()?;
    let mut control = ScriptControl::new(virtual_tick);
    let mut saves = save_dir.map(DirectorySaves::new).transpose()?;
    let mut save_menu = SaveMenuHost::default();
    let mut system_source = system_save_path.map(FileSystemSave::new);
    let mut system_host = SystemSaveHost::default();
    let mut images = platform_root.as_ref().map(ImageArchives::new).transpose()?;
    let mut platform_assets = platform_root.map(DirectoryAssets::new).transpose()?;
    let mut platform_host = PlatformHost::default();
    let mut platform_backend = RecordingBackend::default();
    let mut save_image = SaveImage::default();
    let mut scene = SceneHost::default();
    let mut save_title = SaveTitle::default();
    let mut gesture = PointerGesture::default();
    let mut audio = AudioHost::default();
    let mut audio_backend = RecordingAudio::default();
    if platform_assets.is_some() {
        println!(
            "# RECORDING platform backend: requests and state only; no native window, cursor, icon display, or input polling"
        );
    }
    if script_source.is_some() {
        println!(
            "# script control uses explicit virtual_tick={virtual_tick}; no automatic callback scheduler"
        );
    }
    println!(
        "# Research trace {:?}; initial variables unknown; entry=0x{:x}",
        path,
        machine.pc()
    );
    for step in 0..budget {
        let pc = machine.pc();
        let slot = machine.current_slot();
        let name = machine.current_name().escape_ascii().to_string();
        let event = machine.step()?;
        if event == Step::Yield
            && let Some(delta) = frame_ms
        {
            scene.advance_frame(delta);
        }
        println!(
            "{step}: slot={slot} script={name} pc=0x{pc:04x} -> 0x{:04x} result={:?} stack_bytes={} {event:?}",
            machine.pc(),
            machine.state().result,
            machine.stack().len_bytes()
        );
        if let Step::System { command, .. } = event {
            if let Some(assets) = &mut platform_assets {
                if let Some(event) = scene.fade().try_wait(&mut machine)? {
                    if event == Step::Yield {
                        let Some(delta) = frame_ms else {
                            println!(
                                "# WAIT fade at pc=0x{:x}; PC and stack preserved; supply --frame-ms to advance engine frames",
                                machine.pc()
                            );
                            return Ok(());
                        };
                        scene.advance_frame(delta);
                        println!("# frame delta_ms={delta} fade={:?}", scene.fade().current());
                    }
                    continue;
                }
                let cursors = bootstrap
                    .as_ref()
                    .map(Bootstrap::cursor_names)
                    .unwrap_or(&[]);
                let previous = platform_backend.requests().len();
                if let Some(event) = platform_host.try_handle(
                    &mut machine,
                    &mut platform_backend,
                    assets,
                    cursors,
                )? {
                    println!(
                        "# platform recording 0x{:04x} {event:?}; requests={} thumbnail={:?}",
                        command + 0x2000,
                        platform_backend.requests().len(),
                        platform_host.thumbnail()
                    );
                    if platform_backend.requests().len() > previous {
                        match platform_backend.requests().last().expect("new request") {
                            PlatformRequest::Title(bytes) => {
                                println!("# recorded title={}", bytes.escape_ascii())
                            }
                            PlatformRequest::Cursor { id, bitmap } => println!(
                                "# recorded cursor={id} dimensions={:?}",
                                bitmap
                                    .as_ref()
                                    .map(|b| (b.width(), b.height(), b.hotspot()))
                            ),
                            PlatformRequest::Icon { name, bytes } => println!(
                                "# recorded icon={} bytes={} (directory validated, pixels opaque)",
                                name.escape_ascii(),
                                bytes.len()
                            ),
                        }
                    }
                    continue;
                }
                if save_title.try_handle(&mut machine)? {
                    println!(
                        "# save title 0x2162 = {}; recorded only",
                        save_title.bytes().escape_ascii()
                    );
                    continue;
                }
                if text::try_handle(&mut machine)? {
                    println!(
                        "# text 0x{:04x}: system_integer[0]={:?}",
                        command + 0x2000,
                        machine.state().system_integer(0)
                    );
                    continue;
                }
                if audio.try_handle(&mut machine, &mut audio_backend)? {
                    if let (
                        0xb0,
                        Some(AudioRequest::PlaySe {
                            channel,
                            name,
                            volume,
                            looped,
                            ..
                        }),
                    ) = (command, audio_backend.requests().last())
                    {
                        println!(
                            "# audio 0x20b0 recorded SE[{channel}]={} volume={volume} loop={looped}; not loaded/played, system_integer[0]={:?}",
                            name.escape_ascii(),
                            machine.state().system_integer(0)
                        );
                    } else if let (0xb1, Some(AudioRequest::StartSe { channel, looped })) =
                        (command, audio_backend.requests().last())
                    {
                        println!(
                            "# audio 0x20b1 recorded start SE[{channel}] loop={looped}; no playback, system_integer[0]={:?}",
                            machine.state().system_integer(0)
                        );
                    }
                    continue;
                }
                if save_menu.try_handle_popup(&mut machine)? {
                    println!(
                        "# popup menu 0x{:04x}: enabled={}; system_integer[0]={:?}; no native menu",
                        command + 0x2000,
                        save_menu.popup_enabled(),
                        machine.state().system_integer(0)
                    );
                    continue;
                }
                if gesture.try_handle(&mut machine)? {
                    println!(
                        "# pointer gesture config 0x{:04x}; fields +0..+36={:?}; state only, no input",
                        command + 0x2000,
                        (0..=36)
                            .step_by(4)
                            .map(|offset| gesture.field(offset).unwrap_or(0))
                            .collect::<Vec<_>>()
                    );
                    continue;
                }
                if scene.try_handle(&mut machine)? {
                    println!(
                        "# scene state 0x{:04x}: {:?} rgba={:?} objects={:?}; recorded only, nothing drawn",
                        command + 0x2000,
                        scene.clear(),
                        scene.clear().rgba(),
                        scene.objects().keys().collect::<Vec<_>>()
                    );
                    continue;
                }
                let no_paths = BTreeMap::new();
                let paths = bootstrap.as_ref().map_or(&no_paths, Bootstrap::paths);
                if let Some(images) = &mut images
                    && scene.try_handle_image(&mut machine, &mut images.with_paths(paths))?
                {
                    println!(
                        "# scene image 0x{:04x}: system_integer[0]={:?} objects={:?}; decoded only, nothing drawn",
                        command + 0x2000,
                        machine.state().system_integer(0),
                        scene
                            .objects()
                            .iter()
                            .filter_map(|(slot, object)| object.image.as_ref().map(|image| (
                                slot,
                                image.name.escape_ascii().to_string(),
                                image.pixels.width(),
                                image.pixels.height(),
                                image.alpha
                            )))
                            .collect::<Vec<_>>()
                    );
                    continue;
                }
            }
            if let Some(random) = &mut random
                && random.try_handle(&mut machine)?
            {
                println!(
                    "# rand 0x20d4: system_integer[0]={:?} (explicit research seed)",
                    machine.state().system_integer(0)
                );
                continue;
            }
            if let Some(source) = &mut system_source {
                let game_id = bootstrap
                    .as_ref()
                    .and_then(|host| host.settings().get(&0x201d).copied());
                if let Some(event) = system_host.try_handle(&mut machine, source, game_id)? {
                    println!(
                        "# system save {event:?}; known integers={} known bits={}",
                        machine.state().integers().len(),
                        machine.state().bits().len()
                    );
                    continue;
                }
            }
            let control_event = if let Some(source) = &mut script_source {
                control.try_handle(&mut machine, source)?
            } else {
                None
            };
            if let Some(event) = control_event {
                println!(
                    "# script control 0x{:04x}: {event:?}; slot={} script={} pc=0x{:x} callbacks={}",
                    command + 0x2000,
                    machine.current_slot(),
                    machine.current_name().escape_ascii(),
                    machine.pc(),
                    control.callbacks().len()
                );
                if event == ControlEvent::MainPreparedNeedsSnapshot && continue_without_snapshot {
                    save_image.capture_unavailable(&mut machine)?;
                    println!(
                        "# main snapshot NOT built; bit[0] cleared as in original; save image unavailable from {}:0x{:x}",
                        machine.current_name().escape_ascii(),
                        machine.pc()
                    );
                    continue;
                }
                if event == ControlEvent::MainPreparedNeedsSnapshot {
                    println!(
                        "# STOP main context prepared; original engine snapshot still requires a host implementation"
                    );
                    for (slot, name) in machine.loaded_names() {
                        println!("# loaded[{slot}]={}", name.escape_ascii());
                    }
                    println!(
                        "# known integers={} known bits={} stack_bytes={}",
                        machine.state().integers().len(),
                        machine.state().bits().len(),
                        machine.stack().len_bytes()
                    );
                    return Ok(());
                }
                continue;
            }
            if let Some(source) = &mut saves
                && save_menu.try_handle(&mut machine, source)?
            {
                println!(
                    "# save menu applied 0x{:04x}; second_group_enabled={:?}",
                    command + 0x2000,
                    save_menu.second_group_enabled()
                );
                if command == 0x2b3 {
                    let menu = save_menu.menu().expect("completed scan");
                    println!(
                        "# save slots={} latest={:?}; header recognition only, no snapshot restoration",
                        menu.slots.len(),
                        menu.latest
                    );
                    for (slot, status) in menu.slots.iter().enumerate() {
                        match status {
                            SlotStatus::Recognized(header) => println!(
                                "# save[{slot:03}] label={} key=0x{:08x}",
                                header.label.escape_ascii(),
                                header.ordering_key
                            ),
                            other => println!("# save[{slot:03}]={other:?}"),
                        }
                    }
                }
                continue;
            }
            if let Some(host) = &mut bootstrap {
                if let Some(event) = host.try_handle(&mut machine)? {
                    println!(
                        "# bootstrap applied 0x{:04x} -> pc=0x{:04x} {event:?}",
                        command + 0x2000,
                        machine.pc()
                    );
                    continue;
                }
                println!(
                    "# bootstrap handled={} registered_paths={}",
                    host.handled_count(),
                    host.paths().len()
                );
                for ((op, slot), path) in host.paths() {
                    println!("# path 0x{op:04x}[{slot}]={}", path.escape_ascii());
                }
                for (i, name) in host.cursor_names().iter().enumerate() {
                    println!("# cursor[{}]={}", i + 1, name.escape_ascii());
                }
                println!(
                    "# system_integer[0]={:?}",
                    machine.state().system_integer(0)
                );
            }
            println!(
                "# STOP pending system=0x{:04x}; PC and parameters preserved; not game completion",
                command + 0x2000
            );
            if machine.stack().len_bytes() >= 4 {
                println!("# top_word=0x{:08x}", machine.stack().peek_u32(4)?);
            }
            println!(
                "# known integers={} known bits={}",
                machine.state().integers().len(),
                machine.state().bits().len()
            );
            return Ok(());
        }
    }
    Err("research step budget exhausted".into())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
