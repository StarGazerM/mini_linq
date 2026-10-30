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
            decompose = [::cq_gpu_sequence::decompose];
            grade = ::cq_gpu_sequence::machine_grade;
            print = ::cq_gpu_sequence::translate_arena;
        };
        terminals = [::cq_gpu_sequence::translate_arena => batch];
        round = [::cq_gpu_datalog::bind_round(binding = 5)];
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
        std::env::var(name).ok().map_or(default, |value| value.parse().expect(name))
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
