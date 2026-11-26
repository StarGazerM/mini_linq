//! Matched CPU read comparison of the two GPU-built grouped u32-key runs:
//! (a) the error-bounded spline run (`Gpu::build_radix_index`) read by the
//!     native `cq_storage::RadixRunView`, constructed exactly as cq-mac's
//!     `MacRadixRun::view` does (same arrays, same read state);
//! (b) the word run (`Gpu::build_word_run`, CUDA `DeviceRadixRun` layout)
//!     with an exact radix-prefix directory, read by `word_read`.
//! Same keys, rows and probes for every reader; every probe's range and
//! row-id expansion is checked equal across readers before timing.
//! Run with `--ignored --nocapture`.

use std::{collections::HashSet, hint::black_box, time::Instant};

use cq_storage::{RadixRunReadState, RadixRunView, RadixSplineMetadata, RadixSplinePoint};
use mccl::{
    Gpu, Result,
    radix::RadixIndex,
    relation::{IndexSpec, Payload},
    word_read::WordRunView,
    words::{OffsetWidth, WordRunParams},
};

const DISTINCT: usize = 262_144;
const PROBES: usize = 65_536;

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Rows (one key per source row, shuffled) and probes, ~50% hits; misses
/// are near members where possible.
struct Data {
    name: &'static str,
    rows: Vec<u32>,
    probes: Vec<u32>,
}

fn random_distinct(state: &mut u64) -> Vec<u32> {
    let mut seen = HashSet::with_capacity(DISTINCT);
    let mut keys = Vec::with_capacity(DISTINCT);
    while keys.len() < DISTINCT {
        let key = xorshift(state) as u32;
        if seen.insert(key) {
            keys.push(key);
        }
    }
    keys
}

fn data(name: &'static str, distinct: Vec<u32>, copies: impl Fn(u32) -> usize, seed: u64) -> Data {
    let mut state = seed;
    let members: HashSet<u32> = distinct.iter().copied().collect();
    assert_eq!(members.len(), DISTINCT);
    let mut rows: Vec<u32> = distinct
        .iter()
        .flat_map(|&key| std::iter::repeat_n(key, copies(key)))
        .collect();
    for index in (1..rows.len()).rev() {
        rows.swap(index, xorshift(&mut state) as usize % (index + 1));
    }
    let probes = (0..PROBES)
        .map(|_| {
            let key = distinct[xorshift(&mut state) as usize % DISTINCT];
            if xorshift(&mut state) & 1 == 0 {
                return key;
            }
            for miss in [key.wrapping_add(1), key.wrapping_sub(1)] {
                if !members.contains(&miss) {
                    return miss;
                }
            }
            loop {
                let miss = xorshift(&mut state) as u32;
                if !members.contains(&miss) {
                    return miss;
                }
            }
        })
        .collect();
    Data { name, rows, probes }
}

fn datasets() -> Vec<Data> {
    let mut state = 0x5eed_d1c7;
    let sparse = random_distinct(&mut state);
    let heavy = random_distinct(&mut state);
    vec![
        // Every other u32 from zero: uniform, fully predictable spacing.
        data(
            "dense",
            (0..DISTINCT as u32).map(|i| 2 * i).collect(),
            |_| 1,
            1,
        ),
        data("sparse", sparse, |_| 1, 2),
        // Cubic: consecutive at the bottom, gaps up to ~2^14 at the top, so
        // most keys crowd into the lowest uniform-prefix buckets.
        data(
            "skewed",
            (0..DISTINCT as u64)
                .map(|i| ((i * i * i >> 23) + i) as u32)
                .collect(),
            |_| 1,
            3,
        ),
        // One key in 16 holds 32 rows (~2.9 rows per key, ~66% of rows hot).
        data(
            "dup-heavy",
            heavy,
            |key| {
                if key.wrapping_mul(0x9e37_79b9) >> 28 == 0 {
                    32
                } else {
                    1
                }
            },
            4,
        ),
    ]
}

fn spline_view<'a>(
    index: &'a RadixIndex,
    state: &'a RadixRunReadState,
) -> RadixRunView<'a, u32, u32> {
    // As cq-mac `MacRadixRun::view`: MCCL publishes only initialized
    // immutable prefixes; the u64 key/rank ABI equals the native points.
    unsafe {
        let points = std::slice::from_raw_parts(
            index.points().as_ptr().cast::<RadixSplinePoint>(),
            index.points().len(),
        );
        RadixRunView::from_parts(
            index.keys(),
            index.offsets(),
            index.items(),
            points,
            index.radix(),
            RadixSplineMetadata {
                min_key: index.min_key(),
                max_key: index.max_key(),
                len: index.keys().len(),
                shift: index.shift(),
                search_error: index.search_error(),
            },
            state,
        )
    }
}

/// One reader's three probe loops: membership, group range, and range plus
/// expansion of every row id. Each returns a checksum.
trait Reader {
    fn contains(&self, probes: &[u32]) -> u64;
    fn ranges(&self, probes: &[u32]) -> u64;
    fn expand(&self, probes: &[u32]) -> u64;
}

impl Reader for RadixRunView<'_, u32, u32> {
    fn contains(&self, probes: &[u32]) -> u64 {
        probes
            .iter()
            .map(|key| RadixRunView::contains(self, key) as u64)
            .sum()
    }

    fn ranges(&self, probes: &[u32]) -> u64 {
        probes
            .iter()
            .map(|key| {
                let (begin, end) = self.range(key);
                (begin ^ end) as u64
            })
            .sum()
    }

    fn expand(&self, probes: &[u32]) -> u64 {
        let items = self.items();
        probes
            .iter()
            .map(|key| {
                let (begin, end) = self.range(key);
                items[begin..end]
                    .iter()
                    .map(|&row| u64::from(row))
                    .sum::<u64>()
            })
            .sum()
    }
}

impl Reader for WordRunView<'_> {
    fn contains(&self, probes: &[u32]) -> u64 {
        probes
            .iter()
            .map(|&key| WordRunView::contains(self, &[key.into()]) as u64)
            .sum()
    }

    fn ranges(&self, probes: &[u32]) -> u64 {
        probes
            .iter()
            .map(|&key| {
                let (begin, end) = self.range(&[key.into()]);
                (begin ^ end) as u64
            })
            .sum()
    }

    fn expand(&self, probes: &[u32]) -> u64 {
        let order = self.order();
        probes
            .iter()
            .map(|&key| {
                let (begin, end) = self.range(&[key.into()]);
                order[begin..end]
                    .iter()
                    .map(|&row| u64::from(row))
                    .sum::<u64>()
            })
            .sum()
    }
}

/// Two untimed warmups, then the median of seven back-to-back runs, in µs.
fn time(mut read: impl FnMut() -> u64) -> (f64, u64) {
    let mut checksum = 0;
    for _ in 0..2 {
        checksum = black_box(read());
    }
    let mut samples: Vec<f64> = (0..7)
        .map(|_| {
            let start = Instant::now();
            assert_eq!(black_box(read()), checksum);
            start.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    (samples[3], checksum)
}

#[test]
#[ignore]
fn spline_and_word_directories_read_the_same_groups() -> Result<()> {
    let gpu = Gpu::new()?;
    println!(
        "| data | rows | reader | directory bytes | contains µs | range µs | range+expand µs | ns/probe (expand) | hits |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for set in datasets() {
        let relation = gpu.relation_u32(set.rows.len(), |row| [set.rows[row]])?;
        let index = gpu.build_radix_index(
            &relation,
            IndexSpec {
                key_column: 0,
                payload: Payload::RowId,
            },
            None,
        )?;
        let state = RadixRunReadState::new(index.keys());
        let spline = spline_view(&index, &state);
        let wide: Vec<u64> = set.rows.iter().map(|&key| key.into()).collect();
        let words = [0, 12, 16]
            .into_iter()
            .map(|radix_bits| {
                let params = WordRunParams {
                    lanes: 1,
                    ms_first: false,
                    active_bits: 32,
                    radix_bits,
                    offsets: OffsetWidth::U32,
                };
                gpu.build_word_run(&wide, params)
                    .map(|run| (radix_bits, run.expect("nonempty")))
            })
            .collect::<Result<Vec<_>>>()?;
        let views: Vec<_> = words
            .iter()
            .map(|(bits, run)| (*bits, WordRunView::from(run)))
            .collect();
        // Identical groups, stable order and row ids for every probe.
        let mut hits = 0;
        for &key in &set.probes {
            let (begin, end) = spline.range(&key);
            hits += (begin != end) as usize;
            for (_, view) in &views {
                let (b, e) = view.range(&[key.into()]);
                assert_eq!(
                    &spline.items()[begin..end],
                    &view.order()[b..e],
                    "{} key {key}",
                    set.name
                );
            }
        }
        let report = |reader: &dyn Reader, label: &str, bytes: usize| {
            let (contains, found) = time(|| reader.contains(&set.probes));
            let (ranges, _) = time(|| reader.ranges(&set.probes));
            let (expand, _) = time(|| reader.expand(&set.probes));
            assert_eq!(found as usize, hits);
            println!(
                "| {} | {} | {label} | {bytes} | {contains:.0} | {ranges:.0} | {expand:.0} | {:.1} | {hits} |",
                set.name,
                set.rows.len(),
                expand * 1e3 / PROBES as f64,
            );
        };
        report(
            &spline,
            &format!(
                "spline ({} points, error {})",
                index.points().len(),
                index.search_error()
            ),
            index.points().len() * 16 + index.radix().len() * 8,
        );
        for (bits, view) in &views {
            let label = if *bits == 0 {
                "word, binary search".to_string()
            } else {
                format!("word, radix_bits {bits}")
            };
            report(view, &label, view.directory().len() * 4);
        }
    }
    Ok(())
}
