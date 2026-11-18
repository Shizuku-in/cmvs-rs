use cmvs_core::{
    format::digest::sha256, format::digest::to_hex, format::mgv::Decoder, format::mgv::MgvIndex,
};
use cmvs_video::TheoraStream;
use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::Write,
};

const USAGE: &str = "\
Usage: cmvs-mgv <movie.mgv> [--check-frames | --frame <index> <new-file>]

Inspect an MGV1 movie. Without options, print header and packet counts.

Options (choose at most one):
  --check-frames             decode all video packets and print RGBA hashes
  --frame <index> <new-file>  export one frame; index is zero-based
                            .rgba writes raw top-down RGBA; otherwise writes PNG
  -h, --help                 show this help

Output files must not already exist. Input files are read only.
Audio is not decoded. Video decoding uses the portable Theora backend.
Build: cargo build --bin cmvs-mgv
Example: cmvs-mgv op.mgv --frame 0 frame.png";

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let raw_args: Vec<_> = std::env::args_os().skip(1).collect();
    if cmvs_cli::wants_help(&raw_args) {
        println!("{USAGE}");
        return Ok(());
    }
    let mut path: Option<std::ffi::OsString> = None;
    let mut check_frames = false;
    let mut frame_spec: Option<(u32, std::ffi::OsString)> = None;
    let mut positional_only = false;
    let mut args = raw_args.into_iter();
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
    let path = path.ok_or("missing movie path")?;
    let mut file = File::open(&path)?;
    let index = MgvIndex::read_io(&mut file)?;
    println!(
        "MGV1 {}x{} frames={} packets={} rate={}/{} audio={} video={}",
        index.width,
        index.height,
        index.frames.len(),
        index.packet_count(),
        index.rate,
        index.rate_time,
        index.audio_size,
        index.video_size
    );
    if !check_frames && frame_spec.is_none() {
        return Ok(());
    }
    let mut decoder = TheoraStream::open(Box::new(file), &index)?;
    if check_frames {
        let mut last = None;
        for (i, entry) in index.frames.iter().enumerate() {
            if last == Some(entry.packet) {
                continue;
            }
            let image = decoder.frame(i as u32)?;
            println!(
                "frame {i} packet={} rgba={}",
                entry.packet,
                to_hex(&sha256(image.pixels()))
            );
            last = Some(entry.packet);
        }
        decoder.finish()?;
        println!(
            "verified {} nominal frames, {} decoded packets",
            index.frames.len(),
            index.packet_count()
        );
    } else if let Some((frame, out_path)) = frame_spec {
        let image = decoder.frame(frame)?;
        let bytes = if std::path::Path::new(&out_path)
            .extension()
            .is_some_and(|e| e == "rgba")
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
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-mgv --help' for usage.");
        std::process::exit(1);
    }
}
