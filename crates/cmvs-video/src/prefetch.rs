//! Bounded video lookahead on a worker thread. Cached frames can be displayed while
//! the worker decodes a more expensive packet; explicit misses still wait and report errors.
use crate::TheoraStream;
use cmvs_core::{
    mgv::{self, MgvIndex, ReadSeek},
    raster::RgbaImage,
};
use std::{
    collections::VecDeque,
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread,
};

/// Opens each movie on its own decoding thread. `on_thread_start` runs first on that
/// thread, e.g. to move it to a CPU core the host's main loop does not use.
#[derive(Debug, Clone, Copy)]
pub struct PrefetchTheoraFactory {
    pub on_thread_start: fn(),
}
impl Default for PrefetchTheoraFactory {
    fn default() -> Self {
        Self {
            on_thread_start: || {},
        }
    }
}

#[derive(Clone)]
enum CachedFrame {
    Rgba(RgbaImage),
    Yuv(Arc<crate::theora::Frame>),
}
impl CachedFrame {
    fn rgba(
        &self,
        ident: &crate::TheoraIdentHeader,
        index: &MgvIndex,
    ) -> Result<RgbaImage, Box<dyn Error>> {
        match self {
            Self::Rgba(image) => Ok(image.clone()),
            Self::Yuv(frame) => Ok(crate::to_rgba(ident, frame, index)?),
        }
    }
}

struct Cache {
    frames: Mutex<VecDeque<(u32, CachedFrame)>>,
    wanted: AtomicU32,
    capacity: usize,
    yuv: bool,
}
impl Cache {
    fn get(&self, packet: u32) -> Option<CachedFrame> {
        let mut frames = self.frames.lock().unwrap();
        frames.retain(|(p, _)| *p >= packet);
        frames
            .iter()
            .find(|(p, _)| *p == packet)
            .map(|(_, image)| image.clone())
    }
    fn insert(&self, packet: u32, image: CachedFrame) {
        let wanted = self.wanted.load(Ordering::Relaxed);
        let mut frames = self.frames.lock().unwrap();
        frames.retain(|(p, _)| *p >= wanted && *p != packet);
        frames.push_back((packet, image));
        while frames.len() > self.capacity {
            frames.pop_front();
        }
    }
}

#[derive(Debug)]
struct BufferedFactory {
    factory: PrefetchTheoraFactory,
    bytes: usize,
    frames: usize,
    yuv: bool,
}
impl PrefetchTheoraFactory {
    /// Bounds the YUV lookahead by bytes and frame count (minimum two frames).
    /// RGBA conversion happens only for the displayed frame, outside the cache lock.
    pub fn with_lookahead(self, bytes: usize, frames: usize) -> impl mgv::DecoderFactory {
        BufferedFactory {
            factory: self,
            bytes,
            frames,
            yuv: true,
        }
    }
}
impl mgv::DecoderFactory for PrefetchTheoraFactory {
    fn open(
        &self,
        reader: Box<dyn ReadSeek>,
        index: &MgvIndex,
    ) -> Result<Box<dyn mgv::Decoder>, Box<dyn Error>> {
        BufferedFactory {
            factory: *self,
            bytes: 64 << 20,
            frames: 16,
            yuv: false,
        }
        .open(reader, index)
    }
}
impl mgv::DecoderFactory for BufferedFactory {
    fn open(
        &self,
        reader: Box<dyn ReadSeek>,
        index: &MgvIndex,
    ) -> Result<Box<dyn mgv::Decoder>, Box<dyn Error>> {
        let stream = TheoraStream::open(reader, index)?;
        // YUV420 lookahead uses 3/2 bytes per coded pixel instead of 4 for RGBA.
        // Very large frames retain the stream's existing minimum two-frame cache.
        let bytes = if self.yuv {
            stream.ident.width as u64 * stream.ident.height as u64 * 3 / 2
        } else {
            u64::from(index.width) * u64::from(index.height) * 4
        };
        let capacity = (self.bytes as u64 / bytes).clamp(2, self.frames.max(2) as u64) as usize;
        let ident = stream.ident.clone();
        let cache = Arc::new(Cache {
            frames: Mutex::new(VecDeque::new()),
            wanted: AtomicU32::new(u32::MAX),
            capacity,
            yuv: self.yuv,
        });
        let worker_cache = cache.clone();
        let (requests, worker_requests) = sync_channel(1);
        let (worker_replies, replies) = sync_channel(1);
        let start = self.factory.on_thread_start;
        thread::Builder::new()
            .name("mgv-decode".into())
            .spawn(move || {
                start();
                serve(stream, &worker_requests, &worker_replies, &worker_cache);
            })?;
        Ok(Box::new(PrefetchDecoder {
            requests,
            replies,
            cache,
            index: index.clone(),
            ident,
        }))
    }
}

enum Request {
    Frame(u32),
    Finish,
    Wake,
}
type Reply = Result<Option<RgbaImage>, String>;

fn serve(
    mut stream: TheoraStream,
    requests: &Receiver<Request>,
    replies: &SyncSender<Reply>,
    cache: &Cache,
) {
    use mgv::Decoder;
    loop {
        let request = match requests.try_recv() {
            Ok(request) => request,
            Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {
                let wanted = cache.wanted.load(Ordering::Relaxed);
                if !stream.failed
                    && wanted != u32::MAX
                    && stream.next_packet > wanted
                    && stream.next_packet - wanted < cache.capacity as u32
                    && stream.next_packet < stream.index.packet_count()
                {
                    let packet = stream.next_packet;
                    if cache.yuv {
                        if let Ok(frame) = stream.prefetch_yuv() {
                            cache.insert(packet, CachedFrame::Yuv(Arc::new(frame)));
                        }
                    } else {
                        stream.prefetch();
                        if let Some(image) = stream.recent(packet) {
                            cache.insert(packet, CachedFrame::Rgba(image.clone()));
                        }
                    }
                    continue;
                }
                match requests.recv() {
                    Ok(request) => request,
                    Err(_) => return,
                }
            }
        };
        let reply = match request {
            Request::Wake => continue,
            Request::Frame(frame) => {
                let packet = u32::from(stream.index.frames[frame as usize].packet);
                let image = if let Some(image) = cache.get(packet) {
                    image.rgba(&stream.ident, &stream.index)
                } else {
                    // A real backwards seek or a jump beyond lookahead may replay.
                    cache.frames.lock().unwrap().clear();
                    stream.frame(frame)
                };
                image.map(|image| {
                    cache.insert(packet, CachedFrame::Rgba(image.clone()));
                    Some(image)
                })
            }
            Request::Finish => stream.finish().map(|()| None),
        };
        if replies.send(reply.map_err(|e| e.to_string())).is_err() {
            return;
        }
    }
}

/// A cache hit never waits for the in-progress packet. Dropping the decoder detaches
/// its worker, which exits after the current packet when the request channel closes.
struct PrefetchDecoder {
    requests: SyncSender<Request>,
    replies: Receiver<Reply>,
    cache: Arc<Cache>,
    index: MgvIndex,
    ident: crate::TheoraIdentHeader,
}
impl fmt::Debug for PrefetchDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrefetchDecoder").finish_non_exhaustive()
    }
}
impl PrefetchDecoder {
    fn call(&self, request: Request) -> Result<Option<RgbaImage>, Box<dyn Error>> {
        const GONE: &str = "MGV decoding thread stopped";
        self.requests.send(request).map_err(|_| GONE)?;
        Ok(self.replies.recv().map_err(|_| GONE)??)
    }
}
impl mgv::Decoder for PrefetchDecoder {
    fn frame(&mut self, frame: u32) -> Result<RgbaImage, Box<dyn Error>> {
        let packet = self
            .index
            .frames
            .get(frame as usize)
            .ok_or(cmvs_core::FormatError("MGV nominal frame out of range"))?
            .packet;
        let packet = u32::from(packet);
        self.cache.wanted.store(packet, Ordering::Relaxed);
        if let Some(image) = self.cache.get(packet) {
            // Only a wake notification is queued. The atomic always holds the latest
            // target, so cached requests cannot build an unbounded request backlog.
            let _ = self.requests.try_send(Request::Wake);
            let image = image.rgba(&self.ident, &self.index)?;
            self.cache.insert(packet, CachedFrame::Rgba(image.clone()));
            return Ok(image);
        }
        Ok(self
            .call(Request::Frame(frame))?
            .expect("frame request returns an image"))
    }
    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        self.cache.wanted.store(u32::MAX, Ordering::Relaxed);
        self.call(Request::Finish).map(|_| ())
    }
}
