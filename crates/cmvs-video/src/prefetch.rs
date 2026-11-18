//! Bounded video lookahead on a worker thread. Cached frames can be displayed while
//! the worker decodes a more expensive packet; explicit misses still wait and report errors.
use crate::TheoraStream;
use cmvs_core::{
    format::mgv::{self, MgvIndex, ReadSeek},
    render::raster::RgbaImage,
};
use std::{
    collections::VecDeque,
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread,
};

/// Future packets beyond `wanted` that the worker pre-converts to RGBA in
/// `with_lookahead` mode whenever `budget_bytes` permits, keeping `Decoder::frame()`
/// cache hits allocation- and conversion-free on the calling thread while deeper
/// lookahead stays in compact YUV420.
const RGBA_LOOKAHEAD: u32 = 2;

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

fn yuv_frame_bytes(frame: &crate::theora::Frame) -> usize {
    frame.planes.iter().map(Vec::len).sum()
}

enum CachedFrame {
    Rgba(RgbaImage),
    Yuv(crate::theora::Frame),
}
impl CachedFrame {
    fn byte_len(&self) -> usize {
        match self {
            Self::Rgba(image) => image.pixels().len(),
            Self::Yuv(frame) => yuv_frame_bytes(frame),
        }
    }
    fn rgba(
        self,
        ident: &crate::TheoraIdentHeader,
        index: &MgvIndex,
    ) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
        match self {
            Self::Rgba(image) => Ok(image),
            Self::Yuv(frame) => Ok(crate::to_rgba(ident, &frame, index)?),
        }
    }
}

struct CacheState {
    frames: VecDeque<(u32, CachedFrame)>,
    bytes: usize,
    wanted: u32,
    awaited: Option<u32>,
    #[cfg(test)]
    peak_bytes: usize,
}
impl CacheState {
    fn new() -> Self {
        Self {
            frames: VecDeque::new(),
            bytes: 0,
            wanted: u32::MAX,
            awaited: None,
            #[cfg(test)]
            peak_bytes: 0,
        }
    }
    fn record_bytes(&mut self, bytes: usize) {
        self.bytes = bytes;
        #[cfg(test)]
        {
            self.peak_bytes = self.peak_bytes.max(bytes);
        }
    }
    fn retain_wanted(&mut self) {
        let wanted = self.wanted;
        let awaited = self.awaited;
        let mut removed = 0usize;
        self.frames.retain(|(p, frame)| {
            if *p > wanted || awaited == Some(*p) {
                true
            } else {
                removed += frame.byte_len();
                false
            }
        });
        self.bytes -= removed;
    }
    fn clear(&mut self) {
        self.frames.clear();
        self.bytes = 0;
    }
    fn insert_or_replace(
        &mut self,
        packet: u32,
        frame: CachedFrame,
        budget_bytes: usize,
        max_frames: usize,
    ) {
        if self.wanted == u32::MAX {
            return;
        }
        self.retain_wanted();
        if packet <= self.wanted && self.awaited != Some(packet) {
            return;
        }
        let new_bytes = frame.byte_len();
        if let Some((_, existing)) = self.frames.iter_mut().find(|(p, _)| *p == packet) {
            let old_bytes = existing.byte_len();
            let updated = self.bytes - old_bytes + new_bytes;
            if updated <= budget_bytes {
                *existing = frame;
                self.record_bytes(updated);
            }
            return;
        }
        if self.frames.len() < max_frames && self.bytes + new_bytes <= budget_bytes {
            let pos = self.frames.partition_point(|(p, _)| *p < packet);
            self.frames.insert(pos, (packet, frame));
            self.record_bytes(self.bytes + new_bytes);
        }
    }
}

enum WorkerPlan {
    Promote(u32, crate::theora::Frame),
    DecodeRgba,
    DecodeYuv,
}

struct Cache {
    state: Mutex<CacheState>,
    shutdown: AtomicBool,
    budget_bytes: usize,
    max_frames: usize,
    yuv: bool,
}
impl Cache {
    fn pop_packet(state: &mut CacheState, packet: u32) -> Option<CachedFrame> {
        let pos = state.frames.iter().position(|(p, _)| *p == packet)?;
        let (_, frame) = state.frames.remove(pos).expect("validated index");
        state.bytes -= frame.byte_len();
        Some(frame)
    }
    /// Pops `packet` for `PrefetchDecoder`, or marks it awaited if missing so an in-flight
    /// worker decode preserves `packet` for the incoming `Request::Frame`.
    fn take_or_await(&self, packet: u32) -> Option<CachedFrame> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.wanted = packet;
        let taken = Self::pop_packet(&mut state, packet);
        state.awaited = if taken.is_some() { None } else { Some(packet) };
        state.retain_wanted();
        taken
    }
    /// Pops `packet` while servicing `Request::Frame` on the worker and clears `awaited`.
    fn take_for_request(&self, packet: u32) -> Option<CachedFrame> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.wanted = packet;
        state.awaited = None;
        let taken = Self::pop_packet(&mut state, packet);
        state.retain_wanted();
        taken
    }
    fn insert_or_replace(&self, packet: u32, frame: CachedFrame) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.insert_or_replace(packet, frame, self.budget_bytes, self.max_frames);
    }
    fn clear(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    fn reset_wanted(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.wanted = u32::MAX;
        state.awaited = None;
        state.clear();
    }
    fn plan_work(
        &self,
        stream_failed: bool,
        next_packet: u32,
        packet_count: u32,
        rgba_bytes: usize,
        yuv_bytes: usize,
    ) -> Option<WorkerPlan> {
        if stream_failed || self.shutdown.load(Ordering::Acquire) {
            return None;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let wanted = state.wanted;
        if wanted == u32::MAX || state.awaited == Some(wanted) {
            return None;
        }
        state.retain_wanted();

        // Claim the YUV under the lock. A request arriving during conversion then
        // awaits this worker's result instead of converting the same packet again.
        let rgba_limit = wanted.saturating_add(RGBA_LOOKAHEAD);
        let upgrade = state
            .frames
            .iter()
            .take_while(|(packet, _)| *packet <= rgba_limit)
            .position(|(_, frame)| {
                matches!(frame, CachedFrame::Yuv(_))
                    && state.bytes - frame.byte_len() + rgba_bytes <= self.budget_bytes
            });
        if let Some(pos) = upgrade
            && let Some((packet, CachedFrame::Yuv(yuv))) = state.frames.remove(pos)
        {
            state.bytes -= yuv_frame_bytes(&yuv);
            return Some(WorkerPlan::Promote(packet, yuv));
        }

        if next_packet <= wanted
            || next_packet >= packet_count
            || state.frames.len() >= self.max_frames
        {
            return None;
        }
        let want_rgba = !self.yuv || next_packet - wanted <= RGBA_LOOKAHEAD;
        if want_rgba && state.bytes + rgba_bytes <= self.budget_bytes {
            Some(WorkerPlan::DecodeRgba)
        } else if self.yuv && state.bytes + yuv_bytes <= self.budget_bytes {
            Some(WorkerPlan::DecodeYuv)
        } else {
            None
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
    /// Bounds lookahead of upcoming packets (`> current`) by exact cached byte usage
    /// (`bytes`) and frame count (`frames`, minimum two frames). Near-future frames are
    /// pre-converted to RGBA on the worker thread when `bytes` permits, while deeper
    /// lookahead remains YUV420; the decoder retains the current packet's RGBA image
    /// directly so repeated nominal frames never rewind even at zero lookahead budget.
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
    ) -> Result<Box<dyn mgv::Decoder>, Box<dyn Error + Send + Sync>> {
        BufferedFactory {
            factory: *self,
            bytes: 64 << 20,
            frames: 16,
            yuv: false,
        }
        .open(reader, index)
    }
}
impl BufferedFactory {
    fn open_worker(
        &self,
        reader: Box<dyn ReadSeek>,
        index: &MgvIndex,
    ) -> Result<Result<PrefetchDecoder, TheoraStream>, Box<dyn Error + Send + Sync>> {
        let stream = TheoraStream::open(reader, index)?;
        let rgba_bytes = usize::from(index.width) * usize::from(index.height) * 4;
        let yuv_bytes = yuv_frame_bytes(stream.decoder.current_frame());
        let ident = stream.ident.clone();
        let cache = Arc::new(Cache {
            state: Mutex::new(CacheState::new()),
            shutdown: AtomicBool::new(false),
            budget_bytes: self.bytes,
            max_frames: self.frames.max(2),
            yuv: self.yuv,
        });
        let worker_cache = cache.clone();
        let (requests, worker_requests) = sync_channel(1);
        let (worker_replies, replies) = sync_channel(1);
        let start = self.factory.on_thread_start;
        let stream_slot = Arc::new(Mutex::new(Some(stream)));
        let worker_stream = Arc::clone(&stream_slot);
        if thread::Builder::new()
            .name("mgv-decode".into())
            .spawn(move || {
                let Some(stream) = worker_stream
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                else {
                    return;
                };
                start();
                serve(
                    stream,
                    &worker_requests,
                    &worker_replies,
                    &worker_cache,
                    rgba_bytes,
                    yuv_bytes,
                );
            })
            .is_err()
        {
            let stream = stream_slot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .expect("stream still present after failed spawn");
            return Ok(Err(stream));
        }
        Ok(Ok(PrefetchDecoder {
            requests,
            replies,
            cache,
            index: index.clone(),
            ident,
            current: None,
        }))
    }

    #[cfg(test)]
    fn open_prefetch(
        &self,
        reader: Box<dyn ReadSeek>,
        index: &MgvIndex,
    ) -> Result<PrefetchDecoder, Box<dyn Error + Send + Sync>> {
        match self.open_worker(reader, index)? {
            Ok(decoder) => Ok(decoder),
            Err(_) => Err("failed to spawn MGV decoding thread".into()),
        }
    }
}
impl mgv::DecoderFactory for BufferedFactory {
    fn open(
        &self,
        reader: Box<dyn ReadSeek>,
        index: &MgvIndex,
    ) -> Result<Box<dyn mgv::Decoder>, Box<dyn Error + Send + Sync>> {
        match self.open_worker(reader, index)? {
            Ok(decoder) => Ok(Box::new(decoder)),
            Err(stream) => Ok(Box::new(stream)),
        }
    }
}

enum Request {
    Frame(u32),
    Finish,
    Wake,
}
type Reply = Result<Option<RgbaImage>, String>;

fn decode_rgba(stream: &mut TheoraStream) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
    stream.decode_packet()?;
    stream.recent = [None, None];
    match crate::to_rgba(&stream.ident, stream.decoder.current_frame(), &stream.index) {
        Ok(image) => Ok(image),
        Err(err) => {
            stream.failed = true;
            Err(err.into())
        }
    }
}

fn serve(
    mut stream: TheoraStream,
    requests: &Receiver<Request>,
    replies: &SyncSender<Reply>,
    cache: &Cache,
    rgba_bytes: usize,
    yuv_bytes: usize,
) {
    use mgv::Decoder;
    let packet_count = stream.index.packet_count();
    loop {
        if cache.shutdown.load(Ordering::Acquire) {
            return;
        }
        let request = match requests.try_recv() {
            Ok(request) => request,
            Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {
                if let Some(plan) = cache.plan_work(
                    stream.failed,
                    stream.next_packet,
                    packet_count,
                    rgba_bytes,
                    yuv_bytes,
                ) {
                    match plan {
                        WorkerPlan::Promote(packet, yuv) => {
                            match crate::to_rgba(&stream.ident, &yuv, &stream.index) {
                                Ok(image) => {
                                    if !cache.shutdown.load(Ordering::Acquire) {
                                        cache.insert_or_replace(packet, CachedFrame::Rgba(image));
                                    }
                                }
                                Err(_) => {
                                    stream.failed = true;
                                }
                            }
                        }
                        WorkerPlan::DecodeRgba => {
                            let packet = stream.next_packet;
                            if let Ok(image) = decode_rgba(&mut stream)
                                && !cache.shutdown.load(Ordering::Acquire)
                            {
                                cache.insert_or_replace(packet, CachedFrame::Rgba(image));
                            }
                        }
                        WorkerPlan::DecodeYuv => {
                            let packet = stream.next_packet;
                            if let Ok(frame) = stream.prefetch_yuv()
                                && !cache.shutdown.load(Ordering::Acquire)
                            {
                                cache.insert_or_replace(packet, CachedFrame::Yuv(frame));
                            }
                        }
                    }
                    continue;
                }
                match requests.recv() {
                    Ok(request) if !cache.shutdown.load(Ordering::Acquire) => request,
                    _ => return,
                }
            }
        };
        let reply = match request {
            Request::Wake => continue,
            Request::Frame(frame) => {
                let packet = u32::from(stream.index.frames[frame as usize].packet);
                let image = if let Some(cached) = cache.take_for_request(packet) {
                    cached.rgba(&stream.ident, &stream.index)
                } else {
                    // A real backwards seek or a jump beyond lookahead may replay.
                    cache.clear();
                    let res = stream.frame(frame);
                    stream.recent = [None, None];
                    res
                };
                image.map(Some)
            }
            Request::Finish => {
                cache.clear();
                let res = stream.finish().map(|()| None);
                stream.recent = [None, None];
                res
            }
        };
        if replies.send(reply.map_err(|e| e.to_string())).is_err() {
            return;
        }
    }
}

/// A host reader can block indefinitely. Drop signals shutdown and disconnects both
/// channels; the detached worker releases its stream when the pending read returns.
struct PrefetchDecoder {
    requests: SyncSender<Request>,
    replies: Receiver<Reply>,
    cache: Arc<Cache>,
    index: MgvIndex,
    ident: crate::TheoraIdentHeader,
    /// Retains the most recently displayed nominal packet so repeated nominal frames
    /// never rewind or consume future lookahead budget.
    current: Option<(u32, RgbaImage)>,
}
impl fmt::Debug for PrefetchDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrefetchDecoder").finish_non_exhaustive()
    }
}
impl Drop for PrefetchDecoder {
    fn drop(&mut self) {
        self.cache.shutdown.store(true, Ordering::Release);
    }
}
impl PrefetchDecoder {
    fn call(&self, request: Request) -> Result<Option<RgbaImage>, Box<dyn Error + Send + Sync>> {
        const GONE: &str = "MGV decoding thread stopped";
        self.requests.send(request).map_err(|_| GONE)?;
        Ok(self.replies.recv().map_err(|_| GONE)??)
    }
}
impl mgv::Decoder for PrefetchDecoder {
    fn frame(&mut self, frame: u32) -> Result<RgbaImage, Box<dyn Error + Send + Sync>> {
        let packet = self
            .index
            .frames
            .get(frame as usize)
            .ok_or(cmvs_core::FormatError("MGV nominal frame out of range"))?
            .packet;
        let packet = u32::from(packet);
        if let Some((cur_packet, image)) = &self.current
            && *cur_packet == packet
        {
            return Ok(image.clone());
        }
        if let Some(cached) = self.cache.take_or_await(packet) {
            // Only a wake notification is queued. The cache state always holds the
            // latest target, so cached requests cannot build an unbounded request backlog.
            let _ = self.requests.try_send(Request::Wake);
            let image = cached.rgba(&self.ident, &self.index)?;
            self.current = Some((packet, image.clone()));
            return Ok(image);
        }
        let image = self
            .call(Request::Frame(frame))?
            .expect("frame request returns an image");
        self.current = Some((packet, image.clone()));
        Ok(image)
    }
    fn finish(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.current = None;
        self.cache.reset_wanted();
        self.call(Request::Finish).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmvs_core::format::mgv::Decoder;
    use std::{
        io::Cursor,
        time::{Duration, Instant},
    };

    fn fixture() -> Vec<u8> {
        include_str!("../tests/fixtures/mgv1.hex")
            .trim()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn request_during_promotion_consumes_the_workers_converted_frame() {
        let cache = Cache {
            state: Mutex::new(CacheState::new()),
            shutdown: AtomicBool::new(false),
            budget_bytes: 16,
            max_frames: 2,
            yuv: true,
        };
        cache.take_for_request(0);
        let yuv = crate::theora::Frame {
            planes: [vec![0; 4], vec![0], vec![0]],
        };
        cache.insert_or_replace(1, CachedFrame::Yuv(yuv));
        assert!(matches!(
            cache.plan_work(false, 2, 2, 16, 6),
            Some(WorkerPlan::Promote(1, _))
        ));
        assert!(cache.take_or_await(1).is_none());
        let image = RgbaImage::new(2, 2, vec![128; 16]).unwrap();
        cache.insert_or_replace(1, CachedFrame::Rgba(image.clone()));
        assert!(
            matches!(cache.take_for_request(1), Some(CachedFrame::Rgba(result)) if result == image)
        );
    }

    #[test]
    fn lookahead_preconverts_on_worker_and_strictly_honors_byte_budget() {
        let bytes = fixture();
        let index = MgvIndex::read(&mut Cursor::new(&bytes)).unwrap();
        let rgba_bytes = usize::from(index.width) * usize::from(index.height) * 4;
        let yuv_bytes = rgba_bytes * 3 / 8;
        assert_eq!(rgba_bytes, 4096);
        assert_eq!(yuv_bytes, 1536);

        let mut reference =
            TheoraStream::open(Box::new(Cursor::new(bytes.clone())), &index).unwrap();
        let expected: Vec<RgbaImage> = (0..index.frames.len() as u32)
            .map(|f| reference.frame(f).unwrap())
            .collect();

        // Exercise zero, sub-frame, 1-RGBA+1-YUV, 2-RGBA, and hybrid RGBA+YUV budgets.
        for budget in [
            0,
            yuv_bytes,
            rgba_bytes - 1,
            rgba_bytes,
            rgba_bytes + yuv_bytes,
            2 * rgba_bytes,
            2 * rgba_bytes + yuv_bytes,
            3 * rgba_bytes + yuv_bytes,
        ] {
            let factory = BufferedFactory {
                factory: PrefetchTheoraFactory::default(),
                bytes: budget,
                frames: 8,
                yuv: true,
            };
            let mut decoder = factory
                .open_prefetch(Box::new(Cursor::new(bytes.clone())), &index)
                .unwrap();

            let first = decoder.frame(0).unwrap();
            assert_eq!(first, expected[0], "budget={budget} frame=0");
            assert_eq!(
                decoder.frame(0).unwrap(),
                expected[0],
                "repeated budget={budget} frame=0"
            );

            // Allow the background worker to fill its lookahead up to `budget`.
            if budget >= rgba_bytes {
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline {
                    let ready = {
                        let state = decoder.cache.state.lock().unwrap();
                        state
                            .frames
                            .iter()
                            .any(|(p, f)| *p == 1 && matches!(f, CachedFrame::Rgba(_)))
                    };
                    if ready {
                        break;
                    }
                    thread::yield_now();
                }
                let state = decoder.cache.state.lock().unwrap();
                assert!(
                    state
                        .frames
                        .iter()
                        .any(|(p, f)| *p == 1 && matches!(f, CachedFrame::Rgba(_))),
                    "worker must pre-convert next frame to RGBA when budget={budget} allows"
                );
            }

            for (i, want) in expected.iter().enumerate().skip(1) {
                let got = decoder.frame(i as u32).unwrap();
                assert_eq!(&got, want, "budget={budget} frame={i}");
                let repeated = decoder.frame(i as u32).unwrap();
                assert_eq!(&repeated, want, "repeated budget={budget} frame={i}");
                let state = decoder.cache.state.lock().unwrap();
                assert!(
                    state.bytes <= budget,
                    "live bytes {} exceeded budget {}",
                    state.bytes,
                    budget
                );
                assert!(
                    state.peak_bytes <= budget,
                    "peak bytes {} exceeded budget {}",
                    state.peak_bytes,
                    budget
                );
            }
            decoder.finish().unwrap();
            let state = decoder.cache.state.lock().unwrap();
            assert!(state.peak_bytes <= budget);
        }
    }
}
