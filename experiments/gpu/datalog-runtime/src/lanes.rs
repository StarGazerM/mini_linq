//! Formal rule DL-WaveLanes: the holes of one wave run on several streams.
//!
//! Lane 0 is the caller's thread on the program pool's stream. Every other lane
//! is a persistent worker thread on its own [`Pool::lane`] stream, created
//! once per program: module caches, readback counters and the context binding
//! are thread-local, so a worker keeps them for every wave. Per wave:
//!
//! 1. [`Lanes::fork`] records an event after all work enqueued on the main
//!    stream; each lane stream waits for it before its first hole. Inputs are
//!    immutable committed layouts (DL-WaveCommute), and every batch of an
//!    earlier wave was consumed on the main stream before the fork, so a lane
//!    reusing an arena slot or freed block is ordered after those reads.
//! 2. [`Lanes::submit`] sends a hole to its lane; a lane runs its holes in
//!    recorded order, each into the lane's arena of the hole's head relation.
//! 3. [`Lanes::join`] records an event on each lane stream after its holes and
//!    makes the main stream wait for all of them before returning the batches,
//!    so admission and publication (DL-PublishAfterReaders) follow every read.
//!
//! Batches stay bound to their lane pool. Blocks a lane frees after the join
//! are reused by RCCL only behind that lane's free event, and a lane reuses a
//! block or arena slot only after its next fork, which follows the main
//! stream's consumption of the batch.
use crate::{CandidateArena, CandidateBatch, Pool, Result};
use cuda_core::CudaEvent;
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{Arc, mpsc},
    thread,
};

/// A lane worker's stream and its candidate arenas, one per head relation.
/// Arenas live until [`Lanes::release`] at the component boundary.
pub struct Lane {
    pool: Arc<Pool>,
    arenas: BTreeMap<usize, CandidateArena>,
}

impl Lane {
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    /// This lane's arena of head relation `relation`, selecting the recorded
    /// Round hole's capacity hint (GPU-ReservePrevious). A hole always runs on
    /// the same lane, so its hint stays in this arena.
    pub fn arena(&mut self, relation: usize, hole: usize) -> &mut CandidateArena {
        let pool = &self.pool;
        self.arenas
            .entry(relation)
            .or_insert_with(|| CandidateArena::new(pool.clone()))
            .hole(hole)
    }
}

/// One hole for a lane worker; it returns the hole's candidate batch.
pub type Job = Box<dyn FnOnce(&mut Lane) -> CandidateBatch + Send>;

enum Message {
    Run(usize, Job),
    Join,
    Release,
}

enum Reply {
    Joined(Vec<(usize, thread::Result<CandidateBatch>)>),
    Released,
}

struct Worker {
    pool: Arc<Pool>,
    messages: Option<mpsc::Sender<Message>>,
    replies: mpsc::Receiver<Reply>,
    done: Arc<CudaEvent>,
    handle: Option<thread::JoinHandle<()>>,
}

/// The program's lanes: the caller's stream plus `streams - 1` workers.
pub struct Lanes {
    main: Arc<Pool>,
    fork: CudaEvent,
    workers: Vec<Worker>,
}

impl Lanes {
    pub fn new(main: &Arc<Pool>, streams: usize) -> Result<Self> {
        if streams == 0 {
            return Err("wave lanes need at least one stream".into());
        }
        let mut workers = Vec::with_capacity(streams - 1);
        for index in 1..streams {
            let pool = main.lane()?;
            let done = Arc::new(main.context().new_event(None)?);
            let (messages, inbox) = mpsc::channel::<Message>();
            let (outbox, replies) = mpsc::channel::<Reply>();
            let (lane_pool, lane_done) = (pool.clone(), done.clone());
            let handle = thread::Builder::new()
                .name(format!("gpu-datalog-lane-{index}"))
                .spawn(move || work(lane_pool, lane_done, inbox, outbox))?;
            workers.push(Worker {
                pool,
                messages: Some(messages),
                replies,
                done,
                handle: Some(handle),
            });
        }
        Ok(Self {
            main: main.clone(),
            fork: main.context().new_event(None)?,
            workers,
        })
    }

    /// Order every lane stream after all work enqueued on the main stream.
    pub fn fork(&self) -> Result<()> {
        self.fork.record(self.main.stream())?;
        for worker in &self.workers {
            worker.pool.stream().wait(&self.fork)?;
        }
        Ok(())
    }

    /// Run `job` on lane `lane` (1-based worker lanes) as wave hole `position`.
    pub fn submit(&self, lane: usize, position: usize, job: Job) -> Result<()> {
        let worker = lane
            .checked_sub(1)
            .and_then(|index| self.workers.get(index))
            .ok_or("wave lane outside the program's streams")?;
        worker
            .messages
            .as_ref()
            .expect("live lane worker")
            .send(Message::Run(position, job))
            .map_err(|_| "wave lane worker exited".into())
    }

    /// Wait for every lane's holes, order the main stream after them, and
    /// return the batches with their wave positions. A hole that panicked on
    /// a lane re-raises its panic here.
    pub fn join(&self) -> Result<Vec<(usize, CandidateBatch)>> {
        for worker in &self.workers {
            worker
                .messages
                .as_ref()
                .expect("live lane worker")
                .send(Message::Join)
                .map_err(|_| "wave lane worker exited")?;
        }
        let mut batches = Vec::new();
        let mut panic = None;
        for worker in &self.workers {
            let Ok(Reply::Joined(results)) = worker.replies.recv() else {
                return Err("wave lane worker exited before joining".into());
            };
            self.main.stream().wait(&worker.done)?;
            for (position, result) in results {
                match result {
                    Ok(batch) => batches.push((position, batch)),
                    Err(payload) => panic = panic.or(Some(payload)),
                }
            }
        }
        if let Some(payload) = panic {
            resume_unwind(payload);
        }
        Ok(batches)
    }

    /// Drop every lane's arenas at the component boundary (recorded
    /// component-scoped arenas); returns once the workers have freed them.
    pub fn release(&self) -> Result<()> {
        for worker in &self.workers {
            worker
                .messages
                .as_ref()
                .expect("live lane worker")
                .send(Message::Release)
                .map_err(|_| "wave lane worker exited")?;
        }
        for worker in &self.workers {
            let Ok(Reply::Released) = worker.replies.recv() else {
                return Err("wave lane worker exited before releasing".into());
            };
        }
        Ok(())
    }
}

impl Drop for Lanes {
    fn drop(&mut self) {
        for worker in &mut self.workers {
            worker.messages.take();
            if let Some(handle) = worker.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

fn work(
    pool: Arc<Pool>,
    done: Arc<CudaEvent>,
    inbox: mpsc::Receiver<Message>,
    outbox: mpsc::Sender<Reply>,
) {
    if pool.context().bind_to_thread().is_err() {
        return;
    }
    let mut lane = Lane {
        pool,
        arenas: BTreeMap::new(),
    };
    let mut results = Vec::new();
    while let Ok(message) = inbox.recv() {
        match message {
            Message::Run(position, job) => {
                results.push((position, catch_unwind(AssertUnwindSafe(|| job(&mut lane)))));
            }
            Message::Join => {
                // The main stream waits for this event before admission.
                if let Err(error) = done.record(lane.pool.stream()) {
                    let failure: Box<dyn std::any::Any + Send> =
                        Box::new(format!("wave lane join event failed: {error:?}"));
                    results.push((usize::MAX, Err(failure)));
                }
                if outbox.send(Reply::Joined(std::mem::take(&mut results))).is_err() {
                    return;
                }
            }
            Message::Release => {
                lane.arenas.clear();
                if outbox.send(Reply::Released).is_err() {
                    return;
                }
            }
        }
    }
}
