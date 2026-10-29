#![cfg(feature = "audio-output")]
use cmvs_rs::{
    audio_output::{Envelope, VorbisProgram},
    vorbis::Vorbis,
};
use std::sync::Arc;
const FIRST: &[u8] = include_bytes!("fixtures/audio/mono440.ogg");
const SECOND: &[u8] = include_bytes!("fixtures/audio/mono880.ogg");
fn samples(bytes: &[u8]) -> Vec<f32> {
    let mut program = VorbisProgram::new(bytes.to_vec(), None, false).unwrap();
    let mut samples = vec![];
    while let Some(v) = program.next_sample().unwrap() {
        samples.push(v);
    }
    samples
}
#[test]
fn vorbis_decodes_synthetic_packets_and_reports_format() {
    let mut reader = Vorbis::new(Arc::from(FIRST)).unwrap();
    assert_eq!(reader.channels(), 1);
    assert_eq!(reader.sample_rate(), 8000);
    let mut decoded = vec![];
    while let Some(packet) = reader.packet().unwrap() {
        decoded.extend(packet);
    }
    assert_eq!(decoded.len(), 768);
    assert!(decoded.iter().all(|x| x.is_finite()));
    assert!(decoded.iter().any(|&v| v > 0.1));
    assert!(decoded.iter().any(|&v| v < -0.1));
    assert_eq!(decoded, samples(FIRST));
}
#[test]
fn two_file_bgm_plays_intro_once_and_repeats_second() {
    let first = samples(FIRST);
    let second = samples(SECOND);
    let mut program = VorbisProgram::new(FIRST.to_vec(), Some(SECOND.to_vec()), true).unwrap();
    let actual: Vec<_> = (0..first.len() + second.len() * 3)
        .map(|_| program.next_sample().unwrap().unwrap())
        .collect();
    assert_eq!(&actual[..first.len()], first);
    for cycle in 0..3 {
        assert_eq!(
            &actual[first.len() + cycle * second.len()..first.len() + (cycle + 1) * second.len()],
            second
        );
    }
    let mut program = VorbisProgram::new(FIRST.to_vec(), Some(SECOND.to_vec()), false).unwrap();
    for v in first {
        assert_eq!(program.next_sample().unwrap(), Some(v));
    }
    assert_eq!(program.next_sample().unwrap(), None);
}
#[test]
fn single_file_loop_and_exhausted_source_are_stable() {
    let first = samples(FIRST);
    let mut program = VorbisProgram::new(FIRST.to_vec(), None, true).unwrap();
    for _ in 0..3 {
        for &v in &first {
            assert_eq!(program.next_sample().unwrap(), Some(v));
        }
    }
    let mut program = VorbisProgram::new(FIRST.to_vec(), None, false).unwrap();
    for _ in 0..first.len() {
        program.next_sample().unwrap();
    }
    for _ in 0..3 {
        assert_eq!(program.next_sample().unwrap(), None);
    }
}
#[test]
fn malformed_headers_and_packets_fail_before_output() {
    for bytes in [vec![], b"OggS".to_vec(), FIRST[..100].to_vec()] {
        assert!(VorbisProgram::new(bytes, None, false).is_err());
    }
    assert!(VorbisProgram::new(FIRST.to_vec(), Some(b"bad secondary".to_vec()), true).is_err());
    let mut corrupt = FIRST.to_vec();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xff;
    let result = VorbisProgram::new(corrupt, None, false);
    if let Ok(mut program) = result {
        let mut failed = false;
        for _ in 0..2048 {
            match program.next_sample() {
                Err(_) => {
                    failed = true;
                    break;
                }
                Ok(None) => break,
                _ => {}
            }
        }
        assert!(failed);
    }
}
#[test]
fn fade_envelopes_reach_exact_endpoints_without_overshoot() {
    let fade = Envelope::new(0.75, 0.0, 2000);
    assert_eq!(fade.value_at(0), 0.75);
    assert_eq!(fade.value_at(1000), 0.375);
    assert_eq!(fade.value_at(2000), 0.0);
    assert_eq!(fade.value_at(u32::MAX), 0.0);
    assert_eq!(Envelope::new(0.0, 1.0, 0).value_at(0), 1.0);
}

/// Explicit hardware diagnostic: synthetic quiet tones only, no game files or window.
/// Keep ignored so normal offline/CI tests never open a device or require audio hardware.
#[test]
#[ignore = "opens the default native audio device; run explicitly outside restricted sandboxes"]
fn native_device_smoke() -> Result<(), Box<dyn std::error::Error>> {
    use cmvs_rs::{
        audio::{AudioBackend, AudioRequest, MovieAudioClock},
        audio_output::NativeAudio,
    };
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        thread,
        time::{Duration, Instant},
    };
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            for name in ["first.ogg", "second.ogg"] {
                let _ = fs::remove_file(self.0.join("sound").join(name));
            }
            let _ = fs::remove_dir(self.0.join("sound"));
            let _ = fs::remove_dir(&self.0);
        }
    }
    fn pump(audio: &mut NativeAudio) -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..30 {
            thread::sleep(Duration::from_millis(10));
            audio.update()?;
        }
        Ok(())
    }
    let path = std::env::temp_dir().join(format!("cmvs-native-audio-smoke-{}", std::process::id()));
    fs::create_dir(&path)?;
    let scratch = Scratch(path);
    fs::create_dir(scratch.0.join("sound"))?;
    fs::write(scratch.0.join("sound/first.ogg"), FIRST)?;
    fs::write(scratch.0.join("sound/second.ogg"), SECOND)?;
    // Declared after scratch so channels and the device drop before deleting the files.
    let mut audio = NativeAudio::new(&scratch.0)?;
    audio.configure_paths(&BTreeMap::from([
        ((0x2012, 0), b"sound\\".to_vec()),
        ((0x2013, 0), b"sound\\".to_vec()),
        ((0x2014, 0), b"sound\\".to_vec()),
    ]))?;
    println!("# smoke: prepare/start SE");
    assert!(audio.play(&AudioRequest::PlaySe {
        channel: 0,
        name: b"first.ogg".to_vec(),
        volume: 16,
        looped: false,
        flag: false,
        fade_in_ms: 0,
    })?);
    audio.start(0, false)?;
    pump(&mut audio)?;
    audio.stop(0)?;
    println!("# smoke: voice");
    assert!(audio.play_voice(&AudioRequest::PlayVoice {
        channel: 0,
        name: b"first.ogg".to_vec(),
        volume: 16,
        start: true,
    })?);
    pump(&mut audio)?;
    audio.stop_voice(0)?;
    println!("# smoke: two-file looping BGM and fade");
    assert_eq!(audio.bgm_position(0)?, None);
    assert_eq!(audio.bgm_position(1)?, None);
    assert!(audio.bgm_position(2).is_err());
    audio.play_bgm(&AudioRequest::PlayBgm {
        channel: 0,
        previous: 1,
        primary: b"first.ogg".to_vec(),
        secondary: b"second.ogg".to_vec(),
        volume: 16,
        fade_ms: 0,
        looped: true,
        flag_1576: 0,
    })?;
    pump(&mut audio)?;
    assert!(audio.bgm_position(0)?.is_some_and(|ms| ms > 0));
    assert_eq!(audio.bgm_position(1)?, None);
    println!("# smoke: switch BGM channels, then fade and stop");
    audio.play_bgm(&AudioRequest::PlayBgm {
        channel: 1,
        previous: 0,
        primary: b"second.ogg".to_vec(),
        secondary: vec![],
        volume: 16,
        fade_ms: 0,
        looped: true,
        flag_1576: 0,
    })?;
    pump(&mut audio)?;
    assert_eq!(audio.bgm_position(0)?, None);
    assert!(audio.bgm_position(1)?.is_some_and(|ms| ms > 0));
    audio.fade_out_bgm(1, 20)?;
    pump(&mut audio)?;
    assert_eq!(audio.bgm_position(1)?, None);
    audio.stop_bgm()?;
    assert_eq!(audio.bgm_position(0)?, None);
    assert_eq!(audio.bgm_position(1)?, None);
    println!("# smoke: movie audio clock and EOF");
    audio.play_movie_audio(&AudioRequest::PlayMovieAudio {
        path: b"sound/first.ogg".to_vec(),
        offset: 0,
        size: FIRST.len() as u32,
        volume: 16,
    })?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while audio.movie_clock()? != Some(MovieAudioClock::Ended) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
        audio.update()?;
    }
    assert_eq!(audio.movie_clock()?, Some(MovieAudioClock::Ended));
    audio.stop_movie_audio()?;
    println!("# smoke: all native channels completed; releasing device");
    Ok(())
}
