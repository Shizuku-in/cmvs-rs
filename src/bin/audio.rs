//! Read-only bounded Vorbis verification; no audio device is opened.
use cmvs_rs::{MAX_PAYLOAD, audio_output::VorbisProgram, cmv::CmvIndex};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
};
fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let path = args
        .next()
        .ok_or("usage: cmvs-audio <file.ogg|movie.cmv> [--pcm <new.f32>]")?;
    if path == "--help" {
        println!(
            "Usage: cmvs-audio <file.ogg|movie.cmv> [--pcm <new.f32>]\nDecode Vorbis read-only without opening a device. CMV input selects its Ogg resource. Optional output is interleaved little-endian f32 PCM; refuses overwrite. Reports channels, rate, sample count, duration and peak."
        );
        return Ok(());
    }
    let output = match (args.next(), args.next(), args.next()) {
        (None, None, None) => None,
        (Some(flag), Some(path), None) if flag == "--pcm" => Some(path),
        _ => return Err("invalid audio inspector options".into()),
    };
    let mut file = File::open(path)?;
    let mut signature = [0; 4];
    file.read_exact(&mut signature)?;
    file.seek(SeekFrom::Start(0))?;
    let bytes = if &signature == b"CMV6" {
        let index = CmvIndex::read(&mut file)?;
        let audio = index.audio().ok_or("CMV without audio")?;
        if audio.stored_size as usize > MAX_PAYLOAD {
            return Err("CMV audio exceeds limit".into());
        }
        file.seek(SeekFrom::Start(audio.file_offset(&index.header)))?;
        let mut bytes = vec![0; audio.stored_size as usize];
        file.read_exact(&mut bytes)?;
        bytes
    } else {
        let mut bytes = vec![];
        file.take(MAX_PAYLOAD as u64 + 1).read_to_end(&mut bytes)?;
        bytes
    };
    let mut program = VorbisProgram::new(bytes, None, false)?;
    let mut output = output
        .map(|p| OpenOptions::new().write(true).create_new(true).open(p))
        .transpose()?
        .map(std::io::BufWriter::new);
    let (mut count, mut peak) = (0u64, 0f32);
    while let Some(sample) = program.next_sample()? {
        count += 1;
        peak = peak.max(sample.abs());
        if let Some(out) = &mut output {
            out.write_all(&sample.to_le_bytes())?;
        }
    }
    if let Some(out) = &mut output {
        out.flush()?;
    }
    println!(
        "Vorbis channels={} rate={} samples={} frames={} duration_ms={} peak={peak}",
        program.channels(),
        program.sample_rate(),
        count,
        count / u64::from(program.channels()),
        count * 1000 / u64::from(program.channels()) / u64::from(program.sample_rate())
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
