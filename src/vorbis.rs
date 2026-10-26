//! Optional bounded streaming Vorbis adapter; codec implementation is lewton.
use crate::MAX_PAYLOAD;
use lewton::{inside_ogg::OggStreamReader, samples::InterleavedSamples};
use std::{error::Error, io::Cursor, sync::Arc};

pub struct Vorbis {
    reader: OggStreamReader<Cursor<Arc<[u8]>>>,
    channels: u16,
    rate: u32,
}
impl Vorbis {
    pub fn new(bytes: Arc<[u8]>) -> Result<Self, Box<dyn Error>> {
        if bytes.len() > MAX_PAYLOAD {
            return Err("Ogg resource exceeds research limit".into());
        }
        let reader = OggStreamReader::new(Cursor::new(bytes))?;
        let channels = u16::from(reader.ident_hdr.audio_channels);
        let rate = reader.ident_hdr.audio_sample_rate;
        if !(1..=2).contains(&channels) || !(8000..=192000).contains(&rate) {
            return Err("only mono/stereo Vorbis at 8..192 kHz is supported".into());
        }
        Ok(Self {
            reader,
            channels,
            rate,
        })
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn packet(&mut self) -> Result<Option<Vec<f32>>, Box<dyn Error>> {
        let packet = self
            .reader
            .read_dec_packet_generic::<InterleavedSamples<f32>>()?;
        if u16::from(self.reader.ident_hdr.audio_channels) != self.channels
            || self.reader.ident_hdr.audio_sample_rate != self.rate
        {
            return Err("changing Vorbis stream format is unsupported".into());
        }
        match packet {
            Some(packet)
                if packet.samples.len() > 65536
                    || !packet.samples.len().is_multiple_of(self.channels as usize)
                    || packet.samples.iter().any(|x| !x.is_finite()) =>
            {
                Err("invalid Vorbis PCM packet".into())
            }
            Some(packet) => Ok(Some(packet.samples)),
            None => Ok(None),
        }
    }
}
