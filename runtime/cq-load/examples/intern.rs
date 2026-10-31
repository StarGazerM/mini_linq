//! Load a Parquet string column once, then compare the former intern pass,
//! borrowed one-pass interning, and a contiguous byte scan, interleaved five times.
//! Run in release with `--features parquet --example intern -- FILE COLUMN`.
//! Append `--profile` to isolate the new pass (and teardown) for a CPU profiler.
use std::hint::black_box;
use std::time::Instant;

use arrow_array::{Array, LargeStringArray, StringArray, StringViewArray};
use cq_load::{StringCode, StringDictionary};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn old(values: &[&str]) -> (Vec<String>, Vec<StringCode>) {
    let mut distinct = values.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    let dictionary: Vec<String> = distinct.into_iter().map(str::to_owned).collect();
    let codes = values
        .iter()
        .map(|value| {
            StringCode(
                dictionary
                    .binary_search_by(|candidate| candidate.as_str().cmp(value))
                    .unwrap() as u32,
            )
        })
        .collect();
    (dictionary, codes)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let profile = args.len() == 4 && args[3] == "--profile";
    if args.len() != 3 && !profile {
        return Err("usage: intern FILE.parquet COLUMN [--profile]".into());
    }
    let batches = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&args[1])?)?
        .with_batch_size(65536)
        .build()?
        .collect::<Result<Vec<_>, _>>()?;
    let mut values = Vec::new();
    for batch in &batches {
        let column = batch.column_by_name(&args[2]).ok_or("missing column")?;
        if column.null_count() != 0 {
            return Err("null string column".into());
        }
        macro_rules! strings {
            ($ty:ty) => {
                if let Some(array) = column.as_any().downcast_ref::<$ty>() {
                    values.extend(array.iter().map(Option::unwrap));
                    continue;
                }
            };
        }
        strings!(StringArray);
        strings!(LargeStringArray);
        strings!(StringViewArray);
        return Err("expected a string column".into());
    }
    let bytes: Vec<u8> = values.iter().flat_map(|value| value.bytes()).collect();
    println!("iteration\tmethod\trows\tbytes\tms\tMB_per_s");
    let report = |iteration, method, elapsed: std::time::Duration| {
        println!(
            "{iteration}\t{method}\t{}\t{}\t{:.6}\t{:.3}",
            values.len(),
            bytes.len(),
            elapsed.as_secs_f64() * 1000.0,
            bytes.len() as f64 / elapsed.as_secs_f64() / 1e6,
        );
    };
    for iteration in 0..5 {
        if profile {
            let start = Instant::now();
            let (dictionary, codes) =
                black_box(StringDictionary::encode(black_box(values.iter().copied())));
            report(iteration, "new", start.elapsed());
            if iteration == 0 {
                eprintln!(
                    "distinct entries: {}",
                    codes
                        .iter()
                        .map(|code| u64::from(code.0) + 1)
                        .max()
                        .unwrap_or(0)
                );
            }
            black_box((dictionary, codes));
            continue;
        }
        let mut old_result = None;
        let mut new_result = None;
        for method in if iteration % 2 == 0 { [0, 1] } else { [1, 0] } {
            let start = Instant::now();
            if method == 0 {
                let result = black_box(old(black_box(&values)));
                report(iteration, "old", start.elapsed());
                old_result = Some(result);
            } else {
                let result = black_box(StringDictionary::encode(black_box(values.iter().copied())));
                report(iteration, "new", start.elapsed());
                new_result = Some(result);
            }
        }
        let start = Instant::now();
        let sum = black_box(&bytes)
            .iter()
            .fold(0u64, |sum, &byte| sum + u64::from(byte));
        black_box(sum);
        report(iteration, "byte_scan", start.elapsed());
        let (old_dictionary, old_codes) = old_result.unwrap();
        let (new_dictionary, new_codes) = new_result.unwrap();
        assert_eq!(old_codes, new_codes);
        for (code, value) in old_dictionary.iter().enumerate() {
            assert_eq!(new_dictionary.lookup(value), Some(code as u32));
        }
    }
    Ok(())
}
