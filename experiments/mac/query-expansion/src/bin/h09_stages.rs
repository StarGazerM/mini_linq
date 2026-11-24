//! Resident H09 stage profile: the fused GPU graph against the same chain as
//! stepwise synchronous primitives (join, three semijoins, full-width distinct),
//! then the first and second fused query after 0.5 s idle, 2 s idle, 2 s CPU
//! spin and 2 s of tiny GPU work every 20 ms, with CPU minor-fault deltas.
//! Usage: `h09_stages <retail_h09 data dir> [warm runs, default 11]`.
use cq_mac::{GpuRowsInput, GpuSemiJoin, IndexSpec, MacContext, Payload};
use mac_query_expansion::{
    compiled_h09::{self, Construction},
    h09,
    manual_h09::{Prepared, green_parts},
};
use std::{collections::HashSet, error::Error, time::Instant};

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn main() -> Result<(), Box<dyn Error>> {
    let dir = std::env::args().nth(1).ok_or("data dir")?;
    let runs: usize = std::env::args().nth(2).map_or(Ok(11), |r| r.parse())?;
    let data = h09::load(std::path::Path::new(&dir), "default")?;
    let context = MacContext::new()?;
    let source = Prepared::new(&context, &data)?;
    let index = compiled_h09::build(&context, &source, Construction::Gpu)?;
    let lineitem = index.lineitem().ok_or("no demand")?;
    let probes: Vec<i64> = green_parts(&data).collect();

    let partsupp = context.relation_i64(data.partsupp.len(), |i| {
        let (p, s) = data.partsupp[i];
        [p, s]
    })?;
    let orders = context.relation_i64(data.orders.len(), |i| [data.orders[i].0])?;
    let supplier = context.relation_i64(data.supplier.len(), |i| [data.supplier[i].0])?;
    let nations: HashSet<i32> = data.nation.iter().map(|&(n,)| n).collect();
    let selection = context.relation_u32(data.supplier.len(), |i| {
        [u32::from(nations.contains(&data.supplier[i].1))]
    })?;
    let rows = IndexSpec {
        key_column: 0,
        payload: Payload::RowId,
    };
    let ps = context.build_radix(
        &partsupp,
        IndexSpec {
            key_column: 0,
            payload: Payload::Column(1),
        },
        None,
    )?;
    let or = context.build_radix(&orders, rows, None)?;
    let su = context.build_radix(&supplier, rows, Some(&selection))?;
    let steps = [
        GpuSemiJoin {
            key_column: 1,
            item_column: Some(2),
            run: &ps,
        },
        GpuSemiJoin {
            key_column: 0,
            item_column: None,
            run: &or,
        },
        GpuSemiJoin {
            key_column: 2,
            item_column: None,
            run: &su,
        },
    ];

    println!(
        "fused_ms\tcollect_ms\tdrop_ms\t|\tjoin_ms\tsemi_ps_ms\tsemi_or_ms\tsemi_su_ms\tdistinct_ms\tanswers"
    );
    for round in 0..runs + 2 {
        let t = Instant::now();
        let fused = lineitem.join_semijoin_distinct(&context, &probes, &steps, [0, 1, 2])?;
        let fused_ms = ms(t);
        let t = Instant::now();
        let answers: Vec<(i64, i64, i64)> =
            fused.tuples().iter().map(|&[a, b, c]| (a, b, c)).collect();
        let collect_ms = ms(t);
        let t = Instant::now();
        drop(fused);
        let drop_ms = ms(t);

        let t = Instant::now();
        let join = lineitem.join_row_ids(&context, &probes)?;
        let join_ms = ms(t);
        let t = Instant::now();
        let a = lineitem.semijoin(&context, GpuRowsInput::Join(&join), 1, Some(2), &ps)?;
        let a_ms = ms(t);
        let t = Instant::now();
        let b = lineitem.semijoin(&context, GpuRowsInput::Rows(&a), 0, None, &or)?;
        let b_ms = ms(t);
        let t = Instant::now();
        let c = lineitem.semijoin(&context, GpuRowsInput::Rows(&b), 2, None, &su)?;
        let c_ms = ms(t);
        let t = Instant::now();
        let d = lineitem.distinct(&context, GpuRowsInput::Rows(&c), [0, 1, 2])?;
        let d_ms = ms(t);
        assert_eq!(d.len(), answers.len());
        if round >= 2 {
            println!(
                "{fused_ms:.2}\t{collect_ms:.2}\t{drop_ms:.2}\t|\t{join_ms:.2}\t{a_ms:.2}\t{b_ms:.2}\t{c_ms:.2}\t{d_ms:.2}\t{}",
                answers.len()
            );
        }
    }
    for (label, pause) in [
        ("idle_500ms", 0),
        ("idle_2s", 1),
        ("cpu_spin_2s", 2),
        ("gpu_tick_2s", 3),
    ] {
        let mut samples = Vec::new();
        for _ in 0..7 {
            match pause {
                0 => std::thread::sleep(std::time::Duration::from_millis(500)),
                1 => std::thread::sleep(std::time::Duration::from_secs(2)),
                3 => {
                    let t = Instant::now();
                    while t.elapsed().as_secs() < 2 {
                        std::hint::black_box(lineitem.join_row_ids(&context, &probes[..64])?.len());
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
                _ => {
                    let t = Instant::now();
                    let mut x = 0u64;
                    while t.elapsed().as_secs() < 2 {
                        x = std::hint::black_box(
                            x.wrapping_mul(6364136223846793005).wrapping_add(1),
                        );
                    }
                }
            }
            let mut pair = [0.0; 2];
            let mut faults = [0i64; 2];
            for k in 0..2 {
                let f = minflt();
                let t = Instant::now();
                let fused =
                    lineitem.join_semijoin_distinct(&context, &probes, &steps, [0, 1, 2])?;
                std::hint::black_box(fused.tuples().len());
                pair[k] = (ms(t) * 100.0).round() / 100.0;
                faults[k] = minflt() - f;
            }
            samples.push((pair, faults));
        }
        println!("{label}: [first, second] ms / minor faults");
        for s in samples {
            println!("  {:?} {:?}", s.0, s.1);
        }
    }
    Ok(())
}

fn minflt() -> i64 {
    #[repr(C)]
    struct Rusage([i64; 18]);
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut usage = Rusage([0; 18]);
    unsafe { getrusage(0, &mut usage) };
    // ru_utime, ru_stime (2 × timeval = 4 words), then maxrss, ixrss, idrss, isrss, minflt.
    usage.0[8]
}
