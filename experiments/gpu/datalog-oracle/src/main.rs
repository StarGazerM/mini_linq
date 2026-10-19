#![recursion_limit = "2048"]
use cq_gpu_runtime::{Codec, DeviceColumns, Dictionary, Runtime};
use std::{collections::BTreeSet, time::Instant};

type Pairs = BTreeSet<(i64, i64)>;
type Nodes = BTreeSet<(i64,)>;

fn closure(edges: &[(i64, i64)]) -> Pairs {
    let mut result: Pairs = edges.iter().copied().collect();
    loop {
        let old = result.clone();
        for &(x, y) in &old {
            for &(u, z) in &old {
                if y == u {
                    result.insert((x, z));
                }
            }
        }
        if old == result {
            return result;
        }
    }
}

fn run(runtime: &Runtime, name: &str, edges: &[(i64, i64)], seeds: &[(i64,)], nodes: &[(i64,)]) {
    let setup = Instant::now();
    let mut dictionary = Dictionary::default();
    let edge = runtime
        .upload_rows(edges, &mut dictionary, &[Codec::Plain, Codec::Plain])
        .unwrap();
    let seed = runtime
        .upload_rows(seeds, &mut dictionary, &[Codec::Plain])
        .unwrap();
    let node = runtime
        .upload_rows(nodes, &mut dictionary, &[Codec::Plain])
        .unwrap();
    let setup_ms = setup.elapsed().as_secs_f64() * 1000.0;
    ::cq_gpu_runtime::datalog::reset_round_count();
    let started = Instant::now();
    let (
        reach,
        linear,
        nonlinear,
        even,
        odd,
        absent,
        empty,
        distinct_edges,
        flag,
        projected,
        repeated,
        existence,
        constant,
        composite,
        copied,
    ) = cq_gpu_datalog::datalog! {
        struct Oracle;
        relation Edge(x:i64,y:i64) = (&edge);
        relation Seed(x:i64) = (&seed);
        relation Node(x:i64) = (&node);
        relation Reach(x:i64);
        relation Linear(x:i64,y:i64);
        relation Nonlinear(x:i64,y:i64);
        relation Even(x:i64);
        relation Odd(x:i64);
        relation Absent(x:i64);
        relation Empty(x:i64);
        relation Flag(x:i64,tag:i64);
        relation Projected(x:i64);
        relation Repeated(x:i64);
        relation Existence(x:i64);
        relation Constant(x:i64);
        relation Composite(x:i64,y:i64);
        relation Copied(x:i64,y:i64);
        output Reach, Linear, Nonlinear, Even, Odd, Absent, Empty, Edge, Flag,
            Projected, Repeated, Existence, Constant, Composite, Copied;
        Reach(*x) :- Seed(x);
        Reach(*y) :- Reach(x), Edge(x,y);
        Linear(*x,*y) :- Edge(x,y);
        Linear(*x,*z) :- Linear(x,y), Edge(y,z);
        Nonlinear(*x,*y) :- Edge(x,y);
        Nonlinear(*x,*z) :- Nonlinear(x,y), Nonlinear(y,z);
        Even(*x) :- Seed(x);
        Odd(*y) :- Even(x), Edge(x,y);
        Even(*y) :- Odd(x), Edge(x,y);
        Absent(*x) :- Node(x), !Reach(x);
        Flag(*x,7i64) :- Reach(x);
        Projected(*x) :- Node(x), !Edge(x,_);
        Repeated(*x) :- Node(x), !Edge(x,x);
        Existence(*x) :- Node(x), !Edge(_,_);
        Constant(*x) :- Node(x), !Edge(x,1i64);
        Composite(*x,*y) :- Nonlinear(x,y), !Edge(x,y);
        Copied(*x,*x) :- Nonlinear(x,y);
    };
    let execution_ms = started.elapsed().as_secs_f64() * 1000.0;
    let nonempty_publications = ::cq_gpu_runtime::datalog::round_count();
    // A separately compiled query consumes the resident IDB without downloading it.
    let replay: DeviceColumns<'_, (i64,)> = mini_linq::compile! {
        pipeline {
            structures = [::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)];
            print = ::cq_gpu::print;
        }
        struct Replay;
        relation Input(x:i64) = (&reach);
        answer(x) :- Input(x);
    };
    let expected_tc = closure(edges);
    let mut expected_reach: Nodes = seeds.iter().copied().collect();
    for &(seed,) in seeds {
        for &(x, y) in &expected_tc {
            if seed == x {
                expected_reach.insert((y,));
            }
        }
    }
    let mut expected_even: Nodes = seeds.iter().copied().collect();
    let mut expected_odd = Nodes::new();
    loop {
        let old_even = expected_even.clone();
        let old_odd = expected_odd.clone();
        for &(x, y) in edges {
            if old_even.contains(&(x,)) {
                expected_odd.insert((y,));
            }
            if old_odd.contains(&(x,)) {
                expected_even.insert((y,));
            }
        }
        if old_even == expected_even && old_odd == expected_odd {
            break;
        }
    }
    let actual_reach = replay.to_host().unwrap();
    assert_eq!(
        actual_reach.len(),
        expected_reach.len(),
        "{name}: duplicate reach rows"
    );
    assert_eq!(
        actual_reach.into_iter().collect::<Nodes>(),
        expected_reach,
        "{name}: reach"
    );
    for (label, answer) in [("linear", linear), ("nonlinear", nonlinear)] {
        let rows = answer.to_host().unwrap();
        assert_eq!(rows.len(), expected_tc.len(), "{name}: {label} duplicates");
        assert_eq!(
            rows.into_iter().collect::<Pairs>(),
            expected_tc,
            "{name}: {label}"
        );
    }
    assert_eq!(
        even.to_host().unwrap().into_iter().collect::<Nodes>(),
        expected_even,
        "{name}: even"
    );
    assert_eq!(
        odd.to_host().unwrap().into_iter().collect::<Nodes>(),
        expected_odd,
        "{name}: odd"
    );
    let expected_absent: Nodes = nodes
        .iter()
        .copied()
        .filter(|n| !expected_reach.contains(n))
        .collect();
    assert_eq!(
        absent.to_host().unwrap().into_iter().collect::<Nodes>(),
        expected_absent,
        "{name}: negation"
    );
    assert!(empty.to_host().unwrap().is_empty(), "{name}: empty IDB");
    assert_eq!(
        distinct_edges
            .to_host()
            .unwrap()
            .into_iter()
            .collect::<Pairs>(),
        edges.iter().copied().collect(),
        "{name}: EDB output"
    );
    assert_eq!(
        flag.to_host().unwrap().into_iter().collect::<Pairs>(),
        expected_reach.iter().map(|&(x,)| (x, 7)).collect(),
        "{name}: literal head"
    );
    let expected_projected: Nodes = nodes
        .iter()
        .copied()
        .filter(|&(x,)| !edges.iter().any(|&(a, _)| a == x))
        .collect();
    assert_eq!(
        projected.to_host().unwrap().into_iter().collect::<Nodes>(),
        expected_projected,
        "{name}: projected negative key"
    );
    let expected_repeated: Nodes = nodes
        .iter()
        .copied()
        .filter(|&(x,)| !edges.contains(&(x, x)))
        .collect();
    assert_eq!(
        repeated.to_host().unwrap().into_iter().collect::<Nodes>(),
        expected_repeated,
        "{name}: repeated negative key"
    );
    let expected_existence: Nodes = if edges.is_empty() {
        nodes.iter().copied().collect()
    } else {
        Nodes::new()
    };
    assert_eq!(
        existence.to_host().unwrap().into_iter().collect::<Nodes>(),
        expected_existence,
        "{name}: existence key"
    );
    let expected_constant: Nodes = nodes
        .iter()
        .copied()
        .filter(|&(x,)| !edges.contains(&(x, 1)))
        .collect();
    assert_eq!(
        constant.to_host().unwrap().into_iter().collect::<Nodes>(),
        expected_constant,
        "{name}: constant negative key"
    );
    assert_eq!(
        composite.to_host().unwrap().into_iter().collect::<Pairs>(),
        expected_tc
            .iter()
            .copied()
            .filter(|pair| !edges.contains(pair))
            .collect(),
        "{name}: composite negative key"
    );
    let expected_copied: Pairs = expected_tc.iter().map(|&(x, _)| (x, x)).collect();
    let copied_rows = copied.to_host().unwrap();
    assert_eq!(
        copied_rows.len(),
        expected_copied.len(),
        "{name}: lossy repeated projection duplicates"
    );
    assert_eq!(
        copied_rows.into_iter().collect::<Pairs>(),
        expected_copied,
        "{name}: copied heads"
    );
    println!(
        "scenario={name} exact=true nodes={} edges={} closure={} nonempty_publications={nonempty_publications} setup_ms={setup_ms:.3} execution_ms={execution_ms:.3}",
        nodes.len(),
        edges.len(),
        expected_tc.len()
    );
}

fn main() {
    let runtime = Runtime::new().unwrap();
    // One lifetime-wide counter scope: cached runtime scratch may outlive a
    // scenario, so closing per-scenario scopes would misattribute later frees.
    let persistent_bytes = runtime.pool().resource().push_counters().bytes.value;
    let nodes: Vec<_> = (0..8).map(|x| (x,)).collect();
    for (name, edges, seeds) in [
        ("empty", vec![], vec![]),
        ("self-loop", vec![(0, 0)], vec![(0,)]),
        (
            "chain-alternating-deltas",
            vec![(0, 1), (1, 2), (2, 3), (3, 4)],
            vec![(0,)],
        ),
        ("cycle", vec![(0, 1), (1, 2), (2, 0)], vec![(0,)]),
        (
            "disconnected",
            vec![(0, 1), (1, 2), (4, 5), (5, 4)],
            vec![(0,)],
        ),
        (
            "duplicates",
            vec![(0, 1), (0, 1), (1, 2), (1, 2)],
            vec![(0,), (0,)],
        ),
        ("empty-seeds", vec![(0, 1), (1, 2)], vec![]),
    ] {
        run(&runtime, name, &edges, &seeds, &nodes);
    }
    if std::env::args().any(|arg| arg == "--bench") {
        let nodes: Vec<_> = (0..192).map(|x| (x,)).collect();
        let edges: Vec<_> = (0..191).map(|x| (x, x + 1)).collect();
        run(&runtime, "moderate-chain-192", &edges, &[(0,)], &nodes);
        let value = |x: i64| 3_000_000_000 + x * 17;
        let nodes: Vec<_> = (0..192).map(|x| (value(x),)).collect();
        let mut edges = Vec::new();
        for block in 0..6 {
            for local in 0..32 {
                for step in [1, 5] {
                    edges.push((
                        value(block * 32 + local),
                        value(block * 32 + (local + step) % 32),
                    ));
                }
            }
            if block < 5 {
                edges.push((value(block * 32 + 31), value((block + 1) * 32)));
            }
        }
        let duplicates: Vec<_> = edges.iter().step_by(7).copied().collect();
        edges.extend(duplicates);
        run(
            &runtime,
            "moderate-branching-cycles-wide-duplicates-192",
            &edges,
            &[(value(0),)],
            &nodes,
        );
    }
    let memory = runtime.pool().resource().pop_counters().unwrap();
    println!(
        "scope=all-scenarios peak_device_bytes={} allocations={}",
        persistent_bytes + memory.bytes.peak,
        memory.allocations.total
    );
}
