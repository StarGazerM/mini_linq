use datalog_oracle::{CASES, Case, Data, generate, goldens, random_data};
use std::hint::black_box;

fn main() -> datalog_oracle::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["golden"] => { goldens(false, false)?; }
        ["golden", "--write"] => { goldens(true, false)?; }
        ["bench"] => bench(),
        ["counters"] => counters(),
        _ => return Err("usage: datalog-oracle golden [--write] | counters | bench".into()),
    }
    Ok(())
}

fn counters() {
    println!("program\tnodes\trows\thashes\tbuilds\tlookups\tcursor_steps\trows_enumerated\tdistinct_insertions\tbytes_peak");
    for n in [2000, 4000, 8000] {
        let data = generate("chain", n, 0);
        let expected = datalog_oracle::programs::reach::oracle(&data).0;
        cq_storage::counters::reset();
        let actual = datalog_oracle::programs::reach::mini(&data).0;
        let cost = cq_storage::counters::snapshot();
        assert_eq!(actual, expected, "reach chain n={n}");
        println!("reach\t{n}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", actual.len(),
            cost.hash_computations, cost.index_builds, cost.lookups,
            cost.cursor_steps, cost.rows_enumerated, cost.distinct_insertions,
            cost.bytes_retained_peak);
    }
}
fn bench() {
    assert!(!cfg!(debug_assertions), "bench requires --release");
    println!("program\tscenario\tnodes\tedges\trows\tmini_us\tascent_us");
    for name in ["reach", "tc_left", "tc_nonlinear", "same_generation"] {
        let c = CASES.iter().find(|c| c.name == name).unwrap();
        let sizes = if name == "tc_nonlinear" { [128, 256, 512] } else { [256, 1024, 4096] };
        for n in sizes {
            measure(c, "chain", n, &generate("chain", n, 0));
        }
    }
    let reach = CASES.iter().find(|c| c.name == "reach").unwrap();
    for n in [1000, 5000] {
        let mut data = random_data(n, 0x5eed, 2, n as u64);
        // A fixed seed set avoids benchmarking a single isolated source.
        data.seed = (0..16).map(|x| (x,)).collect();
        measure(reach, "random_degree2_seed16", n, &data);
    }
}

fn measure(c: &Case, scenario: &str, n: i32, d: &Data) {
    let expected = (c.oracle)(d).0;
    assert_eq!((c.mini)(d).0, expected);
    let rows = expected.len();
    drop(expected);
    // Each evaluator clones/loads inputs before starting its timer,
    // times fixed-point evaluation, then canonicalizes after stopping.
    let mut mini = Vec::new(); let mut ascent = Vec::new();
    for _ in 0..5 {
        mini.push(black_box((c.mini)(black_box(d))).1);
        ascent.push(black_box((c.oracle)(black_box(d))).1);
    }
    mini.sort(); ascent.sort();
    println!("{}\t{scenario}\t{n}\t{}\t{rows}\t{:.3}\t{:.3}", c.name, d.edge.len(),
        mini[2].as_secs_f64()*1e6, ascent[2].as_secs_f64()*1e6);
}
