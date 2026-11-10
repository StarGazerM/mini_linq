//! Reproduce the SF1 D85 block opportunity using an externally derived, exact
//! key file. Run with --release --features parquet; arguments: lineitem.parquet
//! and a newline-separated file of the 2,231 order keys. This checks the host,
//! not compiler integration or survivor-only Parquet I/O.
use cq_load::{Batch, ColumnarLoader, ColumnarRelation, Dictionaries, ParquetOptions};
use cq_storage::OrderKey;
use cq_storage::scan::{
    BlockRows, Bytes, Predicate, ScanColumn, ScanColumnBuilder, ScanTable, VECTOR_ROWS,
};
use std::time::Instant;

struct Lineitem;
impl ColumnarRelation for Lineitem {
    const COLUMNS: &'static [&'static str] = &["l_orderkey", "l_partkey", "l_suppkey"];
    type Table = ScanTable<(
        ScanColumn<i64, Bytes>,
        ScanColumn<i64, Bytes>,
        ScanColumn<i64, Bytes>,
    )>;
    type Builders = (
        ScanColumnBuilder<i64, Bytes>,
        ScanColumnBuilder<i64, Bytes>,
        ScanColumnBuilder<i64, Bytes>,
    );
    fn builders(rows: BlockRows, summaries: bool) -> Self::Builders {
        (
            ScanColumnBuilder::new(rows, summaries),
            ScanColumnBuilder::new(rows, summaries),
            ScanColumnBuilder::new(rows, summaries),
        )
    }
    fn append(batch: &Batch<'_>, builders: &mut Self::Builders) -> cq_load::Result<()> {
        batch.append_column(0, &mut builders.0)?;
        batch.append_column(1, &mut builders.1)?;
        batch.append_column(2, &mut builders.2)
    }
    fn finish(builders: Self::Builders, _: &mut Dictionaries) -> Self::Table {
        ScanTable::new((
            builders.0.finish(),
            builders.1.finish(),
            builders.2.finish(),
        ))
    }
}

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
    let predicate = Predicate::Membership(keys.iter().map(OrderKey::ordinal).collect());
    let oracle_keys: std::collections::HashSet<_> = keys.iter().copied().collect();
    for (size, expected_visited) in [(2048, 1571), (122880, 49)] {
        let start = Instant::now();
        let host = ColumnarLoader::<Lineitem>::parquet(
            &args[1],
            ParquetOptions { batch_size: 3000 },
            BlockRows::new(size).unwrap(),
            true,
        )?;
        let load = start.elapsed();
        assert_eq!(host.len(), 6_001_215);
        let column = &host.columns().0;
        // Independent diagnostic oracle decodes only the predicate column,
        // after loading; the actual membership scan below retains residuals.
        let mut scalar = Vec::new();
        let mut buffer = [0_i64; VECTOR_ROWS];
        let mut previous = i64::MIN;
        let mut offset = 0;
        for (block, header) in column.headers().iter().enumerate() {
            for vector in 0..(header.rows as usize).div_ceil(VECTOR_ROWS) {
                let n = column.decode_vector(block, vector, &mut buffer);
                for &key in &buffer[..n] {
                    assert!(previous <= key);
                    previous = key;
                    if oracle_keys.contains(&key) {
                        scalar.push(offset);
                    }
                    offset += 1;
                }
            }
        }
        let considered = column.headers().len();
        let visited = column
            .summaries()
            .unwrap()
            .iter()
            .filter(|summary| {
                keys.iter()
                    .any(|key| summary.lo <= key.ordinal() && key.ordinal() <= summary.hi)
            })
            .count();
        let mut selected = Vec::new();
        column.filter(&predicate, |row| selected.push(row));
        assert_eq!(selected, scalar);
        assert_eq!(visited, expected_visited);
        println!(
            "block_rows={size} rows={} considered={considered} visited={visited} skipped={} matched={} load_ms={:.3} predicate_payload_bytes={}",
            host.len(),
            considered - visited,
            selected.len(),
            load.as_secs_f64() * 1000.,
            column.payload().len(),
        );
    }
    Ok(())
}
