#[test]
fn chunked_scan_range_preserves_answers_with_total_rust_bindings() {
    let rows: Vec<_> = (0_i32..257).map(|x| (x,)).collect();
    let answer: Vec<_> = mini_linq::compile! {
        pipeline { stage = mini_linq::passes::stage(batch(64)); }
        struct StagedRange;
        relation R(x:i32) = (rows);
        answer(x,y) :- R(x), where x >= 60, where x <= 130,
            let y:i32 = *x * 2;
    }
    .map(|(x, y)| (*x, y))
    .collect();
    assert_eq!(answer, (60..=130).map(|x| (x, x * 2)).collect::<Vec<_>>());
}

#[test]
fn staged_scan_without_predicates_still_returns_every_row() {
    let rows = [(1_i32,), (2,), (3,)];
    let actual: Vec<_> = mini_linq::compile! {
        pipeline { stage = mini_linq::passes::stage(batch(2)); }
        struct StagedScan;
        relation R(x:i32) = (rows);
        answer(x,y) :- R(x), let y:i32 = *x + 10;
    }
    .map(|(x, y)| (*x, y))
    .collect();
    assert_eq!(actual, vec![(1, 11), (2, 12), (3, 13)]);
}

#[test]
fn staged_positional_range_keeps_the_same_survivors() {
    let rows = [(1_i64,), (4,), (7,), (12,)];
    let actual: Vec<_> = mini_linq::compile! {
        pipeline {
            select = mini_linq::passes::select(positional(R));
            stage = mini_linq::passes::stage(batch(2));
        }
        struct StagedPositional;
        relation R(x:i64) = (rows);
        answer(x) :- R(x), where x >= 4, where x < 12;
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(actual, vec![4, 7]);
}

#[test]
fn represented_and_staged_scan_composes_before_bind() {
    let rows = [(1_i32,), (4,), (7,)];
    let actual: Vec<_> = mini_linq::compile! {
        pipeline {
            represent = mini_linq::passes::represent(positions);
            stage = mini_linq::passes::stage(batch(2));
        }
        struct StagedPositions;
        relation R(x:i32) = (rows);
        answer(x,y) :- R(x), where x >= 4, let y:i32 = *x + 1;
    }
    .map(|(x, y)| (*x, y))
    .collect();
    assert_eq!(actual, vec![(4, 5), (7, 8)]);
}

#[test]
fn staged_positional_conjunctions_feed_later_column_reads() {
    let rows: Vec<_> = (0_i32..257).map(|x| (x, x % 7, x % 11)).collect();
    let actual: Vec<_> = mini_linq::compile! {
        pipeline {
            select = mini_linq::passes::select(positional(R));
            represent = mini_linq::passes::represent(positions);
            stage = mini_linq::passes::stage(batch(16));
        }
        struct StagedConjunctions;
        relation R(x:i32, y:i32, z:i32) = (rows);
        answer(x,amount) :- R(x,y,z),
            where x >= 30 && x < 220,
            where y >= 2 && y <= 5,
            where z < 4,
            let amount:i32 = *y * *z;
    }
    .map(|(x, amount)| (*x, amount))
    .collect();
    let expected: Vec<_> = (30..220)
        .filter(|x| (2..=5).contains(&(x % 7)) && x % 11 < 4)
        .map(|x| (x, (x % 7) * (x % 11)))
        .collect();
    // Canonical answers are a set; their sort order is not part of the contract.
    let actual: std::collections::BTreeSet<_> = actual.into_iter().collect();
    assert_eq!(actual, expected.into_iter().collect());
}
