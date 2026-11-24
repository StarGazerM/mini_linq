use super::*;
use std::hash::Hasher;

type Fx = std::hash::BuildHasherDefault<rustc_hash::FxHasher>;

#[test]
fn task_candidates_keep_first_occurrences_without_cloning() {
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct Owned(String);
    let hasher = Fx::default();
    let mut task = TaskCandidates::with_hasher(&hasher);
    for value in ["old", "new", "new", "next", "next", "old"] {
        task.insert(Owned(value.to_owned()));
    }
    assert!(std::ptr::eq(task.routing, &hasher));
    let rows: Vec<_> = task.into_rows().into_iter().map(|row| row.0).collect();
    assert_eq!(rows, ["old", "new", "next"]);
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
    let hasher = Fx::default();
    assert!(
        TaskCandidates::<Row, _>::with_hasher(&hasher)
            .into_rows()
            .is_empty()
    );
    let mut task = TaskCandidates::with_hasher(&hasher);
    for value in [3, 1, 3, 2, 1, 2] {
        task.insert(Row(value));
    }
    let rows = task.into_rows();
    assert_eq!(rows, [Row(3), Row(1), Row(2)]);
}
