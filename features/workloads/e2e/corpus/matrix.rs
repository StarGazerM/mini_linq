//! Coverage is checked against actual configuration constants emitted beside tests.
//! No compiler passes run here, and no Rust source is generated.
pub type Config = [usize; 9];
const AXIS_SIZES: [usize; 9] = [11, 2, 2, 2, 3, 2, 2, 2, 2];

pub fn verify_cover(rows: &[Config]) {
    use std::collections::BTreeSet;
    let mut observed = BTreeSet::new();
    for row in rows {
        for (axis, &value) in row.iter().enumerate() {
            assert!(value < AXIS_SIZES[axis], "unknown axis value: {row:?}");
        }
        assert!(
            row[4] == 0 || row[8] == 0,
            "positional advice requires a slice"
        );
        for a in 0..9 {
            for b in a + 1..9 {
                observed.insert((a, row[a], b, row[b]));
            }
        }
    }
    let mut required = 0;
    for a in 0..9 {
        for b in a + 1..9 {
            for x in 0..AXIS_SIZES[a] {
                for y in 0..AXIS_SIZES[b] {
                    // The only domain constraint: maps cannot take positional advice.
                    if a == 4 && b == 8 && x != 0 && y == 1 {
                        continue;
                    }
                    assert!(
                        observed.contains(&(a, x, b, y)),
                        "uncovered axes {a}={x}, {b}={y}"
                    );
                    required += 1;
                }
            }
        }
    }
    assert_eq!(required, 311);
    // Every value participates in at least one required pair, so pair coverage
    // also proves single-axis coverage. Every selected row executes real Rust
    // against its query's independent SQL goldens, including both empty scenarios.
}

pub fn verify_catalog(suite: &str, included: &[&str]) {
    use std::collections::BTreeSet;
    let actual: BTreeSet<_> = included.iter().copied().collect();
    assert_eq!(
        actual.len(),
        included.len(),
        "duplicate {suite} case registration"
    );
    let expected: BTreeSet<_> = cq_workloads::catalog::named(suite)
        .unwrap()
        .iter()
        .map(|case| case.name)
        .collect();
    assert_eq!(
        actual, expected,
        "{suite} execution must include every catalog case"
    );
}
