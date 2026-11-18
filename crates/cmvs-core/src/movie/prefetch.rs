//! Scene-movie frames decoded on one worker thread ahead of playback. A frame is used only
//! when playback asks for exactly the frame requested for that player; anything else is
//! decoded synchronously as before, so results never change, only the timing.
//!
//! Pending jobs are stored in a bounded latest-job-per-slot table (`Mutex<Shared> + Condvar`)
//! so fast-forwarding or frame skipping overwrites stale queued requests for a slot in $O(1)$
//! instead of queueing obsolete decodes behind an unbounded FIFO channel.
use super::SceneMovieIndex;
use crate::raster::RgbaImage;
use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    fmt,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
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
    frame: u32,
    result: Result<RgbaImage, String>,
}

type DecodeFn = Arc<dyn Fn(&Job) -> Result<RgbaImage, String> + Send + Sync>;

fn default_decode(job: &Job) -> Result<RgbaImage, String> {
    job.index
        .decode(&job.data, job.frame, job.base.as_ref())
        .map_err(|e| e.to_string())
}

#[derive(Default)]
struct Shared {
    /// Latest queued (not yet started) job per player id.
    pending: HashMap<u64, Job>,
    /// Fair round-robin order of player ids that may have an entry in `pending`.
    queue: VecDeque<u64>,
    /// Currently decoding `(id, frame)` on the worker thread, if any.
    in_flight: Option<(u64, u32)>,
    /// Latest finished decode per player id matching its `wanted` frame.
    ready: HashMap<u64, Reply>,
    /// Currently requested frame per player id.
    wanted: HashMap<u64, u32>,
    /// Set when shutting down the worker thread.
    shutdown: bool,
    /// Set once the worker thread exits so waiters never block indefinitely.
    worker_exited: bool,
}

#[derive(Default)]
struct SyncState {
    shared: Mutex<Shared>,
    work_cv: Condvar,
    reply_cv: Condvar,
}

struct WorkerExitGuard<'a>(&'a SyncState);

impl Drop for WorkerExitGuard<'_> {
    fn drop(&mut self) {
        let mut shared = self.0.shared.lock().unwrap_or_else(|e| e.into_inner());
        shared.in_flight = None;
        shared.worker_exited = true;
        self.0.reply_cv.notify_all();
    }
}

pub(crate) struct ScenePrefetch {
    on_thread_start: fn(),
    decode: DecodeFn,
    /// Started by the first request; `None` before start or after shutdown.
    worker: Option<(Arc<SyncState>, JoinHandle<()>)>,
    /// The worker could not start or stopped; decode synchronously from now on.
    stopped: bool,
}

impl fmt::Debug for ScenePrefetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut requested: Vec<(u64, u32)> = self
            .worker
            .as_ref()
            .map(|(sync, _)| {
                let shared = sync.shared.lock().unwrap_or_else(|e| e.into_inner());
                shared
                    .wanted
                    .iter()
                    .map(|(&id, &frame)| (id, frame))
                    .collect()
            })
            .unwrap_or_default();
        requested.sort_unstable_by_key(|&(id, _)| id);
        f.debug_struct("ScenePrefetch")
            .field("requested", &requested)
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

impl Drop for ScenePrefetch {
    fn drop(&mut self) {
        self.shutdown_worker();
    }
}

impl ScenePrefetch {
    pub(crate) fn new(on_thread_start: fn()) -> Self {
        Self::with_decoder(on_thread_start, Arc::new(default_decode))
    }

    fn with_decoder(on_thread_start: fn(), decode: DecodeFn) -> Self {
        Self {
            on_thread_start,
            decode,
            worker: None,
            stopped: false,
        }
    }

    fn cancel_locked(shared: &mut Shared, id: u64) {
        shared.wanted.remove(&id);
        shared.pending.remove(&id);
        shared.ready.remove(&id);
        shared.queue.retain(|queued| *queued != id);
    }

    /// Drops any pending or ready prefetch work for player `id`.
    pub(crate) fn cancel(&mut self, id: u64) {
        let Some((sync, _)) = &self.worker else {
            return;
        };
        let mut shared = sync.shared.lock().unwrap_or_else(|e| e.into_inner());
        Self::cancel_locked(&mut shared, id);
    }

    /// Drops requests and decoded frames of players that no longer exist.
    pub(crate) fn retain_players(&mut self, live: &[u64]) {
        let Some((sync, _)) = &self.worker else {
            return;
        };
        let mut shared = sync.shared.lock().unwrap_or_else(|e| e.into_inner());
        shared.wanted.retain(|id, _| live.contains(id));
        shared.pending.retain(|id, _| live.contains(id));
        shared.ready.retain(|id, _| live.contains(id));
        shared.queue.retain(|id| live.contains(id));
    }

    /// Queues `frame` of player `id`, overwriting any stale pending request for `id` in $O(1)$.
    pub(crate) fn request(
        &mut self,
        id: u64,
        frame: u32,
        index: &Arc<SceneMovieIndex>,
        data: &Arc<Vec<u8>>,
        base: &Option<RgbaImage>,
    ) {
        if self.stopped {
            return;
        }
        if self.worker.is_none() {
            self.worker = self.spawn();
        }
        let Some((sync, _)) = &self.worker else {
            self.stopped = true;
            return;
        };
        let mut need_stop = false;
        {
            let mut shared = sync.shared.lock().unwrap_or_else(|e| e.into_inner());
            if shared.worker_exited {
                need_stop = true;
            } else if shared.wanted.get(&id) == Some(&frame) {
                return;
            } else {
                shared.wanted.insert(id, frame);
                shared.ready.remove(&id);
                if shared.in_flight == Some((id, frame)) {
                    // The worker is already decoding this exact frame; drop any superseded
                    // queued frame for `id` and let the in-flight decode publish its reply.
                    shared.pending.remove(&id);
                } else {
                    let job = Job {
                        id,
                        frame,
                        index: Arc::clone(index),
                        data: Arc::clone(data),
                        base: base.clone(),
                    };
                    let replaced = shared.pending.insert(id, job).is_some();
                    if !replaced && !shared.queue.contains(&id) {
                        shared.queue.push_back(id);
                    }
                    sync.work_cv.notify_one();
                }
            }
        }
        if need_stop {
            self.stop();
        }
    }

    /// The requested frame when already decoded or actively in flight on the worker; `None`
    /// when not requested or still queued behind another player so the caller decodes immediately.
    pub(crate) fn take(
        &mut self,
        id: u64,
        frame: u32,
    ) -> Option<Result<RgbaImage, Box<dyn Error + Send + Sync>>> {
        let (sync, _) = self.worker.as_ref()?;
        let sync = Arc::clone(sync);
        let mut worker_died = false;
        let outcome = {
            let mut shared = sync.shared.lock().unwrap_or_else(|e| e.into_inner());
            if shared.wanted.get(&id) != Some(&frame) {
                return None;
            }
            loop {
                if shared.ready.get(&id).is_some_and(|r| r.frame == frame) {
                    let reply = shared.ready.remove(&id).expect("checked above");
                    shared.wanted.remove(&id);
                    break Some(reply.result.map_err(Into::into));
                }
                if shared.worker_exited {
                    worker_died = true;
                    break None;
                }
                if shared.in_flight != Some((id, frame)) {
                    Self::cancel_locked(&mut shared, id);
                    break None;
                }
                shared = sync
                    .reply_cv
                    .wait(shared)
                    .unwrap_or_else(|e| e.into_inner());
            }
        };
        if worker_died {
            self.stop();
        }
        outcome
    }

    fn stop(&mut self) {
        self.shutdown_worker();
        self.stopped = true;
    }

    fn shutdown_worker(&mut self) {
        let Some((sync, handle)) = self.worker.take() else {
            return;
        };
        {
            let mut shared = sync.shared.lock().unwrap_or_else(|e| e.into_inner());
            shared.shutdown = true;
            shared.pending.clear();
            shared.queue.clear();
            shared.ready.clear();
            shared.wanted.clear();
            sync.work_cv.notify_all();
            sync.reply_cv.notify_all();
        }
        let _ = handle.join();
    }

    fn spawn(&self) -> Option<(Arc<SyncState>, JoinHandle<()>)> {
        let sync = Arc::new(SyncState::default());
        let worker_sync = Arc::clone(&sync);
        let start = self.on_thread_start;
        let decode = Arc::clone(&self.decode);
        let handle = thread::Builder::new()
            .name("scene-movie-decode".into())
            .spawn(move || {
                let _exit_guard = WorkerExitGuard(&worker_sync);
                start();
                loop {
                    let job = {
                        let mut shared =
                            worker_sync.shared.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if shared.shutdown {
                                return;
                            }
                            let mut next_job = None;
                            while let Some(id) = shared.queue.pop_front() {
                                if let Some(job) = shared.pending.remove(&id) {
                                    next_job = Some(job);
                                    break;
                                }
                            }
                            if let Some(job) = next_job {
                                shared.in_flight = Some((job.id, job.frame));
                                break job;
                            }
                            shared = worker_sync
                                .work_cv
                                .wait(shared)
                                .unwrap_or_else(|e| e.into_inner());
                        }
                    };

                    let result = decode(&job);

                    let mut shared = worker_sync.shared.lock().unwrap_or_else(|e| e.into_inner());
                    shared.in_flight = None;
                    if shared.shutdown {
                        return;
                    }
                    // Publish only when `(job.id, job.frame)` has not been superseded.
                    if shared.wanted.get(&job.id) == Some(&job.frame) {
                        shared.ready.insert(
                            job.id,
                            Reply {
                                frame: job.frame,
                                result,
                            },
                        );
                    }
                    worker_sync.reply_cv.notify_all();
                }
            })
            .ok()?;
        Some((sync, handle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmv::{CmvHeader, CmvIndex};
    use std::sync::Barrier;

    fn dummy_index_and_data() -> (Arc<SceneMovieIndex>, Arc<Vec<u8>>) {
        let index = CmvIndex {
            header: CmvHeader {
                resource_offset: 44,
                file_size: 44,
                unused: 0,
                max_sequence: 16,
                rate: 30,
                rate_time: 1,
                width: 2,
                height: 2,
                bit_count: 24,
                audio_flag: 0,
            },
            entries: Vec::new(),
            alpha: false,
            jbp1: false,
        };
        (Arc::new(SceneMovieIndex::Cmv6(index)), Arc::new(Vec::new()))
    }

    fn frame_pixel(id: u64, frame: u32) -> RgbaImage {
        RgbaImage::new(1, 1, vec![id as u8, frame as u8, 0xaa, 0xff]).unwrap()
    }

    fn wait_worker_idle(prefetch: &ScenePrefetch) {
        let Some((sync, _)) = &prefetch.worker else {
            return;
        };
        let mut shared = sync.shared.lock().unwrap();
        while !shared.worker_exited && (!shared.pending.is_empty() || shared.in_flight.is_some()) {
            shared = sync.reply_cv.wait(shared).unwrap();
        }
    }

    #[test]
    fn fast_forward_overwrites_stale_pending_frames_per_slot_in_place() {
        let entered_first = Arc::new(Barrier::new(2));
        let release_first = Arc::new(Barrier::new(2));
        let decoded = Arc::new(Mutex::new(Vec::<(u64, u32)>::new()));

        let entered_w = Arc::clone(&entered_first);
        let release_w = Arc::clone(&release_first);
        let decoded_w = Arc::clone(&decoded);
        let mut prefetch = ScenePrefetch::with_decoder(
            || {},
            Arc::new(move |job| {
                if (job.id, job.frame) == (1, 0) {
                    entered_w.wait();
                    release_w.wait();
                }
                decoded_w.lock().unwrap().push((job.id, job.frame));
                Ok(frame_pixel(job.id, job.frame))
            }),
        );

        let (index, data) = dummy_index_and_data();

        // Start decoding frame 0 on the worker and hold it mid-decode.
        prefetch.request(1, 0, &index, &data, &None);
        entered_first.wait();

        // Fast-forward slot 1 across frames 1..=5 before the worker finishes frame 0,
        // while also updating slot 2 across frames 10..=12.
        for frame in 1..=5 {
            prefetch.request(1, frame, &index, &data, &None);
        }
        for frame in 10..=12 {
            prefetch.request(2, frame, &index, &data, &None);
        }

        // Release frame 0 and wait for the worker to drain coalesced requests; obsolete
        // queued frames 1..=4 and 10..=11 must never decode.
        release_first.wait();
        wait_worker_idle(&prefetch);

        // Superseded frames return None so callers never consume stale replies.
        assert!(prefetch.take(1, 0).is_none());
        assert!(prefetch.take(1, 1).is_none());
        assert!(prefetch.take(2, 10).is_none());

        let got_1 = prefetch.take(1, 5).expect("latest slot 1 frame").unwrap();
        let got_2 = prefetch.take(2, 12).expect("latest slot 2 frame").unwrap();
        assert_eq!(got_1, frame_pixel(1, 5));
        assert_eq!(got_2, frame_pixel(2, 12));

        let log = decoded.lock().unwrap().clone();
        assert_eq!(log, vec![(1, 0), (1, 5), (2, 12)]);
    }

    #[test]
    fn retain_players_and_drop_cancel_pending_work_and_join_cleanly() {
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let decoded = Arc::new(Mutex::new(Vec::<(u64, u32)>::new()));

        let entered_w = Arc::clone(&entered);
        let release_w = Arc::clone(&release);
        let decoded_w = Arc::clone(&decoded);
        let mut prefetch = ScenePrefetch::with_decoder(
            || {},
            Arc::new(move |job| {
                if (job.id, job.frame) == (1, 0) {
                    entered_w.wait();
                    release_w.wait();
                }
                decoded_w.lock().unwrap().push((job.id, job.frame));
                Ok(frame_pixel(job.id, job.frame))
            }),
        );

        let (index, data) = dummy_index_and_data();
        prefetch.request(1, 0, &index, &data, &None);
        entered.wait();

        // Queue work for player 2 (later destroyed) and player 3 (kept).
        prefetch.request(2, 7, &index, &data, &None);
        prefetch.request(3, 9, &index, &data, &None);
        prefetch.retain_players(&[1, 3]);

        release.wait();
        wait_worker_idle(&prefetch);

        assert!(prefetch.take(2, 7).is_none());
        assert_eq!(
            prefetch.take(3, 9).expect("kept slot").unwrap(),
            frame_pixel(3, 9)
        );
        assert_eq!(*decoded.lock().unwrap(), vec![(1, 0), (3, 9)]);

        // Dropping while idle or after queueing work joins the worker thread cleanly.
        prefetch.request(3, 10, &index, &data, &None);
        drop(prefetch);
    }

    #[test]
    fn take_does_not_block_behind_unrelated_in_flight_decode_and_keeps_worker_alive() {
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let decoded = Arc::new(Mutex::new(Vec::<(u64, u32)>::new()));

        let entered_w = Arc::clone(&entered);
        let release_w = Arc::clone(&release);
        let decoded_w = Arc::clone(&decoded);
        let mut prefetch = ScenePrefetch::with_decoder(
            || {},
            Arc::new(move |job| {
                if (job.id, job.frame) == (1, 0) {
                    entered_w.wait();
                    release_w.wait();
                }
                decoded_w.lock().unwrap().push((job.id, job.frame));
                Ok(frame_pixel(job.id, job.frame))
            }),
        );

        let (index, data) = dummy_index_and_data();
        prefetch.request(1, 0, &index, &data, &None);
        entered.wait();

        // Slot 2's request is queued behind slow in-flight slot 1.
        prefetch.request(2, 12, &index, &data, &None);

        // Taking slot 2 before releasing slot 1 must return immediately and cancel (2, 12)
        // without shutting down the worker thread.
        assert!(prefetch.take(2, 12).is_none());
        assert!(!prefetch.stopped);
        {
            let shared = prefetch.worker.as_ref().unwrap().0.shared.lock().unwrap();
            assert!(!shared.pending.contains_key(&2));
            assert!(!shared.wanted.contains_key(&2));
        }

        release.wait();
        wait_worker_idle(&prefetch);
        assert_eq!(*decoded.lock().unwrap(), vec![(1, 0)]);

        // Worker remains active for subsequent frames.
        prefetch.request(2, 13, &index, &data, &None);
        wait_worker_idle(&prefetch);
        assert_eq!(
            prefetch.take(2, 13).expect("worker stays alive").unwrap(),
            frame_pixel(2, 13)
        );
    }
}
