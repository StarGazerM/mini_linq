//! Complete-set comparisons through the public Datalog and GPU Sequence pipeline.
use oracle_shared::{Data, RANDOM_SEEDS, Rows, SCENARIOS, generate};
use std::time::{Duration, Instant};

oracle_shared::oracle_programs!(
    gpu,
    mini,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline {
        incremental = ::cq_datalog::passes::incremental(driver(delta_first););
        storage_rewrites = [::cq_gpu_datalog::device_storage];
        cq = pipeline {
            physical = [::cq_gpu_sequence::capture_constants];
            structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
            sequence = ::mini_linq::passes::sequence;
            stage = ::cq_gpu_sequence::stage;
            decompose = [::cq_gpu_sequence::decompose, ::cq_sequence_rewrites::output(reserve_previous)];
            grade = ::cq_gpu_sequence::machine_grade;
            print = ::cq_gpu_sequence::translate_arena;
        };
        terminals = [::cq_gpu_sequence::translate_arena => batch];
        round = [::cq_gpu_datalog::bind_round(binding = 5, head_cursor)];
        print = ::cq_gpu_datalog::print_rounds;
    }
);

type Evaluate = fn(&Data) -> (Rows, Duration);

struct Case {
    name: &'static str,
    gpu: Evaluate,
    mini: Evaluate,
    mini_par: Evaluate,
}

macro_rules! cases {
    ($($name:ident),* $(,)?) => {
        const CASES: &[Case] = &[$(Case {
            name: stringify!($name),
            gpu: gpu::$name::mini,
            mini: oracle_mini::$name::mini,
            mini_par: oracle_mini_par::$name::mini,
        }),*];
    };
}

cases! {
    reach, tc_left, tc_right, tc_nonlinear, same_generation,
    mutual, multiple_seeds, guard, unreachable, nonedges,
    recursive_upper, andersen, cspa,
}

#[test]
fn all_oracle_programs_match_cpu_on_every_fixture() {
    // Keep GPU runs sequential even when the Rust test harness has many workers.
    // In particular, negation covers both completed recursive lower strata and
    // EDB membership, including recursive rules in an upper stratum.
    for case in CASES {
        for (scenario, seed) in SCENARIOS
            .iter()
            .map(|&scenario| (scenario, 0x5eed))
            .chain((0..RANDOM_SEEDS).map(|seed| ("random", seed)))
        {
            let data = generate(scenario, 6, seed);
            let expected = (case.mini)(&data).0;
            assert_eq!(
                (case.gpu)(&data).0,
                expected,
                "{} / {scenario} / seed={seed}",
                case.name,
            );
        }
    }
}

#[test]
fn resident_if_let_preserves_declared_scalar_metadata() {
    let edges: Vec<(i32, i32)> = vec![(1, 2)];
    let (r, s) = cq_datalog::datalog! {
        pipeline {
            incremental = ::cq_datalog::passes::incremental(driver(delta_first););
            storage_rewrites = [::cq_gpu_datalog::device_storage];
            cq = pipeline {
                physical = [::cq_gpu_sequence::capture_constants];
                structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
                sequence = ::mini_linq::passes::sequence;
                stage = ::cq_gpu_sequence::stage;
                decompose = [::cq_gpu_sequence::decompose];
                grade = ::cq_gpu_sequence::machine_grade;
                print = ::cq_gpu_sequence::translate_arena;
            };
            terminals = [::cq_gpu_sequence::translate_arena => batch];
            round = [::cq_gpu_datalog::bind_round(binding = 5)];
            print = ::cq_gpu_datalog::print_rounds;
        }
        struct ResidentIfLet;
        relation Edge(x: i32, y: i32) = (&edges);
        relation R(x: i32, y: i32);
        relation S(x: i32, y: i32);
        output R, S;
        R(*x, *y) :- Edge(x, y);
        S(*x, z) :- R(x, y), if let Some(z) = Some(*y);
    };
    assert_eq!(r.collect::<Vec<(i32, i32)>>(), edges);
    assert_eq!(s.collect::<Vec<(i32, i32)>>(), edges);
}

#[test]
fn legacy_observer_runs_once_alongside_a_grouped_head() {
    use gpu_datalog_runtime::{Result, capture::RowSource};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static OBSERVATIONS: AtomicUsize = AtomicUsize::new(0);
    fn observe(label: &str, _: usize, round: usize, reads: &[&str], sources: &[&dyn RowSource])
        -> Result<impl FnOnce(usize) -> Result<()> + use<>>
    {
        assert_eq!(label, "Observed@seed");
        assert_eq!(round, 0);
        assert_eq!(reads.len(), sources.len());
        assert_eq!(sources[0].len(), 1);
        Ok(|candidates| {
            assert_eq!(candidates, 1);
            OBSERVATIONS.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
    }

    let many: Vec<_> = (0..4096i32).map(|x| (x,)).collect();
    let one = vec![(4096i32,)];
    let (a, b) = cq_datalog::datalog! {
        pipeline {
            incremental = ::cq_datalog::passes::incremental(driver(delta_first););
            storage_rewrites = [::cq_gpu_datalog::device_storage];
            cq = pipeline {
                physical = [::cq_gpu_sequence::capture_constants];
                structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
                sequence = ::mini_linq::passes::sequence;
                stage = ::cq_gpu_sequence::stage;
                decompose = [::cq_gpu_sequence::decompose, ::cq_sequence_rewrites::output(reserve_previous)];
                grade = ::cq_gpu_sequence::machine_grade;
                print = ::cq_gpu_sequence::translate_arena;
            };
            terminals = [::cq_gpu_sequence::translate_arena => batch];
            round = [::cq_gpu_datalog::bind_round(binding = 5, head_cursor, legacy("Observed@seed"))];
            print = ::cq_gpu_datalog::print_rounds(observe("Observed@seed" => observe));
        }
        struct MixedHeads;
        relation Many(x:i32) = (&many);
        relation One(x:i32) = (&one);
        relation A(x:i32);
        relation B(x:i32);
        output A, B;
        A(*x) :- Many(x);
        Observed: B(*x) :- One(x);
        A(*x) :- B(x);
        B(*x) :- A(x);
    };
    assert_eq!(OBSERVATIONS.load(Ordering::Relaxed), 1);
    let expected: Vec<_> = (0..=4096i32).map(|x| (x,)).collect();
    for mut rows in [a.collect::<Vec<_>>(), b.collect::<Vec<_>>()] {
        rows.sort_unstable();
        assert_eq!(rows, expected);
    }
}

/// The `recursion_closure` workload, including its seed predicate (which
/// must not be moved into the recursive union). Compare complete sets on
/// its duplicate/cycle and empty-seed fixtures and on a signed continuation.
#[test]
fn recursion_closure_set_projection() {
    use std::collections::BTreeSet;
    for edges in [
        vec![(0i32,1i32), (1,2), (2,1), (2,3), (7,8), (0,1)],
        vec![(-2,-1)],
        vec![(2,-1), (-1,-2), (-2,3), (3,3)],
    ] {
        let mut expected: BTreeSet<_> = edges.iter().copied().filter(|(x,_)| *x >= 0).collect();
        loop {
            let before = expected.len();
            let extra: Vec<_> = expected.iter().flat_map(|&(x, y)| edges.iter()
                .filter_map(move |&(v,z)| (v == y).then_some((x,z)))).collect();
            expected.extend(extra);
            if expected.len() == before { break; }
        }
        let reached = cq_datalog::datalog! {
            pipeline {
                incremental = ::cq_datalog::passes::incremental(driver(delta_first););
                storage_rewrites = [::cq_gpu_datalog::device_storage];
                cq = pipeline {
                    physical = [::cq_gpu_sequence::capture_constants];
                    structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
                    sequence = ::mini_linq::passes::sequence;
                    stage = ::cq_gpu_sequence::stage;
                    decompose = [::cq_gpu_sequence::decompose];
                    grade = ::cq_gpu_sequence::machine_grade;
                    print = ::cq_gpu_sequence::translate_arena;
                };
                terminals = [::cq_gpu_sequence::translate_arena => batch];
                round = [::cq_gpu_datalog::bind_round(binding = 5, head_cursor)];
                print = ::cq_gpu_datalog::print_rounds;
            }
            struct RecursionClosure;
            relation Edge(x:i32,y:i32) = (&edges);
            relation Reach(x:i32,z:i32);
            output Reach;
            Reach(*x,*y) :- Edge(x,y), where x >= 0;
            Reach(*x,*z) :- Reach(x,y), Edge(y,z);
        };
        assert_eq!(reached.collect::<BTreeSet<_>>(), expected);
    }
}

/// GPU-ExpansionShare: one seed reaches four hubs, each with 50,000
/// successors. The first expansion has four children, so one block holds the
/// second expansion's 200,000 children (782 batches) while every other block
/// has no root tile; that expansion is published and stolen. The answer must
/// be the complete two-hop set.
#[test]
fn skewed_second_expansion_is_shared_without_changing_the_answer() {
    const HUBS: i32 = 4;
    const LEAVES: i32 = 50_000;
    let leaf = |hub: i32, index: i32| 1_000 + hub * 100_000 + index;
    let mut edges = Vec::new();
    for hub in 1..=HUBS {
        edges.push((0, hub));
        edges.extend((0..LEAVES).map(|index| (hub, leaf(hub, index))));
    }
    let seeds = vec![(0i32,)];
    let two_hop = cq_datalog::datalog! {
        pipeline {
            incremental = ::cq_datalog::passes::incremental(driver(delta_first););
            storage_rewrites = [::cq_gpu_datalog::device_storage];
            cq = pipeline {
                physical = [::cq_gpu_sequence::capture_constants];
                structures = [::cq_gpu_datalog::bind_inputs, ::cq_gpu_sequence::bind_runs];
                sequence = ::mini_linq::passes::sequence;
                stage = ::cq_gpu_sequence::stage;
                decompose = [::cq_gpu_sequence::decompose, ::cq_sequence_rewrites::roots(spread)];
                grade = ::cq_gpu_sequence::machine_grade;
                print = ::cq_gpu_sequence::translate_arena;
            };
            terminals = [::cq_gpu_sequence::translate_arena => batch];
            round = [::cq_gpu_datalog::bind_round(binding = 5)];
            print = ::cq_gpu_datalog::print_rounds;
        }
        struct SkewedTwoHop;
        relation Seed(x: i32) = (&seeds);
        relation Edge(x: i32, y: i32) = (&edges);
        relation TwoHop(x: i32, z: i32);
        output TwoHop;
        TwoHop(*x, *z) :- Seed(x), Edge(x, y), Edge(y, z);
    };
    let mut actual: Vec<(i32, i32)> = two_hop.collect();
    actual.sort_unstable();
    let mut expected: Vec<(i32, i32)> = (1..=HUBS)
        .flat_map(|hub| (0..LEAVES).map(move |index| (0, leaf(hub, index))))
        .collect();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

fn bounded_random_graph(nodes: i32, fanout: usize, mut state: u64) -> Data {
    let mut edge = Vec::with_capacity(nodes as usize * fanout);
    for source in 0..nodes {
        for _ in 0..fanout {
            // Use the shared fixtures' stable LCG, with bounded out-degree
            // rather than their dense Bernoulli graph at this larger size.
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            edge.push((source, ((state >> 32) % nodes as u64) as i32));
        }
    }
    edge.sort_unstable();
    edge.dedup();
    Data {
        nodes: (0..nodes).map(|node| (node,)).collect(),
        edge,
        ..Data::default()
    }
}

fn wall_time(evaluate: Evaluate, data: &Data) -> (Rows, Duration) {
    // Unlike the corpus' inner timer, include input cloning, GPU upload/readback,
    // and complete-answer canonicalization for each engine.
    let start = Instant::now();
    let (rows, _) = evaluate(data);
    (rows, start.elapsed())
}

#[test]
#[ignore = "requires CUDA; opt-in end-to-end GPU/mini/15-thread mini-par timing probe"]
fn timing_random_tc_and_same_generation() {
    // Defaults fit a roughly 4 GiB device; override with GPU_DATALOG_PROBE_NODES,
    // GPU_DATALOG_PROBE_FANOUT and GPU_DATALOG_PROBE_CASES (comma-separated).
    let env = |name: &str, default: usize| {
        std::env::var(name)
            .ok()
            .map_or(default, |value| value.parse().expect(name))
    };
    let nodes = env("GPU_DATALOG_PROBE_NODES", 256) as i32;
    let fanout = env("GPU_DATALOG_PROBE_FANOUT", 4);
    let cases = std::env::var("GPU_DATALOG_PROBE_CASES")
        .unwrap_or_else(|_| "tc_left,tc_right,tc_nonlinear,same_generation".into());
    const SEED: u64 = 0x5eed;
    const THREADS: usize = 15;

    // At the defaults: at most 65,536 answer pairs; each nonlinear delta/full
    // join emits at most 256^3 candidates (128 MiB of packed u64 rows), and each
    // same-generation round at most 256^2 * 4^2 candidates (8 MiB), before
    // admission, leaving room for layouts, sort/merge scratch and retries.
    let data = bounded_random_graph(nodes, fanout, SEED);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(THREADS)
        .build()
        .expect("create the 15-thread mini-par timing pool");
    // Exclude CUDA context creation and module loading from the measurements.
    let warm = bounded_random_graph(8, 2, SEED);
    for case in CASES.iter() {
        (case.gpu)(&warm);
    }

    eprintln!(
        "program\tnodes\tedges\tseed\trows\tmini_par_threads\tgpu_wall_ms\tmini_wall_ms\tmini_par_wall_ms"
    );
    for case in CASES
        .iter()
        .filter(|case| cases.split(',').any(|name| name == case.name))
    {
        let (expected, mini_elapsed) = wall_time(case.mini, &data);
        let (actual, gpu_elapsed) = wall_time(case.gpu, &data);
        assert_eq!(actual, expected, "{} / GPU timing result", case.name);
        drop(actual);

        let start = Instant::now();
        let parallel = pool.install(|| {
            assert_eq!(rayon::current_num_threads(), THREADS);
            (case.mini_par)(&data).0
        });
        let parallel_elapsed = start.elapsed();
        assert_eq!(parallel, expected, "{} / mini-par timing result", case.name);

        eprintln!(
            "{}\t{nodes}\t{}\t{SEED}\t{}\t{THREADS}\t{:.3}\t{:.3}\t{:.3}",
            case.name,
            data.edge.len(),
            expected.len(),
            gpu_elapsed.as_secs_f64() * 1_000.0,
            mini_elapsed.as_secs_f64() * 1_000.0,
            parallel_elapsed.as_secs_f64() * 1_000.0,
        );
    }
}
