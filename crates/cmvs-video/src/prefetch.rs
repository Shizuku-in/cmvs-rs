//! Theora decoding on a worker thread that converts the next packet while the host
//! shows the current one. Results are identical to [`TheoraStream`]; only the timing moves.
use crate::TheoraStream;
use cmvs_core::{
    mgv::{self, MgvIndex, ReadSeek},
    raster::RgbaImage,
};
use std::{
    error::Error,
    fmt,
    sync::mpsc::{Receiver, Sender, channel},
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
impl mgv::DecoderFactory for PrefetchTheoraFactory {
    fn open(
        &self,
        reader: Box<dyn ReadSeek>,
        index: &MgvIndex,
    ) -> Result<Box<dyn mgv::Decoder>, Box<dyn Error>> {
        // Headers are checked here, so a bad movie still fails on open.
        let stream = TheoraStream::open(reader, index)?;
        let (requests, worker_requests) = channel();
        let (worker_replies, replies) = channel();
        let start = self.on_thread_start;
        thread::Builder::new()
            .name("mgv-decode".into())
            .spawn(move || {
                start();
                serve(stream, &worker_requests, &worker_replies);
            })?;
        Ok(Box::new(PrefetchDecoder { requests, replies }))
    }
}

enum Request {
    Frame(u32),
    Finish,
}
/// Errors cross the thread as text.
type Reply = Result<Option<RgbaImage>, String>;

fn serve(mut stream: TheoraStream, requests: &Receiver<Request>, replies: &Sender<Reply>) {
    use mgv::Decoder;
    // Ends when the decoder is dropped and the request channel closes.
    while let Ok(request) = requests.recv() {
        let reply = match request {
            Request::Frame(frame) => stream.frame(frame).map(Some),
            Request::Finish => stream.finish().map(|()| None),
        };
        let ok = reply.is_ok();
        if replies.send(reply.map_err(|e| e.to_string())).is_err() {
            return;
        }
        if ok {
            stream.prefetch();
        }
    }
}

/// The host side: each call waits for the worker, which usually has the frame ready.
/// Dropping it detaches the worker, which exits after any packet in progress.
struct PrefetchDecoder {
    requests: Sender<Request>,
    replies: Receiver<Reply>,
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
        Ok(self
            .call(Request::Frame(frame))?
            .expect("frame request returns an image"))
    }
    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        self.call(Request::Finish).map(|_| ())
    }
}
