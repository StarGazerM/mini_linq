use datalog_parallel_oracle::{CASES, Case, Data, generate, goldens, random_data};
use std::hint::black_box;

fn main() -> datalog_oracle::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["golden"] => {
            goldens()?;
        }
        ["bench"] => bench(&[1, 2, 4, 5, 8, 15])?,
        ["bench", "--threads", n] => {
            let n: usize = n.parse()?;
            if n == 0 {
                return Err("--threads must be positive".into());
            }
            bench(&[n])?;
        }
        ["counters"] => {
            #[cfg(feature = "counters")]
            counters();
            #[cfg(not(feature = "counters"))]
            return Err("counters requires --features counters; run cargo run --release -p datalog-parallel-oracle --features counters -- counters".into());
        }
        _ => {
            return Err(
                "usage: datalog-parallel-oracle golden | counters | bench [--threads N]".into(),
            );
        }
    }
    Ok(())
}

#[cfg(feature = "counters")]
fn counters() {
    println!(
        "program\tnodes\trows\thashes\tbuilds\tlookups\tcursor_steps\trows_enumerated\tdistinct_insertions\tbytes_peak"
    );
    for n in [2000, 4000, 8000] {
        let data = generate("chain", n, 0);
        let expected = datalog_oracle::programs::reach::oracle(&data).0;
        cq_storage::counters::reset();
        let actual = datalog_oracle::programs::reach::mini(&data).0;
        let cost = cq_storage::counters::snapshot();
        assert_eq!(actual, expected, "reach chain n={n}");
        assert_eq!(cost.index_builds, 0, "reach chain n={n}");
        // Raw EDB canonicalization visits n rows once; its borrowed adapter
        // adds no core cursor events. Grouped seed/delta/edge reads add 2*n+1.
        assert_eq!(cost.rows_enumerated, 3 * n as u64 + 1, "reach chain n={n}");
        println!(
            "reach\t{n}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            actual.len(),
            cost.hash_computations,
            cost.index_builds,
            cost.lookups,
            cost.cursor_steps,
            cost.rows_enumerated,
            cost.distinct_insertions,
            cost.bytes_retained_peak
        );
    }
}
fn bench(threads: &[usize]) -> datalog_oracle::Result<()> {
    if cq_storage::profile_select!(plain { false } counters { true }) {
        return Err("bench requires counters disabled; run cargo run --release -p datalog-parallel-oracle -- bench without enabling counters in any selected package".into());
    }
    if cfg!(debug_assertions) {
        return Err("bench requires --release".into());
    }
    let pools = threads
        .iter()
        .map(|&n| rayon::ThreadPoolBuilder::new().num_threads(n).build())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    println!(
        "program\tscenario\tnodes\tedges\trows\tthreads\tmini_us\tascent_us\tmini_par_us\tascent_par_us\tmini_speedup"
    );
    for name in ["reach", "tc_left", "tc_nonlinear", "same_generation"] {
        let c = CASES.iter().find(|c| c.name == name).unwrap();
        let sizes = if name == "tc_nonlinear" {
            [128, 256, 512]
        } else {
            [256, 1024, 4096]
        };
        for n in sizes {
            measure(c, "chain", n, &generate("chain", n, 0), &pools);
        }
    }
    let reach = CASES.iter().find(|c| c.name == "reach").unwrap();
    for n in [1000, 5000] {
        let mut data = random_data(n, 0x5eed, 2, n as u64);
        // A fixed seed set avoids benchmarking a single isolated source.
        data.seed = (0..16).map(|x| (x,)).collect();
        measure(reach, "random_degree2_seed16", n, &data, &pools);
    }
    Ok(())
}

fn measure(c: &Case, scenario: &str, n: i32, d: &Data, pools: &[rayon::ThreadPool]) {
    let expected = (c.oracle)(d).0;
    assert_eq!((c.mini)(d).0, expected);
    // Input cloning precedes each evaluator's timer; canonicalization follows it.
    let median = |evaluate: fn(&Data) -> (datalog_oracle::Rows, std::time::Duration)| {
        let mut times = Vec::with_capacity(5);
        for _ in 0..5 {
            let (actual, elapsed) = black_box(evaluate(black_box(d)));
            assert_eq!(actual, expected, "{} {scenario} n={n}", c.name);
            times.push(elapsed);
        }
        times.sort_unstable();
        times[2].as_secs_f64() * 1e6
    };
    let mini = median(c.mini);
    let ascent = median(c.oracle);
    for pool in pools {
        pool.install(|| {
            let mini_par = median(c.mini_parallel);
            let ascent_par = median(c.oracle_parallel);
            println!(
                "{}\t{scenario}\t{n}\t{}\t{}\t{}\t{mini:.3}\t{ascent:.3}\t{mini_par:.3}\t{ascent_par:.3}\t{:.3}",
                c.name, d.edge.len(), expected.len(), rayon::current_num_threads(),
                mini / mini_par
            );
        });
    }
}
