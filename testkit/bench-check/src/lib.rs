//! Timing-only validation. Correctness tests must retain complete comparisons.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

pub type Row = Vec<String>;
pub type Witness = BTreeMap<String, RelationWitness>;
pub type References = BTreeMap<String, BTreeSet<Row>>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Full,
    Sampled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Policy {
    pub mode: Mode,
    pub seed: u64,
    pub sample_size: usize,
}

impl Policy {
    pub fn full() -> Self {
        Self { mode: Mode::Full, seed: 20261103, sample_size: 1000 }
    }

    /// The controller supplies an already key/outer-sample-derived seed. Warmups
    /// use `full()` explicitly and do not advance the measured sample ordinal.
    pub fn from_env(measured_ordinal: usize) -> Result<Self, String> {
        let mode = match std::env::var("BENCH_CHECK_MODE").as_deref() {
            Ok("full") | Err(std::env::VarError::NotPresent) => Mode::Full,
            Ok("sampled") => Mode::Sampled,
            _ => return Err("BENCH_CHECK_MODE must be full or sampled".into()),
        };
        let seed = env_number("BENCH_CHECK_SEED", 20261103)?;
        let sample_size = usize::try_from(env_number("BENCH_CHECK_SIZE", 1000)?)
            .map_err(|_| "BENCH_CHECK_SIZE exceeds usize")?;
        if sample_size == 0 {
            return Err("BENCH_CHECK_SIZE must be positive".into());
        }
        Ok(Self {
            mode: if measured_ordinal == 0 { mode } else { Mode::Sampled },
            seed: if measured_ordinal == 0 { seed } else {
                stable_seed(seed, &measured_ordinal.to_string())
            },
            sample_size,
        })
    }
}

fn env_number(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Ok(value) => value.parse().map_err(|_| format!("{name} must be an unsigned integer")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("{name}: {error}")),
    }
}

/// Stable FNV-1a mixing, never a process-randomized hash or correctness digest.
pub fn stable_seed(seed: u64, label: &str) -> u64 {
    label.bytes().fold(seed ^ 0xcbf29ce484222325, |state, byte| {
        (state ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

/// SplitMix64 with rejection sampling; bounded draws have no modulo bias.
pub struct Random(u64);
impl Random {
    pub fn new(seed: u64) -> Self { Self(seed) }
    pub fn below(&mut self, bound: usize) -> usize {
        assert!(bound > 0);
        let bound = bound as u64;
        let threshold = bound.wrapping_neg() % bound;
        loop {
            self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
            let mut value = self.0;
            value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
            value ^= value >> 31;
            if value >= threshold { return (value % bound) as usize; }
        }
    }
}

/// Uniform distinct actual-row positions, using O(sample_size) scratch space.
/// Full callers should iterate their relation directly instead of allocating
/// its complete position vector.
pub fn sample_indices(count: usize, relation: &str, policy: Policy) -> Vec<usize> {
    if policy.mode == Mode::Full || count <= policy.sample_size {
        return (0..count).collect();
    }
    let mut random = Random::new(stable_seed(policy.seed, relation));
    let mut positions = BTreeSet::new();
    for last in count - policy.sample_size..count {
        let selected = random.below(last + 1);
        if !positions.insert(selected) { positions.insert(last); }
    }
    positions.into_iter().collect()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RelationEvidence {
    pub mode: Mode,
    pub expected_count: usize,
    pub actual_count: usize,
    pub sampled_count: usize,
    pub seed: u64,
    pub validation_seconds: f64,
}

/// `actual_count` is the exact logical cardinality, not an unchecked bag length.
/// The accessor canonicalizes only the requested rows. Reference indexing and
/// actual set canonicalization (when needed) belong outside this call.
pub fn check_relation<T: Ord>(
    reference: &BTreeSet<T>,
    relation: &str,
    actual_count: usize,
    mut row_at: impl FnMut(usize) -> T,
    policy: Policy,
) -> Result<RelationEvidence, String> {
    let started = Instant::now();
    if policy.sample_size == 0 { return Err("sample size must be positive".into()); }
    check_count(reference.len(), actual_count, relation)?;
    let sampled_count;
    if policy.mode == Mode::Full {
        let actual: BTreeSet<_> = (0..actual_count).map(&mut row_at).collect();
        if &actual != reference {
            return Err(format!("{relation}: complete tuple sets differ"));
        }
        sampled_count = actual_count;
    } else {
        let positions = sample_indices(actual_count, relation, policy);
        sampled_count = positions.len();
        for position in positions {
            if !reference.contains(&row_at(position)) {
                return Err(format!("{relation}: sampled actual tuple at {position} is absent from reference"));
            }
        }
    }
    Ok(RelationEvidence {
        mode: policy.mode, expected_count: reference.len(), actual_count,
        sampled_count, seed: policy.seed, validation_seconds: started.elapsed().as_secs_f64(),
    })
}

fn check_count(expected: usize, actual: usize, relation: &str) -> Result<(), String> {
    if actual != expected {
        return Err(format!("{relation}: expected {expected} rows, got {actual}"));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RelationWitness {
    pub count: usize,
    pub rows: Vec<Row>,
}

pub fn make_witness(
    relation: &str,
    count: usize,
    mut row_at: impl FnMut(usize) -> Row,
    policy: Policy,
) -> RelationWitness {
    let rows = if policy.mode == Mode::Full {
        (0..count).map(&mut row_at).collect()
    } else {
        sample_indices(count, relation, policy).into_iter().map(row_at).collect()
    };
    RelationWitness { count, rows }
}

pub fn write_witness(path: &Path, witness: &Witness) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut output = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut output, witness).map_err(|error| error.to_string())?;
    std::io::Write::flush(&mut output).map_err(|error| error.to_string())
}

/// Validate transported witnesses without cloning their tuple strings.
pub fn check_witness(
    references: &References,
    witness: &Witness,
    policy: Policy,
) -> Result<BTreeMap<String, RelationEvidence>, String> {
    if policy.sample_size == 0 { return Err("sample size must be positive".into()); }
    if !references.keys().eq(witness.keys()) {
        return Err("output relation names differ from reference".into());
    }
    let mut evidence = BTreeMap::new();
    for (name, expected) in references {
        let started = Instant::now();
        let actual = &witness[name];
        check_count(expected.len(), actual.count, name)?;
        let required = if policy.mode == Mode::Full { actual.count }
            else { actual.count.min(policy.sample_size) };
        if actual.rows.len() != required {
            return Err(format!("{name}: required {required} witness tuples, got {}", actual.rows.len()));
        }
        let unique: BTreeSet<_> = actual.rows.iter().collect();
        if unique.len() != actual.rows.len() {
            return Err(format!("{name}: duplicate tuples in a distinct-output witness"));
        }
        if policy.mode == Mode::Full {
            if !expected.iter().eq(unique.into_iter()) {
                return Err(format!("{name}: complete tuple sets differ"));
            }
        } else if actual.rows.iter().any(|row| !expected.contains(row)) {
            return Err(format!("{name}: sampled actual tuple is absent from reference"));
        }
        evidence.insert(name.clone(), RelationEvidence {
            mode: policy.mode, expected_count: expected.len(), actual_count: actual.count,
            sampled_count: actual.rows.len(), seed: policy.seed,
            validation_seconds: started.elapsed().as_secs_f64(),
        });
    }
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn samples_are_reproducible_distinct_bounded_and_change_with_seed() {
        let p = Policy { mode: Mode::Sampled, seed: 7, sample_size: 1000 };
        let a = sample_indices(1_000_000, "answer", p);
        assert_eq!(a.len(), 1000);
        assert!(a.windows(2).all(|w| w[0] < w[1]));
        assert!(a.iter().all(|&i| i < 1_000_000));
        assert_eq!(a, sample_indices(1_000_000, "answer", p));
        assert_ne!(a, sample_indices(1_000_000, "other", p));
        assert_ne!(a, sample_indices(1_000_000, "answer", Policy { seed: 8, ..p }));
    }

    #[test]
    fn checks_full_counts_and_selected_actual_membership() {
        let expected: BTreeSet<_> = (0..2000).collect();
        let full = Policy::full();
        assert!(check_relation(&expected, "r", 2000, |i| i, full).is_ok());
        assert!(check_relation(&expected, "r", 2000, |i| if i == 1 { 0 } else { i }, full).is_err());
        let sampled = Policy { mode: Mode::Sampled, ..full };
        let calls = Cell::new(0);
        let checked = check_relation(&expected, "r", 2000, |i| { calls.set(calls.get()+1); i }, sampled).unwrap();
        assert_eq!(checked.sampled_count, 1000);
        assert_eq!(calls.get(), 1000);
        assert!(check_relation(&expected, "r", 1999, |i| i, sampled).is_err());
        let changed = sample_indices(2000, "r", sampled)[0];
        assert!(check_relation(&expected, "r", 2000, |i| if i == changed { 9000 } else { i }, sampled).is_err());
    }

    #[test]
    fn small_and_empty_relations_check_every_actual_tuple() {
        let sampled = Policy { mode: Mode::Sampled, ..Policy::full() };
        let expected = BTreeSet::from([1, 2]);
        assert_eq!(check_relation(&expected, "small", 2, |i| i+1, sampled).unwrap().sampled_count, 2);
        assert!(check_relation(&expected, "small", 2, |i| i+2, sampled).is_err());
        assert_eq!(check_relation(&BTreeSet::<usize>::new(), "empty", 0, |_| unreachable!(), sampled).unwrap().sampled_count, 0);
    }

    #[test]
    fn witnesses_require_all_relations_counts_and_unique_rows() {
        let reference = BTreeMap::from([("r".into(), BTreeSet::from([vec!["1.0".into()],vec!["1".into()]]))]);
        let valid = BTreeMap::from([("r".into(), RelationWitness {count:2, rows:vec![vec!["1".into()],vec!["1.0".into()]]})]);
        assert!(check_witness(&reference, &valid, Policy::full()).is_ok());
        assert!(check_witness(&reference, &Witness::new(), Policy::full()).is_err());
        let duplicate = BTreeMap::from([("r".into(), RelationWitness {count:2, rows:vec![vec!["1".into()],vec!["1".into()]]})]);
        assert!(check_witness(&reference, &duplicate, Policy::full()).is_err());
    }
}
