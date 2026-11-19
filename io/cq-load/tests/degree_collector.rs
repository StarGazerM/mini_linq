use std::collections::{BTreeMap, BTreeSet};

use cq_evidence::{Claim, DegreeRequest, Provenance, Reach, Selection, Source, Unit};
use cq_load::stats::{DegreeCollector, bind_degrees};

type Row = (u32, Vec<i64>);
type Relations = BTreeMap<String, Vec<Row>>;

fn selection(relation: &str) -> Selection { Selection { relation: relation.to_owned(), fixed: Vec::new() } }
fn selected(row: &[i64], selection: &Selection) -> bool { selection.fixed.iter().all(|&(column, value)| row[column] == value) }
fn projection(row: &[i64], columns: &[usize]) -> Vec<i64> { columns.iter().map(|&column| row[column]).collect() }
fn normalize(n: u64, d: u64) -> (u64, u64) { if d == 0 { (0, 1) } else { (n, d) } }

// Deliberately independent of the collector: row tuples, set projections,
// nested-loop equality joins, and a map from values to their earliest round.
fn reference(request: &DegreeRequest, relations: &Relations) -> (u64, u64) {
    match request {
        DegreeRequest::Distinct { rows, columns } => {
            let values: BTreeSet<_> = relations[&rows.relation].iter().filter(|(_, row)| selected(row, rows))
                .map(|(_, row)| projection(row, columns)).collect();
            (values.len() as u64, 1)
        }
        DegreeRequest::Joined { reach, binder, from, read, bound, column, staged } => {
            let (mut numerator, mut denominator) = (0, 0);
            for (round, row) in &relations[&binder.relation] {
                if !selected(row, binder) { continue; }
                let mut values = BTreeMap::new();
                for (seen, candidate) in &relations[&read.relation] {
                    if selected(candidate, read) && from.iter().zip(bound).all(|(&a, &b)| row[a] == candidate[b]) {
                        values.entry(candidate[*column]).and_modify(|old: &mut u32| *old = (*old).min(*seen)).or_insert(*seen);
                    }
                }
                let met = |round: u32| values.values().filter(|&&first| !staged || first <= round).count() as u64;
                if let Some(reach) = reach {
                    for (round, changing) in &relations[&reach.changing.relation] {
                        if selected(changing, &reach.changing) && reach.columns.iter().zip(&reach.binder).all(|(&a, &b)| changing[a] == row[b]) {
                            numerator += met(*round);
                            denominator += 1;
                        }
                    }
                } else {
                    numerator += met(*round);
                    denominator += 1;
                }
            }
            normalize(numerator, denominator)
        }
        DegreeRequest::Visible { changing, relation } => {
            let mut numerator = 0;
            for (round, _) in &relations[changing] {
                for (seen, _) in &relations[relation] { if seen <= round { numerator += 1; } }
            }
            normalize(numerator, (relations[changing].len() as u64) * (relations[relation].len() as u64))
        }
        DegreeRequest::Rounds { relation } => {
            let rounds: BTreeSet<_> = relations[relation].iter().map(|(round, _)| *round).collect();
            (rounds.len() as u64, 1)
        }
    }
}

struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize { (self.next() % n as u64) as usize }
}

fn data(seed: u64) -> Relations {
    let mut random = Random(seed);
    let mut relations = BTreeMap::new();
    for (id, (arity, size)) in [(2, 200), (3, 600), (4, 2000), (5, 0)].into_iter().enumerate() {
        let mut rows = Vec::new();
        let mut unique = BTreeSet::new();
        while rows.len() < size {
            let mut row = Vec::new();
            for column in 0..arity {
                // A small, skewed domain, with signed values and relation-local
                // rare values: frequent matches coexist with many missed probes.
                let draw = random.below(10);
                let value = if draw < 5 { 0 } else { random.below(if column == arity - 1 { 64 } else { 16 }) as i64 - 4 };
                row.push(if value > 4 { value + id as i64 * 19 } else { value });
            }
            if unique.insert(row.clone()) { rows.push((random.below(13) as u32, row)); }
        }
        relations.insert(format!("r{id}"), rows);
    }
    relations
}

fn requests() -> Vec<DegreeRequest> {
    let names = ["r0", "r1", "r2", "r3"];
    let arities = [2, 3, 4, 5];
    let mut random = Random(0xd1b5_4a32_d192_ed03);
    let mut requests = Vec::new();
    for i in 0..360 {
        let b = random.below(4);
        let r = if i % 5 == 0 { b } else { random.below(4) };
        let mut binder = selection(names[b]);
        let mut read = selection(names[r]);
        if i % 3 == 0 { binder.fixed.push((0, if i % 11 == 0 { 777 } else { 0 })); }
        if i % 4 == 0 { read.fixed.push((0, 0)); }
        if i % 17 == 0 { read.fixed.push((arities[r] - 1, -1)); }
        if i % 6 == 0 {
            let columns = match (i / 6) % 3 { 0 => Vec::new(), 1 => vec![arities[b] - 1], _ => (0..arities[b]).rev().collect() };
            requests.push(DegreeRequest::Distinct { rows: binder, columns });
        } else if i % 6 == 1 {
            requests.push(DegreeRequest::Visible { changing: names[b].to_owned(), relation: names[r].to_owned() });
            if i % 12 == 1 { requests.push(DegreeRequest::Rounds { relation: names[r].to_owned() }); }
        } else {
            let width = random.below(arities[b].min(arities[r] - 1) + 1);
            let from = (0..width).rev().collect();
            let bound = (0..width).collect();
            let reach = if i % 2 == 0 {
                let changing = random.below(4);
                let width = 1 + random.below(arities[b].min(arities[changing]));
                let mut changing_rows = selection(names[changing]);
                if i % 5 == 0 { changing_rows.fixed.push((0, 0)); }
                Some(Reach { changing: changing_rows, columns: (0..width).collect(), binder: (0..width).rev().collect() })
            } else { None };
            requests.push(DegreeRequest::Joined { reach, binder, from, read, bound, column: arities[r] - 1, staged: (i / 2) % 2 == 0 });
        }
    }
    // Exact duplicate requests exercise shared structures, repeated output, and
    // release accounting independently from the numerous partially shared ones.
    requests.extend_from_within(..24);
    requests
}

fn collect(requests: &[DegreeRequest], relations: &Relations, threads: usize) -> Vec<(DegreeRequest, u64, u64)> {
    let mut collector = DegreeCollector::new(requests.to_vec());
    assert!(collector.relation("not requested").is_none());
    for (name, rows) in relations {
        let Some(id) = collector.relation(name) else { continue };
        for (round, row) in rows { collector.push(id, *round, row); }
        assert_eq!(collector.rows(id), rows.len());
    }
    collector.finish(threads).unwrap()
}

#[test]
fn differential_random_degrees() {
    let requests = requests();
    assert!(requests.len() >= 300);
    for seed in [0x243f_6a88_85a3_08d3, 0x1319_8a2e_0370_7344] {
        let relations = data(seed);
        let expected: Vec<_> = requests.iter().map(|request| {
            let (numerator, denominator) = reference(request, &relations);
            (request.clone(), numerator, denominator)
        }).collect();
        let serial = collect(&requests, &relations, 1);
        let parallel = collect(&requests, &relations, 4);
        assert_eq!(serial, expected, "serial seed {seed}");
        assert_eq!(parallel, expected, "parallel seed {seed}");
        assert_eq!(serial, parallel);
    }
}

#[test]
fn staged_first_round_and_pair_multiplicities() {
    let relations = BTreeMap::from([
        ("b".to_owned(), vec![(9, vec![1, 2, 0]), (0, vec![1, 2, 1]), (1, vec![3, 4, 0])]),
        ("r".to_owned(), vec![(8, vec![2, 7, 0]), (2, vec![2, 7, 1]), (4, vec![2, 8, 0]), (0, vec![9, 9, 0])]),
        ("x".to_owned(), vec![(2, vec![1, 0]), (2, vec![1, 1]), (4, vec![1, 2]), (0, vec![7, 0])]),
    ]);
    let reached = DegreeRequest::Joined {
        reach: Some(Reach { changing: selection("x"), columns: vec![0], binder: vec![0] }),
        binder: selection("b"), from: vec![1], read: selection("r"), bound: vec![0], column: 1, staged: true,
    };
    let mut unstaged = reached.clone();
    if let DegreeRequest::Joined { staged, .. } = &mut unstaged { *staged = false; }
    let requests = vec![reached, unstaged, DegreeRequest::Distinct { rows: selection("b"), columns: vec![] }];
    let result = collect(&requests, &relations, 4);
    assert_eq!((result[0].1, result[0].2), (8, 6));
    assert_eq!((result[1].1, result[1].2), (12, 6));
    assert_eq!((result[2].1, result[2].2), (1, 1));
    for (request, n, d) in result { assert_eq!((n, d), reference(&request, &relations)); }
}

#[test]
fn short_row_is_reported_at_finish() {
    let requests = vec![DegreeRequest::Distinct { rows: selection("r"), columns: vec![3] }];
    let mut collector = DegreeCollector::new(requests);
    let id = collector.relation("r").unwrap();
    collector.push(id, 0, &[1, 2]);
    assert_eq!(collector.rows(id), 1);
    assert!(collector.finish(4).is_err());
}

#[test]
fn binds_request_identity_and_unit() {
    let requests = vec![
        DegreeRequest::Distinct { rows: selection("r"), columns: vec![0] },
        DegreeRequest::Joined { reach: None, binder: selection("b"), from: vec![0], read: selection("r"), bound: vec![0], column: 1, staged: true },
        DegreeRequest::Visible { changing: "b".to_owned(), relation: "r".to_owned() },
    ];
    let provenance = Provenance { producer: "test".to_owned(), method: "exact degrees".to_owned(), locator: "memory".to_owned(), run: None, artifact_hash: None };
    let facts = bind_degrees(requests.iter().cloned().map(|r| (r, 11, 7)).collect(), "sha256:dataset", Source::Exact, Claim::Equal, provenance.clone());
    for ((fact, request), unit) in facts.iter().zip(&requests).zip([Unit::Distinct, Unit::Degree, Unit::Ratio]) {
        assert_eq!(&fact.request, request);
        assert_eq!(fact.value.id, request.id());
        assert_eq!(fact.value.unit, request.unit());
        assert_eq!(fact.value.unit, unit);
        assert_eq!(fact.value.source, Source::Exact);
        assert_eq!(fact.value.claim, Claim::Equal);
        assert_eq!(fact.value.dataset_hash, "sha256:dataset");
        assert_eq!(fact.value.provenance, provenance);
        assert!(fact.value.lower.is_none());
        assert!(fact.value.upper.is_none());
        let value = fact.value.value.as_ref().unwrap();
        assert_eq!(value.numerator, "11");
        assert_eq!(value.denominator, "7");
    }
}

#[test]
fn empty_requests_and_zero_threads() {
    assert!(DegreeCollector::new(Vec::new()).finish(0).unwrap().is_empty());
    let requests = vec![DegreeRequest::Visible { changing: "r".to_owned(), relation: "r".to_owned() }];
    let mut collector = DegreeCollector::new(requests);
    let id = collector.relation("r").unwrap();
    collector.push(id, u32::MAX, &[]);
    collector.push(id, 0, &[1]);
    let values = collector.finish(0).unwrap();
    assert_eq!((values[0].1, values[0].2), (3, 4));
}

#[test]
fn records_wider_than_eight_cells_agree_with_the_reference() {
    // Nine key columns and the round: the merge path of the record sort.
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let mut unique = BTreeSet::new();
    while unique.len() < 500 { unique.insert((0..9).map(|_| random.below(3) as i64).collect::<Vec<_>>()); }
    let rows: Vec<Row> = unique.into_iter().map(|row| (random.below(5) as u32, row)).collect();
    let relations = Relations::from([("wide".to_owned(), rows)]);
    let requests = vec![
        DegreeRequest::Distinct { rows: selection("wide"), columns: (0..9).rev().collect() },
        DegreeRequest::Joined { reach: None, binder: selection("wide"), from: (0..8).collect(), read: selection("wide"), bound: (0..8).collect(), column: 8, staged: true },
    ];
    let values = collect(&requests, &relations, 2);
    for (request, numerator, denominator) in &values { assert_eq!((*numerator, *denominator), reference(request, &relations), "{}", request.id()); }
    assert_eq!(values[0].1, 500);
}

/// The degrees as the runtime keeps them (PS-Admit): every row admitted,
/// each relation's rows in round order.
fn admitted(requests: &[DegreeRequest], relations: &Relations, arities: &[(&str, usize)]) -> Vec<(DegreeRequest, u64, u64)> {
    let mut degrees = cq_load::stats::admitted_degrees(requests, arities).unwrap();
    for (at, (name, _)) in arities.iter().enumerate() {
        let mut rows: Vec<&Row> = relations[*name].iter().collect();
        rows.sort_by_key(|row| row.0);
        for (round, row) in rows { degrees.admit(at, *round, *round, row); }
    }
    requests.iter().cloned().zip(degrees.finish()).map(|(request, (numerator, denominator))| (request, numerator, denominator)).collect()
}

/// The greatest relative error of the admitted degrees of one kind of
/// request, and their mean, against the reference.
fn errors(requests: &[DegreeRequest], relations: &Relations, kind: impl Fn(&DegreeRequest) -> bool) -> (f64, f64, usize) {
    let arities = [("r0", 2), ("r1", 3), ("r2", 4), ("r3", 5)];
    let (mut worst, mut sum, mut count) = (0.0f64, 0.0, 0);
    for (request, numerator, denominator) in admitted(requests, relations, &arities) {
        if !kind(&request) { continue; }
        let (exact, over) = reference(&request, relations);
        let (estimated, exact) = (numerator as f64 / denominator as f64, exact as f64 / over as f64);
        let error = if exact == 0.0 { estimated } else { (estimated - exact).abs() / exact };
        worst = worst.max(error);
        sum += error;
        count += 1;
    }
    (worst, sum / count.max(1) as f64, count)
}

#[test]
fn admitted_degrees_agree_with_the_reference() {
    // Relations small enough that every key is sampled: what differs from
    // the reference is a sketch's error.
    let requests = requests();
    for seed in [0x243f_6a88_85a3_08d3, 0x1319_8a2e_0370_7344] {
        let relations = data(seed);
        let (worst, _, count) = errors(&requests, &relations, |request| matches!(request, DegreeRequest::Visible { .. }));
        assert!(count > 0 && worst < 1e-12, "visible {worst}");
        // A 4,096-register sketch: 1.6% standard error; three of them.
        let (worst, _, count) = errors(&requests, &relations, |request| matches!(request, DegreeRequest::Distinct { .. }));
        assert!(count > 0 && worst <= 0.05, "distinct {worst}");
        // A key of more than 24 values counts them in a 256-register sketch.
        for (reached, staged) in [(false, false), (false, true), (true, false), (true, true)] {
            let (worst, mean, count) = errors(&requests, &relations, |request| matches!(request, DegreeRequest::Joined { reach, staged: s, .. } if reach.is_some() == reached && *s == staged));
            assert!(count > 0 && worst <= 0.08 && mean <= 0.01, "joined, reach {reached}, staged {staged}: worst {worst}, mean {mean}");
        }
    }
}

#[test]
fn a_sample_of_the_keys_estimates_a_matched_degree() {
    // 40,000 binder keys, five times the capacity of a table: the key `k`
    // holds `1 + k % 7` values in the read, the value `v` admitted in round
    // `v`; the binder holds every key twice, in round 3.
    let keys = 40_000i64;
    let relations = Relations::from([
        ("b".to_owned(), (0..keys).flat_map(|k| [(3, vec![k, 0]), (3, vec![k, 1])]).collect()),
        ("r".to_owned(), (0..keys).flat_map(|k| (0..=k % 7).map(move |v| (v as u32, vec![k, v]))).collect()),
    ]);
    let joined = |staged| DegreeRequest::Joined { reach: None, binder: selection("b"), from: vec![0], read: selection("r"), bound: vec![0], column: 1, staged };
    let values = admitted(&[joined(false), joined(true)], &relations, &[("b", 2), ("r", 2)]);
    let value = |at: usize| values[at].1 as f64 / values[at].2 as f64;
    // The mean of 1..=7, and of the values admitted by round 3: min(1 + k % 7, 4).
    assert!((value(0) - 4.0).abs() <= 0.1, "{}", value(0));
    assert!((value(1) - 22.0 / 7.0).abs() <= 0.1, "{}", value(1));
    // Fewer binder rows than there are: a sample.
    assert!(values[0].2 < 2 * keys as u64 / 2, "{}", values[0].2);
}

#[test]
fn exact_small_cases_of_admitted_degrees() {
    // b reaches r on its second column; x reaches b on its first.
    let relations = BTreeMap::from([
        ("b".to_owned(), vec![(0, vec![1, 2]), (0, vec![1, 3]), (0, vec![5, 9])]),
        ("r".to_owned(), vec![(0, vec![2, 7]), (1, vec![2, 8]), (0, vec![3, 7]), (0, vec![4, 4])]),
        ("x".to_owned(), vec![(0, vec![1]), (0, vec![6])]),
    ]);
    let joined = |reach: bool, staged: bool| DegreeRequest::Joined {
        reach: reach.then(|| Reach { changing: selection("x"), columns: vec![0], binder: vec![0] }),
        binder: selection("b"), from: vec![1], read: selection("r"), bound: vec![0], column: 1, staged,
    };
    let requests = vec![
        joined(false, false), joined(false, true), joined(true, false), joined(true, true),
        DegreeRequest::Distinct { rows: selection("r"), columns: vec![1] },
        DegreeRequest::Distinct { rows: Selection { relation: "r".to_owned(), fixed: vec![(0, 2)] }, columns: vec![] },
        DegreeRequest::Distinct { rows: Selection { relation: "r".to_owned(), fixed: vec![(0, 5)] }, columns: vec![] },
        DegreeRequest::Visible { changing: "b".to_owned(), relation: "r".to_owned() },
        DegreeRequest::Rounds { relation: "r".to_owned() },
        DegreeRequest::Rounds { relation: "x".to_owned() },
    ];
    let values = admitted(&requests, &relations, &[("b", 2), ("r", 2), ("x", 1)]);
    // b's keys 2, 3, 9 meet 2, 1 and 0 values of r, of which 8 arrives a
    // round after b; x = 1 reaches b's first two rows. r has rows of two
    // rounds, x of one.
    let found: Vec<_> = values.iter().map(|(_, numerator, denominator)| (*numerator, *denominator)).collect();
    assert_eq!(found, [(3, 3), (2, 3), (3, 2), (2, 2), (3, 1), (1, 1), (0, 1), (9, 12), (2, 1), (1, 1)]);
    for (request, numerator, denominator) in &values { assert_eq!((*numerator, *denominator), reference(request, &relations), "{}", request.id()); }
}
