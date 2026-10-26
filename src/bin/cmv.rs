//! Read-only CMV6 inspection and bounded JBPD frame decoding.
use cmvs_rs::cmv::{CmvIndex, KIND_JBPX, KIND_OGG};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::Write,
};

fn run() -> Result<(), Box<dyn Error>> {
    let path = env::args_os().nth(1).ok_or("usage: cmvs-cmv <movie.cmv>")?;
    if path == "--help" {
        println!(
            "Usage: cmvs-cmv <movie.cmv> [--frame <index> <new.png|new.rgba> | --check-frames]\nWithout options prints the CMV6 header/index summary. --frame decodes one JBPD frame and creates a new PNG (or raw RGBA when the extension is .rgba); existing files are never overwritten. --check-frames decodes every frame without exporting it. Ogg audio is located, not decoded. All game files read-only."
        );
        return Ok(());
    }
    let mut file = File::open(path)?;
    let index = CmvIndex::read(&mut file)?;
    let args: Vec<_> = env::args_os().skip(2).collect();
    if args.as_slice() == ["--check-frames"] {
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
    if !args.is_empty() {
        if args.len() != 3 || args[0] != "--frame" {
            return Err("usage: cmvs-cmv <movie.cmv> --frame <index> <new.png|new.rgba>".into());
        }
        let frame = args[1].to_str().ok_or("frame index is not text")?.parse()?;
        let image = index.decode_frame(&mut file, frame)?;
        let bytes = if std::path::Path::new(&args[2])
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
            .open(&args[2])?
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
        eprintln!("Error: {error}");
        std::process::exit(1)
    }
}
