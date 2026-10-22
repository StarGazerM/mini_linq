use super::*;
use rayon::ThreadPoolBuilder;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn defaults_have_no_row_bounds_and_round_up_pool_size() {
    struct NoTraits;
    for threads in [1_usize, 3, 5] {
        ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let seen = ShardedSeen::<NoTraits>::default();
                let other = ShardedSeen::<NoTraits>::with_hasher(seen.routing.clone());
                assert_eq!(seen.shards.len(), threads.next_power_of_two());
                assert!(seen.shards.len().is_power_of_two());
                for key in 0..64 {
                    assert_eq!(
                        seen.routing.hash_one(key) as usize & (seen.shards.len() - 1),
                        other.routing.hash_one(key) as usize & (other.shards.len() - 1),
                    );
                }
            });
    }
}

#[test]
fn admission_is_full_tuple_exact_and_threshold_independent() {
    ThreadPoolBuilder::new()
        .num_threads(3)
        .build()
        .unwrap()
        .install(|| {
            let candidates = vec![
                vec![(1, 2), (1, 3), (2, 1), (1, 2)],
                vec![],
                vec![(2, 1), (3, 1), (1, 3), (9, 9)],
            ];
            let mut inline = ShardedSeen::<_>::new();
            let mut parallel = ShardedSeen::with_hasher(inline.routing.clone());
            assert!(inline.insert((9, 9)));
            assert!(parallel.insert((9, 9)));
            assert!(!parallel.insert((9, 9)));
            let first = inline.admit(candidates.clone(), usize::MAX);
            assert_eq!(parallel.admit(candidates, 0), first);
            assert!(first.windows(2).all(|rows| {
                (inline.routing.hash_one(rows[0]) as usize & (inline.shards.len() - 1))
                    <= (inline.routing.hash_one(rows[1]) as usize & (inline.shards.len() - 1))
            }));
            let mut sorted = first.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, [(1, 2), (1, 3), (2, 1), (3, 1)]);
            assert!(parallel.admit(vec![first], 0).is_empty());
            assert_eq!(parallel.admit(vec![vec![(1, 2), (4, 5)]], 2), [(4, 5)]);
            assert!(parallel.admit(vec![], 0).is_empty());
            assert!(parallel.admit(vec![vec![], vec![]], 0).is_empty());
        });
}

#[derive(Debug, PartialEq, Eq)]
struct Colliding(u32);

impl Hash for Colliding {
    fn hash<H: Hasher>(&self, state: &mut H) {
        0_u8.hash(state);
    }
}

impl Clone for Colliding {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

#[test]
fn hash_collisions_keep_distinct_tuples_and_candidate_order() {
    let mut seen = ShardedSeen::<_>::new();
    assert_eq!(
        seen.admit(
            vec![
                vec![Colliding(3), Colliding(1)],
                vec![Colliding(3), Colliding(2)]
            ],
            0,
        ),
        [Colliding(3), Colliding(1), Colliding(2)]
    );
}

struct Tracked {
    value: String,
    clones: Arc<AtomicUsize>,
}

impl PartialEq for Tracked {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for Tracked {}

impl Hash for Tracked {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl Clone for Tracked {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::Relaxed);
        Self {
            value: self.value.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

#[test]
fn admission_clones_only_successful_owned_tuples() {
    let clones = Arc::new(AtomicUsize::new(0));
    let row = |value: &str| Tracked {
        value: value.to_owned(),
        clones: Arc::clone(&clones),
    };
    let mut seen = ShardedSeen::<_>::new();
    assert!(seen.insert(row("old")));
    let admitted = seen.admit(
        vec![vec![row("old"), row("new")], vec![row("new"), row("next")]],
        0,
    );
    assert_eq!(admitted.len(), 2);
    assert_eq!(clones.load(Ordering::Relaxed), 2);
    assert!(seen.admit(vec![admitted], 0).is_empty());
    assert_eq!(clones.load(Ordering::Relaxed), 2);
}

#[test]
fn fx_routes_and_admits_exactly_with_one_hash_per_candidate() {
    use std::hash::BuildHasherDefault;
    type Fx = BuildHasherDefault<rustc_hash::FxHasher>;
    struct Counted {
        value: u64,
        hashes: Arc<AtomicUsize>,
    }
    impl PartialEq for Counted {
        fn eq(&self, other: &Self) -> bool {
            self.value == other.value
        }
    }
    impl Eq for Counted {}
    impl Hash for Counted {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.hashes.fetch_add(1, Ordering::Relaxed);
            self.value.hash(state);
        }
    }
    impl Clone for Counted {
        fn clone(&self) -> Self {
            Self {
                value: self.value,
                hashes: Arc::clone(&self.hashes),
            }
        }
    }
    for threshold in [0, usize::MAX] {
        ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap()
            .install(|| {
                let hashes = Arc::new(AtomicUsize::new(0));
                let row = |value| Counted {
                    value,
                    hashes: Arc::clone(&hashes),
                };
                let mut seen = ShardedSeen::<Counted, Fx>::new();
                // Reserve outside the measured admission: resizing legitimately
                // rehashes existing rows, but routing and insert must share a hash.
                for shard in &mut seen.shards {
                    shard.reserve(64, |old| seen.routing.hash_one(old));
                }
                let candidates = (0..64).map(row).collect();
                let mut admitted = seen.admit(vec![candidates], threshold);
                assert_eq!(hashes.load(Ordering::Relaxed), 64);
                admitted.sort_by_key(|row| row.value);
                assert_eq!(
                    admitted.iter().map(|row| row.value).collect::<Vec<_>>(),
                    (0..64).collect::<Vec<_>>()
                );
                assert!(seen.admit(vec![admitted], threshold).is_empty());
                assert_eq!(hashes.load(Ordering::Relaxed), 128);
            });
    }
}

#[test]
fn task_candidates_move_owned_rows_without_cloning_before_global_admission() {
    let clones = Arc::new(AtomicUsize::new(0));
    let row = |value: &str| Tracked {
        value: value.to_owned(),
        clones: Arc::clone(&clones),
    };
    let mut seen = ShardedSeen::<_>::new();
    seen.insert(row("old"));
    let mut first = TaskCandidates::with_hasher(seen.hasher());
    for value in ["old", "new", "new", "next", "next"] {
        first.insert(row(value));
    }
    let mut second = TaskCandidates::with_hasher(seen.hasher());
    second.insert(row("new"));
    second.insert(row("next"));
    assert!(std::ptr::eq(first.routing, &seen.routing));
    let first = first.into_rows();
    let second = second.into_rows();
    assert_eq!(first.len(), 3);
    assert_eq!(second.len(), 2);
    assert_eq!(clones.load(Ordering::Relaxed), 0);
    let admitted = seen.admit(vec![first, second], 0);
    assert_eq!(admitted.len(), 2);
    assert_eq!(clones.load(Ordering::Relaxed), 2);
    assert!(seen.admit(vec![admitted], 0).is_empty());
    assert_eq!(clones.load(Ordering::Relaxed), 2);
}

#[test]
fn task_candidates_support_nonclone_colliding_rows_and_empty_tasks() {
    #[derive(Debug, PartialEq, Eq)]
    struct Row(u32);
    impl Hash for Row {
        fn hash<H: Hasher>(&self, state: &mut H) {
            0_u8.hash(state);
        }
    }
    let seen = ShardedSeen::<Row, std::hash::BuildHasherDefault<rustc_hash::FxHasher>>::new();
    assert!(
        TaskCandidates::<Row, _>::with_hasher(seen.hasher())
            .into_rows()
            .is_empty()
    );
    let mut task = TaskCandidates::with_hasher(seen.hasher());
    for value in [3, 1, 3, 2, 1, 2] {
        task.insert(Row(value));
    }
    let rows = task.into_rows();
    assert_eq!(rows, [Row(3), Row(1), Row(2)]);
}
