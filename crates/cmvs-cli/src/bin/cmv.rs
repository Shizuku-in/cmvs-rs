//! Read-only CMV6 inspection and bounded JBPD frame decoding.
use cmvs_core::{format::cmv::CmvIndex, format::cmv::KIND_JBPX, format::cmv::KIND_OGG};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::Write,
};

const USAGE: &str = "\
Usage: cmvs-cmv <movie.cmv> [--check-frames | --frame <index> <new-file>]

Inspect a CMV6 movie. Without options, print its header and resource index.

Options (choose at most one):
  --check-frames             decode every JBPD frame without exporting
  --frame <index> <new-file>  export one frame; index is zero-based
                            .rgba writes raw top-down RGBA; otherwise writes PNG
  -h, --help                 show this help

Output files must not already exist. Input files are read only.
Ogg audio is located but not decoded; use cmvs-audio to inspect it.
Example: cmvs-cmv op.cmv --frame 0 frame.png";

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let all_args: Vec<_> = env::args_os().skip(1).collect();
    if cmvs_cli::wants_help(&all_args) {
        println!("{USAGE}");
        return Ok(());
    }
    let mut path: Option<std::ffi::OsString> = None;
    let mut check_frames = false;
    let mut frame_spec: Option<(u32, std::ffi::OsString)> = None;
    let mut positional_only = false;
    let mut args = all_args.into_iter();
    while let Some(arg) = args.next() {
        if !positional_only && arg == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && arg == "--check-frames" {
            if check_frames || frame_spec.is_some() {
                return Err("expected --check-frames or --frame <index> <new-file>".into());
            }
            check_frames = true;
            continue;
        }
        if !positional_only && arg == "--frame" {
            if check_frames || frame_spec.is_some() {
                return Err("expected --check-frames or --frame <index> <new-file>".into());
            }
            let index = args
                .next()
                .and_then(|s| s.to_str().and_then(|s| s.parse::<u32>().ok()))
                .ok_or("--frame index must be a non-negative decimal integer")?;
            let out_path = args
                .next()
                .ok_or("expected --check-frames or --frame <index> <new-file>")?;
            frame_spec = Some((index, out_path));
            continue;
        }
        if !positional_only && arg.to_string_lossy().starts_with('-') {
            return Err(format!("unknown option {}", arg.to_string_lossy()).into());
        }
        if path.is_some() {
            return Err("expected --check-frames or --frame <index> <new-file>".into());
        }
        path = Some(arg);
    }
    let path = path.ok_or("usage: cmvs-cmv <movie.cmv>")?;
    let mut file = File::open(path)?;
    let index = CmvIndex::read_io(&mut file)?;
    if check_frames {
        let start = std::time::Instant::now();
        let mut count = 0;
        for i in 0..index.header.max_sequence {
            index
                .decode_frame(&mut file, i)
                .map_err(|e| format!("frame {i}: {e}"))?;
            count += 1;
        }
        println!(
            "decoded {count} frames successfully in {} ms",
            start.elapsed().as_millis()
        );
        return Ok(());
    }
    if let Some((frame, out_path)) = frame_spec {
        let image = index.decode_frame(&mut file, frame)?;
        let bytes = if std::path::Path::new(&out_path)
            .extension()
            .is_some_and(|s| s == "rgba")
        {
            image.pixels().to_vec()
        } else {
            image.to_png()
        };
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out_path)?
            .write_all(&bytes)?;
        println!(
            "decoded frame {frame}: {}x{}",
            image.width(),
            image.height()
        );
        return Ok(());
    }
    let h = &index.header;
    println!(
        "CMV6 resource_offset={} file_size={} unused={} max_sequence={} rate={}/{} size={}x{} bits={} audio_flag={}",
        h.resource_offset,
        h.file_size,
        h.unused,
        h.max_sequence,
        h.rate,
        h.rate_time,
        h.width,
        h.height,
        h.bit_count,
        h.audio_flag
    );
    let frames: Vec<_> = index.frames().collect();
    let stored: u64 = frames.iter().map(|e| u64::from(e.stored_size)).sum();
    println!(
        "entries={} jbpx_frames={} jbpx_stored_bytes={} other_kinds={}",
        index.entries.len(),
        frames.len(),
        stored,
        index
            .entries
            .iter()
            .filter(|e| e.kind != KIND_JBPX && e.kind != KIND_OGG)
            .count()
    );
    for (label, entry) in [
        ("first_frame", frames.first()),
        ("last_frame", frames.last()),
    ] {
        if let Some(e) = entry {
            println!(
                "{label}: sequence={} stored={} decoded={} file_offset={}",
                e.sequence,
                e.stored_size,
                e.decoded_size,
                e.file_offset(h)
            );
        }
    }
    if let Some(e) = index.audio() {
        println!(
            "ogg: sequence={} stored={} file_offset={}",
            e.sequence,
            e.stored_size,
            e.file_offset(h)
        );
    }
    let millis = u64::from(h.max_sequence) * 1000 * u64::from(h.rate_time) / u64::from(h.rate);
    println!("player_frames=0..{} duration_ms~{millis}", h.max_sequence);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-cmv --help' for usage.");
        std::process::exit(1);
    }
}
