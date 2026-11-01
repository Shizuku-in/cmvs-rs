use cmvs_rs::{
    MAX_PAYLOAD,
    audio::{AudioHost, AudioRequest, RecordingAudio},
    bootstrap::Bootstrap,
    buttons::ButtonHost,
    camera::CameraHost,
    cpz::ArchiveIndex,
    fade::SkipGate,
    gesture::PointerGesture,
    history::HistoryHost,
    image::ImageArchives,
    input::{self, ActionInput},
    machine::{Machine, Step},
    message_flags::MessageFlags,
    movie::{DirectoryMovies, MovieHost},
    platform::{DirectoryAssets, PlatformHost, PlatformRequest, RecordingBackend},
    profile::{CommandGate, Evidence, FileEvidence, Registry},
    ps3::Script,
    random::CrtRandom,
    save_menu::{DirectorySaves, SaveMenuHost, SlotStatus},
    scene::SceneHost,
    script_control::{ArchiveScripts, ControlEvent, ScriptControl},
    sequence::SequenceHost,
    snapshot::{SaveImage, SaveTitle},
    system_save::{FileSystemSave, SystemSaveHost},
    timers::VmTimers,
};
use std::{collections::BTreeMap, env, error::Error, fs::File, io::Read};

const USAGE: &str = "\
Usage: cmvs-trace <script-or-archive> [options]

Runs the VM on a script or script archive and prints the trace. Read-only research tool.

Input and execution:
  --script <name>              load this ASCII basename from the archive as the initial script
  --script-archive <archive>   supply common and main scripts; enables control handling
  --entry <pc>                 start at this address instead of the script entry
  --steps <budget>             VM step budget (default 10000)
  --bootstrap                  apply the verified startup configuration
  --virtual-tick <tick>        callback clock for control handling (default 0)
  --frame-ms <delta>           engine frame time per wait or yield; without it time stays frozen
  --rand-seed <seed>           enable 0x20d4 with CRT rand() from this seed

Saves and platform:
  --save-dir <directory>       scan saveNNN.dat headers read-only for menu state
  --system-save <file>         load CSS1 persistent state for 0x2128 (no gameplay restore)
  --record-platform <root>     recording-only platform backend using that game root
  --continue-without-snapshot  pass main entry and 0x22b2 without building a save image

Engine profile:
  --profile <id>               force a profile; needed for synthetic or unrecorded scripts
  --allow-presumed             research only: run legacy presumed commands
  --allow-unreviewed           research only: run unreviewed commands with base semantics

Notes:
  - The profile is identified from the input, --script-archive and, with --record-platform,
    the executables in that root. Every system command passes the gate first: Base,
    Inherited, Reviewed and ContractEquivalent run existing handlers; absent and no-op
    commands always stop; presumed and unreviewed commands stop unless allowed.
  - --record-platform reads cursor and icon assets, applies verified state (clear color,
    save title, pointer-gesture configuration), records SE requests without audio output
    and decodes 0x2030 object images from 0x2011 entries. No window, drawing or input.
  - Without --continue-without-snapshot, main transfer and 0x22b2 stop at the engine save
    snapshot; with it only the verified live-state effect applies and saving is unavailable.
  - Callback ticks stay separate from --frame-ms; --script-archive schedules no callbacks.
  - The original seeds rand() from timeGetTime and draws every frame, so its values are
    not reproducible.";

// Emit source diagnostics on every return path, including a handler error or budget stop.
struct TraceGate<'a>(CommandGate<'a>);
impl<'a> std::ops::Deref for TraceGate<'a> {
    type Target = CommandGate<'a>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TraceGate<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for TraceGate<'_> {
    fn drop(&mut self) {
        if self.0.selection().profile.base.is_some() {
            use std::io::Write;
            let mut stdout = std::io::stdout().lock();
            for line in self.0.diagnostic_lines() {
                // Diagnostics must not cause a second panic if the output pipe closed.
                let _ = writeln!(stdout, "{line}");
            }
        }
    }
}

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
        .ok_or("usage: cmvs-trace <script-or-archive> [options]; see --help")?;
    if path == "--help" {
        println!("{USAGE}");
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
    let mut profile = None;
    let mut allow_unreviewed = false;
    let mut allow_presumed = false;
    while let Some(flag) = args.next() {
        if flag == "--allow-presumed" {
            allow_presumed = true;
            continue;
        }
        if flag == "--allow-unreviewed" {
            allow_unreviewed = true;
            continue;
        }
        if flag == "--bootstrap" {
            bootstrap = Some(Bootstrap::default());
            continue;
        }
        if flag == "--continue-without-snapshot" {
            continue_without_snapshot = true;
            continue;
        }
        let value = args.next().ok_or("missing option value")?;
        if flag == "--profile" {
            profile = Some(
                value
                    .into_string()
                    .map_err(|_| "profile id must be ASCII")?,
            );
        } else if flag == "--entry" {
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
    let label = |p: &std::ffi::OsStr| p.to_string_lossy().into_owned();
    let mut evidence = Evidence::default();
    if archive_script.is_some() {
        evidence.script_archive = Some(FileEvidence::read(path.as_ref(), label(&path))?);
    } else {
        evidence.start_script = Some(FileEvidence::read(path.as_ref(), label(&path))?);
    }
    if let Some(archive) = &script_archive {
        let read = FileEvidence::read(archive.as_ref(), label(archive))?;
        if evidence
            .script_archive
            .as_ref()
            .is_some_and(|f| f.sha256 != read.sha256)
        {
            return Err("--script-archive differs from the input archive".into());
        }
        evidence.script_archive = Some(read);
    }
    if let Some(root) = &platform_root {
        evidence.executables = Evidence::executables_in(root.as_ref())?;
    }
    let mut gate = TraceGate(CommandGate::new(
        Registry::builtin().select(profile.as_deref(), &evidence)?,
    ));
    gate.set_allow_unreviewed(allow_unreviewed);
    gate.set_allow_presumed(allow_presumed);
    if allow_presumed {
        println!("# research policy: legacy presumed commands allowed (--allow-presumed)");
    }
    for line in gate.selection().log_lines() {
        println!("{line}");
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
        let index = ArchiveIndex::read(&mut file)?;
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
    let movie_root = platform_root
        .as_ref()
        .map(|root| std::path::Path::new(root).canonicalize())
        .transpose()?;
    let mut platform_assets = platform_root.map(DirectoryAssets::new).transpose()?;
    let mut platform_host = PlatformHost::default();
    let mut platform_backend = RecordingBackend::default();
    let mut save_image = SaveImage::default();
    let mut scene = SceneHost::default();
    let mut timers = VmTimers::default();
    let mut resume_run = true;
    let mut frame_delta = 0;
    let mut save_title = SaveTitle::default();
    let mut gesture = PointerGesture::default();
    let mut history = HistoryHost::default();
    let mut message_flags = MessageFlags::default();
    let mut settings = cmvs_rs::settings::SettingsHost::default();
    let mut buttons = ButtonHost::default();
    let mut movies = MovieHost::default();
    let mut sequence = SequenceHost::default();
    let mut camera = CameraHost::default();
    // Traces supply no input: no CMVS action is ever active.
    let mut input = ActionInput::default();
    let mut audio = if let Some(root) = &movie_root {
        let config = match std::fs::read(root.join("cmvs.cfg")) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        AudioHost::from_ini(&config)
    } else {
        AudioHost::default()
    };
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
        if resume_run {
            timers.begin_run(std::mem::take(&mut frame_delta));
            resume_run = false;
        }
        let pc = machine.pc();
        let slot = machine.current_slot();
        let name = machine.current_name().escape_ascii().to_string();
        let event = machine.step()?;
        if event == Step::Yield
            && let Some(delta) = frame_ms
        {
            if let (Some(images), Some(config)) = (&mut images, &bootstrap) {
                sequence.advance_frame_with_hosts(
                    delta,
                    &mut scene,
                    &mut images.with_paths(config.paths()),
                    &mut camera,
                )?;
            } else {
                sequence.advance_frame(delta)?;
            }
            scene.advance_frame(delta);
            camera.advance_frame(delta);
            movies.advance(delta);
            frame_delta = delta;
        }
        if event == Step::Yield {
            resume_run = true;
        }
        println!(
            "{step}: slot={slot} script={name} pc=0x{pc:04x} -> 0x{:04x} result={:?} stack_bytes={} {event:?}",
            machine.pc(),
            machine.state().result,
            machine.stack().len_bytes()
        );
        if let Step::System {
            pc: system_pc,
            command,
        } = event
        {
            let resolved = gate.resolve(&machine, system_pc, command)?;
            if let Some(first) = resolved.first_unreviewed {
                println!(
                    "# UNREVIEWED system command 0x{:04x} allowed with {} (--allow-unreviewed, first use)",
                    0x2000 + first,
                    cmvs_rs::dispatch::semantics_name(gate.selection().profile, first)
                );
            }
            let wait_gate = SkipGate {
                skip: message_flags.skip(),
                ..SkipGate::default()
            };
            let no_paths = BTreeMap::new();
            let paths = bootstrap.as_ref().map_or(&no_paths, Bootstrap::paths);
            let mut registered_images = images.as_mut().map(|images| images.with_paths(paths));
            let is_movie_player = cmvs_rs::dispatch::uses_movie_player(command);
            let movie_settings = audio.settings();
            let mut movie_source = movie_root.as_ref().map(|root| DirectoryMovies {
                root,
                directory: paths.get(&(0x2015, 0)).map(Vec::as_slice),
            });
            let mut hosts = cmvs_rs::dispatch::Hosts {
                buttons: platform_assets.is_some().then_some(&mut buttons),
                movies: platform_assets.is_some().then_some(&mut movies),
                timers: platform_assets.is_some().then_some(&mut timers),
                flags: platform_assets.is_some().then_some(&mut message_flags),
                random: random.as_mut(),
                settings: platform_assets.is_some().then_some(&mut settings),
                history: platform_assets.is_some().then_some(&mut history),
                strings: platform_assets.is_some(),
                gesture: platform_assets.is_some().then_some(&mut gesture),
                popup: platform_assets.is_some().then_some(&mut save_menu),
                save_title: platform_assets.is_some().then_some(&mut save_title),
                effect_objects: platform_assets.is_some(),
                ..Default::default()
            };
            if platform_assets.is_some() {
                if is_movie_player {
                    hosts.movie_player =
                        movie_source
                            .as_mut()
                            .map(|source| cmvs_rs::dispatch::MoviePlayerHosts {
                                source,
                                settings: movie_settings,
                                backend: &mut audio_backend,
                            });
                } else {
                    hosts.audio = Some(cmvs_rs::dispatch::AudioHosts {
                        audio: &mut audio,
                        backend: &mut audio_backend,
                    });
                }
            }
            if platform_assets.is_some() && cmvs_rs::dispatch::uses_sequence(command) {
                hosts.sequence = Some(cmvs_rs::dispatch::SequenceHosts {
                    host: &mut sequence,
                    execution: if bootstrap.is_some() {
                        registered_images.as_mut().map(|images| {
                            cmvs_rs::sequence::SequenceContext {
                                scene: &mut scene,
                                images,
                                camera: Some(&mut camera),
                            }
                        })
                    } else {
                        None
                    },
                });
            } else {
                hosts.camera = platform_assets.is_some().then_some(&mut camera);
                if cmvs_rs::dispatch::uses_images(command) {
                    hosts.images =
                        registered_images
                            .as_mut()
                            .map(|source| cmvs_rs::dispatch::ImageHosts {
                                scene: &mut scene,
                                source,
                                change_images: false,
                            });
                } else if platform_assets.is_some() && cmvs_rs::dispatch::is_wait(command) {
                    hosts.waits = Some(cmvs_rs::dispatch::WaitHosts {
                        scene: Some(&mut scene),
                        camera: None,
                        input: &mut input,
                        gate: wait_gate,
                        change_images: false,
                    });
                } else if platform_assets.is_some() {
                    hosts.scene = Some(&mut scene);
                    hosts.input = Some(cmvs_rs::dispatch::InputHosts {
                        input: &mut input,
                        skip: wait_gate.skip,
                        clock: None,
                    });
                }
            }
            let dispatched = cmvs_rs::dispatch::run(resolved.semantics, &mut machine, &mut hosts)?;

            if dispatched.is_some_and(cmvs_rs::dispatch::Flow::yields) {
                let Some(delta) = frame_ms else {
                    println!(
                        "# WAIT fade/mask transition at pc=0x{:x}; PC and stack preserved; supply --frame-ms to advance engine frames",
                        machine.pc()
                    );
                    return Ok(());
                };
                if let (Some(images), Some(config)) = (&mut images, &bootstrap) {
                    sequence.advance_frame_with_hosts(
                        delta,
                        &mut scene,
                        &mut images.with_paths(config.paths()),
                        &mut camera,
                    )?;
                } else {
                    sequence.advance_frame(delta)?;
                }
                scene.advance_frame(delta);
                camera.advance_frame(delta);
                movies.advance(delta);
                frame_delta = delta;
                resume_run = true;
                println!(
                    "# frame delta_ms={delta} fade={:?} mask={:?}",
                    scene.fade().current(),
                    scene
                        .mask()
                        .map(|mask| (mask.slot, mask.elapsed, mask.duration()))
                );
                continue;
            }
            if dispatched == Some(cmvs_rs::dispatch::Flow::Done) {
                use cmvs_rs::dispatch::Capability;
                let capability = cmvs_rs::dispatch::ROOT
                    .iter()
                    .find(|entry| entry.command == command)
                    .expect("completed root command has an entry")
                    .capability;
                match capability {
                    Capability::TextState
                    | Capability::AuxiliaryScene
                    | Capability::Windows
                    | Capability::WindowPointer
                    | Capability::WindowButtons
                    | Capability::WindowImages => {
                        return Err("trace lacks a message window host".into());
                    }
                    Capability::ButtonInput => {
                        let group = buttons.group(0);
                        println!(
                            "# button input 0x{:04x}: group0 current={:?} pressed={:?}; system_integer[0]={:?}; no input supplied",
                            command + 0x2000,
                            group.and_then(|g| g.current),
                            group.map(|g| g.pressed_visual),
                            machine.state().system_integer(0).map(|v| v as i32)
                        );
                    }
                    Capability::Buttons => {
                        println!(
                            "# button 0x{:04x}: groups={:?} merged_states={}; system_integer[0]={:?}; scene state only, no input",
                            command + 0x2000,
                            (0..6)
                                .filter_map(|i| buttons.group(i).map(|g| (
                                    i,
                                    g.object,
                                    g.buttons.len()
                                )))
                                .collect::<Vec<_>>(),
                            buttons.merged_states(),
                            machine.state().system_integer(0)
                        );
                    }
                    Capability::Input => {
                        if command == 0x2ef {
                            println!(
                                "# skipping 0x22ef: script skip={} system_integer[0]={:?}; no input supplied",
                                message_flags.skip(),
                                machine.state().system_integer(0)
                            );
                        } else {
                            println!(
                                "# input 0x{:04x}: system_integer[0]={:?} [4]={:?}; no input supplied",
                                command + 0x2000,
                                machine.state().system_integer(0),
                                machine.state().system_integer(4)
                            );
                        }
                    }
                    Capability::ListInput | Capability::DragPress => {
                        return Err("trace lacks the dispatched input capability".into());
                    }
                    Capability::Sequence => {
                        println!(
                            "# sequence 0x{:04x}: header={:?}",
                            command + 0x2000,
                            sequence.header()
                        );
                    }
                    Capability::MovieState => {
                        println!(
                            "# movie 0x{:04x}: slots occupied={:?}; creation is not modeled",
                            command + 0x2000,
                            (0..3).map(|i| movies.occupied(i)).collect::<Vec<_>>()
                        );
                    }
                    Capability::MoviePlayer => {
                        println!(
                            "# movie 0x{:04x}: player={} system_integer[0]={:?}; frames not decoded, last audio request={:?}",
                            command + 0x2000,
                            movies.player().map_or("none".into(), |m| format!(
                                "{} frame={}/{} rate={}/{}",
                                m.path.escape_ascii(),
                                m.frame,
                                m.index.header.max_sequence,
                                m.index.header.rate,
                                m.index.header.rate_time
                            )),
                            machine.state().system_integer(0),
                            audio_backend.requests().last().map(|r| match r {
                                AudioRequest::PlayMovieAudio {
                                    offset,
                                    size,
                                    volume,
                                    ..
                                } => format!(
                                    "PlayMovieAudio offset={offset} size={size} volume={volume}"
                                ),
                                other => format!("{other:?}"),
                            })
                        );
                    }
                    Capability::MovieScene | Capability::MovieStart => {
                        return Err("trace lacks scene movie playback".into());
                    }
                    Capability::Quake => {
                        println!(
                            "# quake 0x{:04x}: {:?} system_integer[0]={:?}",
                            command + 0x2000,
                            scene.quake(),
                            machine.state().system_integer(0)
                        );
                    }
                    Capability::Strip => {
                        println!(
                            "# strip wipe 0x{:04x}: {:?} system_integer[0]={:?}; not drawn",
                            command + 0x2000,
                            scene.strip_wipe().map(|wipe| (
                                wipe.mode,
                                wipe.size,
                                wipe.duration,
                                wipe.layer,
                                wipe.strips.len(),
                                wipe.finished
                            )),
                            machine.state().system_integer(0)
                        );
                    }
                    Capability::Scene => {
                        println!(
                            "# scene state 0x{:04x}: {:?} rgba={:?} objects={:?}; recorded only, nothing drawn",
                            command + 0x2000,
                            scene.clear(),
                            scene.clear().rgba(),
                            scene.objects().keys().collect::<Vec<_>>()
                        );
                    }
                    Capability::Camera => {
                        let floats = |c: usize| {
                            camera
                                .camera(c)
                                .map(|cam| (0..19).map(|i| cam.float(i)).collect::<Vec<_>>())
                        };
                        println!(
                            "# camera 0x{:04x}: camera0 words0..18 as f32={:?} camera1={:?} moving={}; stored only, 2D rendering unaffected",
                            command + 0x2000,
                            floats(0),
                            floats(1),
                            camera.moving()
                        );
                    }
                    Capability::SceneImages => {
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
                    }
                    Capability::MaskImages => {
                        let channel = if command >= 0x234 {
                            scene.second_mask()
                        } else {
                            scene.mask()
                        };
                        println!(
                            "# mask transition 0x{:04x}: system_integer[0]={:?} mask={:?}; shader path, nothing drawn",
                            command + 0x2000,
                            machine.state().system_integer(0),
                            channel.map(|mask| (
                                mask.slot,
                                mask.put,
                                mask.mask.name.escape_ascii().to_string(),
                                mask.duration()
                            ))
                        );
                    }
                    Capability::Audio => {
                        if let (
                            0xb0 | 0xba,
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
                        } else if let (0xb3, Some(AudioRequest::StopSe { channel })) =
                            (command, audio_backend.requests().last())
                        {
                            println!(
                                "# audio 0x20b3 recorded stop SE[{channel}]; record={:?}, system_integer[0]={:?}",
                                audio.channel(usize::from(*channel)),
                                machine.state().system_integer(0)
                            );
                        } else if matches!(command, 0xa0..=0xa2 | 0xa4) {
                            let bgm = audio.bgm();
                            println!(
                                "# audio 0x{:04x} BGM channel={} primary={} secondary={} playing={}; last request={:?}; not loaded/played",
                                command + 0x2000,
                                bgm.channel,
                                bgm.primary.escape_ascii(),
                                bgm.secondary.escape_ascii(),
                                bgm.playing,
                                audio_backend.requests().last()
                            );
                        }
                    }
                    Capability::Timers => println!(
                        "# timer 0x{:04x}: values={:?} system_integer[0]={:?}",
                        command + 0x2000,
                        timers.values(),
                        machine.state().system_integer(0)
                    ),
                    Capability::Flags => println!(
                        "# message flag 0x{:04x}: skip={} auto_advance={} bypass={}; system_integer[0]={:?}",
                        command + 0x2000,
                        message_flags.skip(),
                        message_flags.auto_advance(),
                        message_flags.bypass(),
                        machine.state().system_integer(0)
                    ),
                    Capability::Random => println!(
                        "# rand 0x20d4: system_integer[0]={:?} (explicit research seed)",
                        machine.state().system_integer(0)
                    ),
                    Capability::Settings => println!(
                        "# configuration 0x{:04x}; system_integer[0]={:?}",
                        command + 0x2000,
                        machine.state().system_integer(0)
                    ),
                    Capability::History => println!(
                        "# history entries={} (newest first; no history UI)",
                        history.entries().len()
                    ),
                    Capability::Strings => println!(
                        "# text 0x{:04x}: system_integer[0]={:?}",
                        command + 0x2000,
                        machine.state().system_integer(0)
                    ),
                    Capability::SaveTitle => println!(
                        "# save title 0x2162 = {}; recorded only",
                        save_title.bytes().escape_ascii()
                    ),
                    Capability::EffectObjects => {
                        println!("# effect object release 0x22a0: no +4024 object is modeled")
                    }
                    Capability::Popup => println!(
                        "# popup menu 0x{:04x}: enabled={}; system_integer[0]={:?}; no native menu",
                        command + 0x2000,
                        save_menu.popup_enabled(),
                        machine.state().system_integer(0)
                    ),
                    Capability::Gesture => println!(
                        "# pointer gesture config 0x{:04x}; fields +0..+36={:?}; state only, no input",
                        command + 0x2000,
                        (0..=36)
                            .step_by(4)
                            .map(|offset| gesture.field(offset).unwrap_or(0))
                            .collect::<Vec<_>>()
                    ),
                    Capability::FadeWait
                    | Capability::MaskWait
                    | Capability::QuakeWait
                    | Capability::StripWait
                    | Capability::ChangeWait
                    | Capability::CameraWait => {}
                    Capability::Text
                    | Capability::Math
                    | Capability::SaveInfo
                    | Capability::Effects
                    | Capability::ImageChange => {
                        return Err("trace lacks the dispatched capability".into());
                    }
                }
                continue;
            }
            // Root commands without an optional host retain the original stop
            // diagnostics below. A selected research override stops immediately.
            if matches!(
                dispatched,
                Some(cmvs_rs::dispatch::Flow::Unhandled | cmvs_rs::dispatch::Flow::Unavailable)
            ) && resolved.semantics != cmvs_rs::profile::Semantics::Root
            {
                println!(
                    "# STOP pending system=0x{:04x}; selected text handler needs a text host; PC and parameters preserved; not game completion",
                    command + 0x2000
                );
                return Ok(());
            }
            if save_image.try_handle(&mut machine, continue_without_snapshot)? {
                println!(
                    "# scenario snapshot NOT built; bit[0] and repeat count cleared; save image unavailable: {save_image:?}"
                );
                continue;
            }
            if let Some(assets) = &mut platform_assets {
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
                            PlatformRequest::Fullscreen(value) => {
                                println!("# recorded fullscreen={value}")
                            }
                            PlatformRequest::AspectMode(value) => {
                                println!("# recorded aspect mode={value}")
                            }
                            PlatformRequest::AspectScaling(value) => {
                                println!("# recorded aspect scaling={value}")
                            }
                            PlatformRequest::RenderMode(value) => {
                                println!("# recorded render mode={value}")
                            }
                            PlatformRequest::WindowSize(value) => {
                                println!("# recorded window size={value:?}")
                            }
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
                    if event == cmvs_rs::platform::PlatformEvent::Yield {
                        if let Some(delta) = frame_ms {
                            if let (Some(images), Some(config)) = (&mut images, &bootstrap) {
                                sequence.advance_frame_with_hosts(
                                    delta,
                                    &mut scene,
                                    &mut images.with_paths(config.paths()),
                                    &mut camera,
                                )?;
                            } else {
                                sequence.advance_frame(delta)?;
                            }
                            scene.advance_frame(delta);
                            camera.advance_frame(delta);
                            movies.advance(delta);
                            frame_delta = delta;
                        }
                        resume_run = true;
                    }
                    continue;
                }
                if command == 0x34d && input::try_handle(&mut machine, &mut input)? {
                    println!(
                        "# input 0x{:04x}: system_integer[0]={:?} [4]={:?}; no input supplied",
                        command + 0x2000,
                        machine.state().system_integer(0),
                        machine.state().system_integer(4)
                    );
                    continue;
                }
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
                if command == 0x2bb {
                    println!(
                        "# save signature result={:?}; signature recognition only, no snapshot restoration",
                        machine.state().system_integer(0)
                    );
                } else if command == 0x2b3 {
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
