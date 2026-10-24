//! Event-driven step executor for relational progress (P-DL-Relational).
//!
//! The scheduler is pure bookkeeping over component-local member and step-hole
//! indices; it owns no relation data. Generated code supplies one closure that
//! performs an [`Action`] (evaluate a hole under its read locks, or admit and
//! publish one member under its write lock) and reports a [`Completion`].
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc;

/// One unit of work chosen by the scheduler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Evaluate step hole `hole` on batch `batch` of its delta member.
    Hole { hole: usize, batch: u64 },
    /// Admit the pending candidates of `member` and publish any fresh rows.
    Commit { member: usize },
}

/// What a finished action reports back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    /// Members that received at least one candidate from the hole.
    Hole { produced: Vec<usize> },
    /// Whether the commit published a nonempty batch.
    Commit { fresh: bool },
}

/// Bookkeeping for one component's step phase.
#[derive(Clone, Debug)]
pub struct RelationalScheduler {
    delta_of: Vec<usize>,
    readers: Vec<Vec<usize>>,
    version: Vec<u64>,
    consumed: Vec<u64>,
    running: Vec<bool>,
    pending: Vec<bool>,
    committing: Vec<bool>,
}

impl RelationalScheduler {
    /// `delta_of[h]` is the member whose batches step hole `h` consumes.
    /// `initial[m]` states whether the seed publication left `m` a nonempty
    /// current batch; such a member starts at batch 1, otherwise at 0.
    pub fn new(delta_of: Vec<usize>, initial: &[bool]) -> Self {
        let members = initial.len();
        let mut readers = vec![Vec::new(); members];
        for (hole, &member) in delta_of.iter().enumerate() {
            assert!(member < members, "step hole {hole} reads an unknown member");
            readers[member].push(hole);
        }
        Self {
            consumed: vec![0; delta_of.len()],
            running: vec![false; delta_of.len()],
            delta_of,
            readers,
            version: initial
                .iter()
                .map(|&nonempty| u64::from(nonempty))
                .collect(),
            pending: vec![false; members],
            committing: vec![false; members],
        }
    }

    /// Formal rule DL-RelationalScheduler: spawn each hole once per unconsumed committed batch and never concurrently with itself; commit a member only when it has pending candidates, is idle, and every delta reader consumed its current batch.
    ///
    /// Returned actions are marked in flight. Commits are listed before holes.
    pub fn actions(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        for member in 0..self.version.len() {
            if self.pending[member]
                && !self.committing[member]
                && self.readers[member]
                    .iter()
                    .all(|&hole| self.consumed[hole] == self.version[member])
            {
                self.pending[member] = false;
                self.committing[member] = true;
                out.push(Action::Commit { member });
            }
        }
        for hole in 0..self.delta_of.len() {
            let batch = self.version[self.delta_of[hole]];
            if !self.running[hole] && self.consumed[hole] < batch {
                self.running[hole] = true;
                out.push(Action::Hole { hole, batch });
            }
        }
        out
    }

    /// Record a finished action.
    pub fn complete(&mut self, action: Action, completion: &Completion) {
        match (action, completion) {
            (Action::Hole { hole, batch }, Completion::Hole { produced }) => {
                assert!(self.running[hole], "hole {hole} completed without running");
                self.running[hole] = false;
                self.consumed[hole] = batch;
                for &member in produced {
                    self.pending[member] = true;
                }
            }
            (Action::Commit { member }, Completion::Commit { fresh }) => {
                assert!(
                    self.committing[member],
                    "member {member} committed while idle"
                );
                self.committing[member] = false;
                if *fresh {
                    self.version[member] += 1;
                }
            }
            _ => panic!("completion kind differs from its action"),
        }
    }

    /// Nothing runs, nothing is pending, and every batch has been consumed.
    pub fn quiescent(&self) -> bool {
        !self.running.iter().any(|&r| r)
            && !self.committing.iter().any(|&c| c)
            && !self.pending.iter().any(|&p| p)
            && self
                .delta_of
                .iter()
                .zip(&self.consumed)
                .all(|(&member, &consumed)| consumed == self.version[member])
    }

    /// Committed nonempty batches of `member` so far, including the seed batch.
    pub fn version(&self, member: usize) -> u64 {
        self.version[member]
    }
}

/// Run the step phase to quiescence inside the current Rayon pool.
///
/// The calling thread only dispatches: it spawns every action the scheduler
/// releases into one joining scope and blocks for completions. A panicking
/// action stops further dispatch; the panic resumes after in-flight work ends.
pub fn drive(scheduler: &mut RelationalScheduler, run: &(dyn Fn(Action) -> Completion + Sync)) {
    let (sender, receiver) = mpsc::channel();
    let mut failure = None;
    rayon::in_place_scope(|scope| {
        let mut in_flight = 0usize;
        loop {
            if failure.is_none() {
                for action in scheduler.actions() {
                    in_flight += 1;
                    let sender = sender.clone();
                    scope.spawn(move |_| {
                        let result = catch_unwind(AssertUnwindSafe(|| run(action)));
                        let _ = sender.send((action, result));
                    });
                }
            }
            if in_flight == 0 {
                break;
            }
            let (action, result) = receiver.recv().expect("dispatcher holds a sender");
            in_flight -= 1;
            match result {
                Ok(completion) if failure.is_none() => scheduler.complete(action, &completion),
                Ok(_) => {}
                Err(payload) => {
                    failure.get_or_insert(payload);
                }
            }
        }
    });
    if let Some(payload) = failure {
        resume_unwind(payload);
    }
    debug_assert!(scheduler.quiescent());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Mutex;

    #[test]
    fn holes_wait_for_batches_and_commits_wait_for_readers() {
        // members 0,1; hole 0 reads delta of 0, holes 1 and 2 read delta of 1.
        let mut s = RelationalScheduler::new(vec![0, 1, 1], &[true, false]);
        assert_eq!(s.actions(), vec![Action::Hole { hole: 0, batch: 1 }]);
        assert_eq!(s.actions(), vec![], "a running hole is not respawned");
        s.complete(
            Action::Hole { hole: 0, batch: 1 },
            &Completion::Hole { produced: vec![1] },
        );
        assert_eq!(s.actions(), vec![Action::Commit { member: 1 }]);
        s.complete(
            Action::Commit { member: 1 },
            &Completion::Commit { fresh: true },
        );
        let next = s.actions();
        assert_eq!(
            next,
            vec![
                Action::Hole { hole: 1, batch: 1 },
                Action::Hole { hole: 2, batch: 1 }
            ]
        );
        // A new candidate for 1 cannot commit until both readers consumed batch 1.
        s.complete(next[0], &Completion::Hole { produced: vec![1] });
        assert_eq!(s.actions(), vec![]);
        s.complete(next[1], &Completion::Hole { produced: vec![] });
        assert_eq!(s.actions(), vec![Action::Commit { member: 1 }]);
        s.complete(
            Action::Commit { member: 1 },
            &Completion::Commit { fresh: false },
        );
        assert!(s.quiescent());
        assert_eq!(s.version(1), 1);
    }

    #[test]
    fn candidates_during_a_commit_are_kept_for_the_next_commit() {
        let mut s = RelationalScheduler::new(vec![0, 0], &[true]);
        let first = s.actions();
        s.complete(first[0], &Completion::Hole { produced: vec![0] });
        assert_eq!(s.actions(), vec![], "reader 1 has not consumed batch 1");
        s.complete(first[1], &Completion::Hole { produced: vec![0] });
        assert_eq!(s.actions(), vec![Action::Commit { member: 0 }]);
        // Fresh rows: batch 2 releases both readers again.
        s.complete(
            Action::Commit { member: 0 },
            &Completion::Commit { fresh: true },
        );
        assert_eq!(s.actions().len(), 2);
        assert!(!s.quiescent());
    }

    /// Transitive closure of a chain driven through `drive` on a real pool:
    /// checks the executor reaches the same fixpoint and that every hole saw
    /// every batch of its delta member exactly once.
    #[test]
    fn drive_reaches_the_fixpoint_with_tiled_windows() {
        let edges: Vec<(u32, u32)> = (0..40).map(|i| (i, i + 1)).collect();
        for threads in [2, 4, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            pool.install(|| {
                // One member T, one step hole: T(x,z) :- T(x,y), E(y,z) with delta on T.
                let full = Mutex::new(edges.iter().copied().collect::<BTreeSet<_>>());
                let batches: Mutex<Vec<Vec<(u32, u32)>>> = Mutex::new(vec![edges.clone()]);
                let pending: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());
                let seen_batches: Mutex<HashMap<u64, usize>> = Mutex::new(HashMap::new());
                let mut scheduler = RelationalScheduler::new(vec![0], &[true]);
                drive(&mut scheduler, &|action| match action {
                    Action::Hole { batch, .. } => {
                        *seen_batches.lock().unwrap().entry(batch).or_default() += 1;
                        let delta = batches.lock().unwrap()[batch as usize - 1].clone();
                        let out: Vec<_> = delta
                            .iter()
                            .flat_map(|&(x, y)| {
                                edges
                                    .iter()
                                    .filter(move |e| e.0 == y)
                                    .map(move |&(_, z)| (x, z))
                            })
                            .collect();
                        let produced = if out.is_empty() { vec![] } else { vec![0] };
                        pending.lock().unwrap().extend(out);
                        Completion::Hole { produced }
                    }
                    Action::Commit { .. } => {
                        let candidates = std::mem::take(&mut *pending.lock().unwrap());
                        let mut full = full.lock().unwrap();
                        let fresh: Vec<_> = candidates
                            .into_iter()
                            .filter(|t| full.insert(*t))
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect();
                        let nonempty = !fresh.is_empty();
                        if nonempty {
                            batches.lock().unwrap().push(fresh);
                        }
                        Completion::Commit { fresh: nonempty }
                    }
                });
                let full = full.into_inner().unwrap();
                assert_eq!(full.len(), 40 * 41 / 2, "closure of a 41-node chain");
                let seen = seen_batches.into_inner().unwrap();
                assert_eq!(seen.len() as u64, scheduler.version(0));
                assert!(seen.values().all(|&count| count == 1), "{seen:?}");
            });
        }
    }

    #[test]
    #[should_panic(expected = "boom")]
    fn a_panicking_action_resumes_after_in_flight_work() {
        let mut s = RelationalScheduler::new(vec![0, 0], &[true]);
        drive(&mut s, &|action| match action {
            Action::Hole { hole: 0, .. } => panic!("boom"),
            Action::Hole { .. } => Completion::Hole { produced: vec![] },
            Action::Commit { .. } => Completion::Commit { fresh: false },
        });
    }
}
