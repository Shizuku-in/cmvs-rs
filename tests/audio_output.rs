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
