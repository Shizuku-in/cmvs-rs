//! Scene-movie frames decoded on one worker thread ahead of playback. A frame is used only
//! when playback asks for exactly the frame requested for that player; anything else is
//! decoded synchronously as before, so results never change, only the timing.
use super::SceneMovieIndex;
use crate::raster::RgbaImage;
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
    thread,
};

struct Job {
    id: u64,
    frame: u32,
    index: Arc<SceneMovieIndex>,
    data: Arc<Vec<u8>>,
    base: Option<RgbaImage>,
}
/// Errors cross the thread as text.
struct Reply {
    id: u64,
    frame: u32,
    result: Result<RgbaImage, String>,
}

pub(crate) struct ScenePrefetch {
    on_thread_start: fn(),
    /// Started by the first request; `None` again after the worker stopped.
    worker: Option<(Sender<Job>, Receiver<Reply>)>,
    /// Requested and not yet taken: (player id, frame), at most one per player.
    requested: Vec<(u64, u32)>,
    /// Replies to entries of `requested`.
    ready: Vec<Reply>,
    /// The worker could not start or stopped; decode synchronously from now on.
    stopped: bool,
}
impl fmt::Debug for ScenePrefetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScenePrefetch")
            .field("requested", &self.requested)
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

impl ScenePrefetch {
    pub(crate) fn new(on_thread_start: fn()) -> Self {
        Self {
            on_thread_start,
            worker: None,
            requested: Vec::new(),
            ready: Vec::new(),
            stopped: false,
        }
    }

    /// Drops requests and decoded frames of players that no longer exist.
    pub(crate) fn retain_players(&mut self, live: &[u64]) {
        self.requested.retain(|(id, _)| live.contains(id));
        self.ready.retain(|reply| live.contains(&reply.id));
    }

    /// Queues `frame` of player `id`, replacing an earlier request for that player.
    pub(crate) fn request(
        &mut self,
        id: u64,
        frame: u32,
        index: &Arc<SceneMovieIndex>,
        data: &Arc<Vec<u8>>,
        base: &Option<RgbaImage>,
    ) {
        if self.stopped || self.requested.contains(&(id, frame)) {
            return;
        }
        self.requested.retain(|&(other, _)| other != id);
        self.ready.retain(|reply| reply.id != id);
        if self.worker.is_none() {
            self.worker = self.spawn();
        }
        let Some((jobs, _)) = &self.worker else {
            self.stopped = true;
            return;
        };
        let job = Job {
            id,
            frame,
            index: Arc::clone(index),
            data: Arc::clone(data),
            base: base.clone(),
        };
        if jobs.send(job).is_err() {
            self.stop();
            return;
        }
        self.requested.push((id, frame));
    }

    /// The requested frame, waiting for the worker if it is still decoding; `None` when
    /// that frame was not requested, so the caller decodes it itself.
    pub(crate) fn take(
        &mut self,
        id: u64,
        frame: u32,
    ) -> Option<Result<RgbaImage, Box<dyn Error>>> {
        let at = self.requested.iter().position(|&r| r == (id, frame))?;
        loop {
            if let Some(i) = self
                .ready
                .iter()
                .position(|reply| (reply.id, reply.frame) == (id, frame))
            {
                self.requested.swap_remove(at);
                return Some(self.ready.swap_remove(i).result.map_err(Into::into));
            }
            let Some(reply) = self.worker.as_ref().and_then(|(_, r)| r.recv().ok()) else {
                self.stop();
                return None;
            };
            // Replies to superseded requests are dropped.
            if self.requested.contains(&(reply.id, reply.frame)) {
                self.ready.push(reply);
            }
        }
    }

    fn stop(&mut self) {
        self.worker = None;
        self.requested.clear();
        self.ready.clear();
        self.stopped = true;
    }

    fn spawn(&self) -> Option<(Sender<Job>, Receiver<Reply>)> {
        let (jobs, worker_jobs) = channel::<Job>();
        let (worker_replies, replies) = channel();
        let start = self.on_thread_start;
        thread::Builder::new()
            .name("scene-movie-decode".into())
            .spawn(move || {
                start();
                // Ends when the host drops its sender.
                while let Ok(job) = worker_jobs.recv() {
                    let result = job
                        .index
                        .decode(&job.data, job.frame, job.base.as_ref())
                        .map_err(|e| e.to_string());
                    let reply = Reply {
                        id: job.id,
                        frame: job.frame,
                        result,
                    };
                    if worker_replies.send(reply).is_err() {
                        return;
                    }
                }
            })
            .ok()?;
        Some((jobs, replies))
    }
}
