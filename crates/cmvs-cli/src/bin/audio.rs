//! Read-only bounded audio inspection without opening a device.
use cmvs_audio::AudioProgram;
use cmvs_core::{MAX_PAYLOAD, format::cmv::CmvIndex};
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
};

const USAGE: &str = "\
Usage: cmvs-audio <audio-or-movie> [--pcm <new.f32>]

Inspect Ogg Vorbis, PCM WAVE, MV2X or a CMV6 Ogg soundtrack without opening
an audio device. Print channel count, sample rate, duration and peak amplitude.

Options:
  --pcm <new.f32>  export interleaved little-endian f32 PCM (no header)
  -h, --help       show this help

Output files must not already exist. Input files are read only.
Build: cargo build --features audio-output --bin cmvs-audio
Example: cmvs-audio voice.ogg --pcm voice.f32";

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let all_args: Vec<_> = env::args_os().skip(1).collect();
    if cmvs_cli::wants_help(&all_args) {
        println!("{USAGE}");
        return Ok(());
    }
    let mut args = all_args.into_iter();
    let mut positional = false;
    let mut path = None;
    let mut output = None;
    while let Some(arg) = args.next() {
        if !positional && arg == "--" {
            positional = true;
        } else if !positional && arg == "--pcm" {
            if output.is_some() {
                return Err("duplicate --pcm option".into());
            }
            output = Some(args.next().ok_or("--pcm requires an output path")?);
        } else if !positional && arg.to_string_lossy().starts_with('-') {
            return Err(format!("unknown option {}", arg.to_string_lossy()).into());
        } else if path.is_none() {
            path = Some(arg);
        } else {
            return Err("unexpected extra audio inspector argument".into());
        }
    }
    let path = path.ok_or("missing audio or movie path")?;
    let mut file = File::open(path)?;
    let mut signature = [0; 4];
    file.read_exact(&mut signature)?;
    file.seek(SeekFrom::Start(0))?;
    let bytes = if &signature == b"CMV6" {
        let index = CmvIndex::read_io(&mut file)?;
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
    let mut program = AudioProgram::new(bytes, None, false)?;
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
        eprintln!("Error: {error}\nRun 'cmvs-audio --help' for usage.");
        std::process::exit(1);
    }
}
