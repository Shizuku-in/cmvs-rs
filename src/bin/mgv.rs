use cmvs_core::{
    digest::{sha256, to_hex},
    mgv::{Decoder, MgvIndex},
};
use cmvs_rs::desktop::video::TheoraStream;
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
Build: cargo build --features desktop --bin cmvs-mgv
Example: cmvs-mgv op.mgv --frame 0 frame.png";

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if args.is_empty() {
        return Err("missing movie path".into());
    }
    if args[0].to_string_lossy().starts_with('-') {
        return Err(format!("unknown option {}", args[0].to_string_lossy()).into());
    }
    if !(args.len() == 1
        || (args.len() == 2 && args[1] == "--check-frames")
        || (args.len() == 4 && args[1] == "--frame"))
    {
        return Err("expected --check-frames or --frame <index> <new-file>".into());
    }
    let frame = if args.len() == 4 {
        Some(
            args[2]
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or("--frame index must be a non-negative decimal integer")?,
        )
    } else {
        None
    };
    let mut file = File::open(&args[0])?;
    let index = MgvIndex::read(&mut file)?;
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
    if args.len() == 1 {
        return Ok(());
    }
    let mut decoder = TheoraStream::open(Box::new(file), &index)?;
    if args.len() == 2 && args[1] == "--check-frames" {
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
    } else if let Some(frame) = frame {
        let image = decoder.frame(frame)?;
        let bytes = if std::path::Path::new(&args[3])
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
            .open(&args[3])?
            .write_all(&bytes)?;
    } else {
        return Err("invalid MGV options; use --help".into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}\nRun 'cmvs-mgv --help' for usage.");
        std::process::exit(1);
    }
}
