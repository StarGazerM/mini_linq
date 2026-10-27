//! Actual shared-memory GPU construction followed by ordinary CPU index reads.
use cq_mac::{IndexSpec, MacContext, MaintainedIndex, MaintenancePolicy, Payload};
use cq_storage::{Build, HashIndex, IndexRead, RadixRun};
use std::{hint::black_box, time::Instant};

fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let value = f();
    (value, start.elapsed().as_secs_f64() * 1000.0)
}

fn numeric_row(i: usize) -> [u32; 2] {
    let key = if i.is_multiple_of(19) {
        u32::MAX
    } else {
        (i as u32).wrapping_mul(7919) % 4093
    };
    [key, (i as u32).wrapping_mul(104729)]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let n = std::env::args()
        .nth(1)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(65_539usize);
    let (context, setup_ms) = timed(MacContext::new);
    let context = context?;
    let spec = IndexSpec {
        key_column: 0,
        payload: Payload::Column(1),
    };
    let (source, ingestion_ms) = timed(|| context.relation_u32(n, numeric_row));
    let source = source?;
    // CPU correctness references are outside ingestion and construction timing.
    let rows = source
        .u32_values()?
        .chunks_exact(2)
        .map(|r| (r[0], r[1]))
        .collect::<Vec<_>>();
    let (run, build_ms) = timed(|| context.build(&source, spec, None));
    let run = run?;
    let (radix, radix_ms) = timed(|| RadixRun::from_pairs(rows.iter().copied()));
    // HashIndex is lazy; force materialization inside its measured region.
    let (hash, hash_ms) = timed(|| {
        let index = HashIndex::build(rows.iter().copied());
        black_box(index.lookup(&0).count());
        index
    });
    let queries = (0..8192).chain([u32::MAX]).collect::<Vec<_>>();
    for &key in &queries {
        let values = run.lookup(&key).collect::<Vec<_>>();
        assert_eq!(values, (&radix).lookup(&key).collect::<Vec<_>>());
        assert_eq!(values, hash.lookup(&key).collect::<Vec<_>>());
    }
    let (hits, membership_ms) = timed(|| {
        queries
            .iter()
            .filter(|key| black_box(run.contains(key)))
            .count()
    });
    let (checksum, expansion_ms) = timed(|| {
        (&run)
            .probe::<1, _, _>(queries.iter().copied(), |key| *key)
            .fold(0u64, |sum, (key, value)| {
                sum.wrapping_add(black_box(key as u64 + value as u64))
            })
    });
    println!(
        "rows={n} source_bytes={} setup_ms={setup_ms:.3} ingestion_ms={ingestion_ms:.3} metal_build_ms={build_ms:.3} native_radix_build_ms={radix_ms:.3} native_hash_build_ms={hash_ms:.3}",
        n * 2 * size_of::<u32>()
    );
    println!(
        "queries={} membership_ms={membership_ms:.3} hits={hits} expansion_ms={expansion_ms:.3} checksum={checksum}",
        queries.len()
    );
    // Each incoming batch is ingested once, then reused by both policies.
    let (batches, batch_ingestion_ms) = timed(|| {
        (2..=7)
            .map(|epoch| context.relation_u32(n.saturating_sub(epoch * 13), numeric_row))
            .collect::<Result<Vec<_>, _>>()
    });
    let batches = batches?;
    println!("append_batch_ingestion_ms={batch_ingestion_ms:.3}");
    for policy in [MaintenancePolicy::Flat, MaintenancePolicy::BinaryCarry] {
        let mut owner = MaintainedIndex::new(policy);
        let mut prefix = Vec::new();
        let pinned;
        let mut append_ms = 0.0;
        let mut query_ms = 0.0;
        let first = owner.append(&context, &source, spec)?;
        prefix.extend_from_slice(&rows);
        pinned = first.clone();
        let mut old_cursor = pinned.lookup(&0);
        let first_value = old_cursor.next();
        for batch_source in &batches {
            // Unequal batches force carries to depend on chronology, not size.
            let batch = &rows[..batch_source.len()];
            let (snapshot, elapsed) = timed(|| owner.append(&context, batch_source, spec));
            let snapshot = snapshot?;
            append_ms += elapsed;
            prefix.extend_from_slice(batch);
            let reference = RadixRun::from_pairs(prefix.iter().copied());
            for &key in &queries {
                assert_eq!(
                    snapshot.lookup(&key).collect::<Vec<_>>(),
                    (&reference).lookup(&key).collect::<Vec<_>>()
                );
            }
            let (_, elapsed) = timed(|| {
                black_box(
                    (&snapshot)
                        .probe::<1, _, _>(queries.iter().copied(), |key| *key)
                        .fold(0u64, |sum, (_, value)| sum.wrapping_add(value as u64)),
                )
            });
            query_ms += elapsed;
            println!(
                "policy={policy:?} epoch={} rows={} runs={}",
                snapshot.epoch(),
                snapshot.len(),
                snapshot.run_count()
            );
        }
        drop(owner);
        let want = rows
            .iter()
            .filter(|(key, _)| *key == 0)
            .map(|(_, value)| *value)
            .collect::<Vec<_>>();
        assert_eq!(
            first_value
                .into_iter()
                .chain(old_cursor)
                .collect::<Vec<_>>(),
            want
        );
        println!(
            "policy={policy:?} appends_2_to_7_ms={append_ms:.3} queries_2_to_7_ms={query_ms:.3} retained_cursor=exact"
        );
    }
    println!(
        "Exact native comparisons and online snapshot checks passed; construction timings include allocation/encoding/wait, exclude separately reported ingestion and context setup. Single-run smoke, not crossover evidence."
    );
    Ok(())
}
