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
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!(
            "Usage: cmvs-mgv <movie.mgv> [--check-frames | --frame <index> <new.png|new.rgba>]"
        );
        return Ok(());
    }
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
    } else if args.len() == 4 && args[1] == "--frame" {
        let frame = args[2].to_str().ok_or("frame is not text")?.parse()?;
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
