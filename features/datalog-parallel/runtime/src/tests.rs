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
                let other = ShardedSeen::<NoTraits>::default();
                assert_eq!(seen.shards.len(), threads.next_power_of_two());
                assert!(seen.shards.len().is_power_of_two());
                for key in 0..64 {
                    assert_eq!(
                        shard_for(&key, &seen.routing, seen.shards.len()),
                        shard_for(&key, &other.routing, other.shards.len()),
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
            let mut inline = ShardedSeen::new();
            let mut parallel = ShardedSeen::new();
            assert!(inline.insert((9, 9)));
            assert!(parallel.insert((9, 9)));
            assert!(!parallel.insert((9, 9)));
            let first = inline.admit(candidates.clone(), usize::MAX);
            assert_eq!(parallel.admit(candidates, 0), first);
            assert!(first.windows(2).all(|rows| {
                shard_for(&rows[0], &inline.routing, inline.shards.len())
                    <= shard_for(&rows[1], &inline.routing, inline.shards.len())
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
    let mut seen = ShardedSeen::new();
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
    let mut seen = ShardedSeen::new();
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
