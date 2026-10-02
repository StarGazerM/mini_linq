//! Ungated file loading measurement; generation is outside the timed region.
//! Both formats produce the same owned tuple vector, with 2048-row batches.
use std::{hint::black_box, path::Path, time::Instant};

mini_linq::relation_loaders! {
    relation Lineitem(orderkey: i64, linenumber: i32, price: cq_load::Decimal,
                      shipdate: cq_load::Date, comment: String);
}

fn measure<T>(label: &str, path: &Path, mut read: impl FnMut() -> cq_load::Result<Vec<T>>) {
    let mut samples = Vec::new();
    let mut count = 0;
    for _ in 0..5 {
        let start = Instant::now();
        let rows = black_box(read().expect("load benchmark file"));
        let elapsed = start.elapsed().as_secs_f64();
        count = rows.len();
        samples.push(elapsed);
        black_box(rows);
    }
    samples.sort_by(f64::total_cmp);
    let seconds = samples[2];
    let bytes = std::fs::metadata(path).unwrap().len();
    println!(
        "{label}: rows={count}, bytes={bytes}, median_s={seconds:.6}, rows_per_s={:.0}, file_MB_per_s={:.2}, samples_s={samples:?}",
        count as f64 / seconds,
        bytes as f64 / seconds / 1_000_000.0
    );
}

fn main() {
    let directory = std::env::args()
        .nth(1)
        .expect("directory containing lineitem.csv and lineitem.parquet");
    let directory = Path::new(&directory);
    let csv = directory.join("lineitem.csv");
    let parquet = directory.join("lineitem.parquet");
    let csv_options = cq_load::CsvOptions::default();
    let parquet_options = cq_load::ParquetOptions::default();
    // Check format equality once, outside timing; this also warms both files.
    let expected = LineitemLoader::csv(&csv, csv_options).unwrap();
    assert_eq!(expected.len(), 1_000_000);
    assert_eq!(
        expected,
        LineitemLoader::parquet(&parquet, parquet_options).unwrap()
    );
    drop(expected);
    measure("CSV", &csv, || LineitemLoader::csv(&csv, csv_options));
    measure("Parquet", &parquet, || {
        LineitemLoader::parquet(&parquet, parquet_options)
    });
}
