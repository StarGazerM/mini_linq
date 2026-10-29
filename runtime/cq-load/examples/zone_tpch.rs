//! Reproduce the SF1 D85 block opportunity using an externally derived, exact
//! key file. Run with --release --features parquet; arguments: lineitem.parquet
//! and a newline-separated file of the 2,231 order keys. This checks the host,
//! not compiler integration or survivor-only Parquet I/O.
use cq_load::{Batch, Loader, MinMax, ParquetOptions, Relation, ZoneRows, column};
use cq_storage::{AdaptiveKeySet, BlockRead, I64Codec, Membership, PositionalRows, counters};
use std::time::Instant;

struct Lineitem;
impl Relation for Lineitem {
    type Row = (i64, i64, i64);
    const COLUMNS: &'static [&'static str] = &["l_orderkey", "l_partkey", "l_suppkey"];
    // DECODE V: this benchmark decoder materializes lineitem before membership selection; P5 requires encoded scan columns and late materialization.
    fn decode(batch: &Batch<'_>) -> cq_load::Result<Vec<Self::Row>> {
        Ok(batch
            // DECODE V: order keys are decoded before membership tests; P5 evaluates preserved predicates in the scan representation.
            .column::<i64>(0)?
            .into_iter()
            // DECODE V: part keys are materialized before row selection despite no query read; P5 must retain them encoded.
            .zip(batch.column::<i64>(1)?)
            // DECODE V: supplier keys are materialized before membership selection; P5 materializes only demanded survivor columns.
            .zip(batch.column::<i64>(2)?)
            .map(|((order, part), supplier)| (order, part, supplier))
            .collect())
    }
}
type Schema = (MinMax<i64>, MinMax<i64>, MinMax<i64>);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: zone_tpch lineitem.parquet keys.txt");
    let keys: Vec<i64> = std::fs::read_to_string(&args[2])?
        .lines()
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    assert_eq!(keys.len(), 2231);
    assert_eq!(
        keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
        2231
    );
    let set = AdaptiveKeySet::<i64, I64Codec, 67108864>::from_keys(keys, I64Codec);
    for (size, expected_visited) in [(2048, 1571), (122880, 49)] {
        let start = Instant::now();
        // DECODE V: the benchmark loads all logical tuples before zone pruning; P5 must replace eager row loading with columnar scan storage.
        let rows = Loader::<Lineitem>::parquet(&args[1], ParquetOptions { batch_size: 3000 })?;
        let load = start.elapsed();
        assert_eq!(rows.len(), 6_001_215);
        assert!(rows.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        // Independent scalar membership oracle retains exact positions/order.
        let scalar: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| cq_storage::ExactMembership::contains(&set, &row.0))
            .map(|(i, _)| i)
            .collect();
        let start = Instant::now();
        let host = ZoneRows::from_rows::<Schema>(rows, size.try_into()?);
        let build = start.elapsed();
        let predicate = Membership(&set);
        counters::reset();
        let selected: Vec<_> = (&host)
            .read(column::<0>(), &predicate)
            .map(|id| id.offset())
            .collect();
        let counts = counters::snapshot();
        assert_eq!(selected, scalar);
        assert_eq!(counts.blocks_considered, host.len().div_ceil(size) as u64);
        assert_eq!(counts.blocks_visited, expected_visited);
        assert_eq!(
            counts.blocks_skipped,
            counts.blocks_considered - expected_visited
        );
        println!(
            "block_rows={size} rows={} considered={} visited={} skipped={} rows_tested={} matched={} load_ms={:.3} metadata_build_ms={:.3} metadata_bytes={} keyset_bytes={}",
            host.len(),
            counts.blocks_considered,
            counts.blocks_visited,
            counts.blocks_skipped,
            counts.rows_tested,
            selected.len(),
            load.as_secs_f64() * 1000.,
            build.as_secs_f64() * 1000.,
            host.metadata_bytes(),
            set.retained_bytes()
        );
    }
    Ok(())
}
