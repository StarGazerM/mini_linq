//! Native runtime comparison, not a reproduction of the C++ artifact timings.
//! Usage: native-index-bench [distinct_keys=1048576] [probes=262144] [reps=5]
//! Input is already grouped; builds are CPU-only. Enumeration uses a checksum
//! sink, not full tuple materialization. All modes are warm, single-threaded.
use cq_storage::{Build, ExactMembership, HashIndex, IndexRead, NoDense, RadixRun, SortedRun};
use std::{hint::black_box, time::Instant};

fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Answer {
    count: u64,
    sum: u64,
}
impl Answer {
    fn add(&mut self, value: u64) {
        self.count += 1;
        self.sum = self.sum.wrapping_add(value);
    }
}

struct Data {
    keys: Vec<u64>,
    pairs: Vec<(u64, u64)>,
    payload: Vec<u64>,
    offsets: Vec<usize>,
    queries: Vec<u64>,
    expected: Answer,
    hits: u64,
}
impl Data {
    fn new(n: usize, q: usize) -> Self {
        let mut state = 42_u64;
        let mut random = || {
            state = state.wrapping_add(0x9e3779b97f4a7c15);
            mix(state)
        };
        let mut keys = Vec::with_capacity(n);
        while keys.len() < n {
            keys.extend((keys.len()..n).map(|_| random() & 0x7fff_ffff_ffff_fffe));
            keys.sort_unstable();
            keys.dedup();
        }
        let mut pairs = Vec::with_capacity(n * 4);
        let mut payload = Vec::with_capacity(n * 4);
        let mut offsets = Vec::with_capacity(n + 1);
        for (rank, &key) in keys.iter().enumerate() {
            offsets.push(payload.len());
            for j in 0..2 + random() % 5 {
                // Equal-key duplicate payload occurrences must remain observable.
                let value = mix(rank as u64 ^ (j / 2));
                pairs.push((key, value));
                payload.push(value);
            }
        }
        offsets.push(payload.len());
        let mut expected = Answer::default();
        let mut hits = 0;
        let mut queries = Vec::with_capacity(q);
        for i in 0..q {
            let rank = (random() % n as u64) as usize;
            let hit = i % 2 == 0;
            queries.push(keys[rank] | u64::from(!hit));
            if hit {
                hits += 1;
                for &value in &payload[offsets[rank]..offsets[rank + 1]] {
                    expected.add(value);
                }
            }
        }
        for i in (1..queries.len()).rev() {
            queries.swap(i, (random() % (i + 1) as u64) as usize);
        }
        Self {
            keys,
            pairs,
            payload,
            offsets,
            queries,
            expected,
            hits,
        }
    }
}

struct Hashbrown<'a> {
    map: hashbrown::HashMap<u64, usize>,
    payload: &'a [u64],
    offsets: &'a [usize],
}
impl<'a> Hashbrown<'a> {
    fn new(data: &'a Data) -> Self {
        let mut map = hashbrown::HashMap::with_capacity(data.keys.len());
        for (rank, &key) in data.keys.iter().enumerate() {
            map.insert(key, rank);
        }
        Self {
            map,
            payload: &data.payload,
            offsets: &data.offsets,
        }
    }
}
impl ExactMembership<u64> for Hashbrown<'_> {
    fn contains(&self, key: &u64) -> bool {
        self.map.contains_key(key)
    }
}
impl<'a> IndexRead for &'a Hashbrown<'_> {
    type Key = u64;
    type Item = u64;
    type Cursor = std::iter::Copied<std::slice::Iter<'a, u64>>;
    fn lookup(&self, key: &u64) -> Self::Cursor {
        let slice = self.map.get(key).map_or(&[][..], |rank| {
            &self.payload[self.offsets[*rank]..self.offsets[*rank + 1]]
        });
        slice.iter().copied()
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
fn measure(reps: usize, expected: Answer, mut work: impl FnMut() -> Answer) -> f64 {
    let mut samples = Vec::with_capacity(reps);
    for _ in 0..reps {
        assert_eq!(black_box(work()), expected, "warmup answer");
        let start = Instant::now();
        let answer = black_box(work());
        samples.push(start.elapsed().as_secs_f64() * 1e3);
        assert_eq!(answer, expected, "timed answer");
    }
    median(&mut samples)
}
fn emit(
    name: &str,
    mode: &str,
    data: &Data,
    reps: usize,
    build_ms: f64,
    query_ms: f64,
    sort_ms: f64,
) {
    println!(
        "{name},{mode},{},{},{reps},{build_ms:.6},{query_ms:.6},{sort_ms:.6},{:.3},{},{}",
        data.keys.len(),
        data.queries.len(),
        query_ms * 1e6 / data.queries.len() as f64,
        data.expected.count,
        data.expected.sum
    );
}
fn benchmark<'a, I>(
    name: &str,
    index: &'a I,
    data: &Data,
    sorted: &[u64],
    reps: usize,
    build_ms: f64,
    sort_ms: f64,
) where
    I: ExactMembership<u64>,
    &'a I: IndexRead<Key = u64, Item = u64>,
{
    // Exact membership and complete per-key payloads are independently checked.
    for &q in &data.queries {
        let rank = data.keys.binary_search(&q).ok();
        assert_eq!(index.contains(&q), rank.is_some());
        let expected = rank.map_or(&[][..], |i| {
            &data.payload[data.offsets[i]..data.offsets[i + 1]]
        });
        assert!(index.lookup(&q).eq(expected.iter().copied()));
    }
    let ms = measure(
        reps,
        Answer {
            count: data.hits,
            sum: 0,
        },
        || {
            let count = data.queries.iter().filter(|q| index.contains(q)).count() as u64;
            Answer { count, sum: 0 }
        },
    );
    emit(name, "membership", data, reps, build_ms, ms, 0.0);
    for (mode, queries, sorting) in [
        ("immediate", data.queries.as_slice(), 0.0),
        ("sorted_immediate", sorted, sort_ms),
    ] {
        let ms = measure(reps, data.expected, || {
            let mut answer = Answer::default();
            for key in queries {
                for value in index.lookup(key) {
                    answer.add(value);
                }
            }
            answer
        });
        emit(name, mode, data, reps, build_ms, ms, sorting);
    }
    let mut retained = Vec::with_capacity(data.queries.len());
    let ms = measure(reps, data.expected, || {
        retained.extend(data.queries.iter().map(|key| index.lookup(key)));
        let mut answer = Answer::default();
        for cursor in retained.drain(..) {
            for value in cursor {
                answer.add(value);
            }
        }
        answer
    });
    emit(name, "retained", data, reps, build_ms, ms, 0.0);
    let ms = measure(reps, data.expected, || {
        let mut answer = Answer::default();
        for (_, value) in index.probe::<32, _, _>(data.queries.iter().copied(), |key| *key) {
            answer.add(value);
        }
        answer
    });
    emit(name, "provider_probe32", data, reps, build_ms, ms, 0.0);
}
fn segmented<'a, I>(name: &str, index: &'a I, data: &Data, reps: usize, build_ms: f64)
where
    &'a I: IndexRead<Key = u64, Item = u64>,
    <&'a I as IndexRead>::Cursor: Copy + ExactSizeIterator,
{
    let ms = measure(reps, data.expected, || {
        let mut answer = Answer::default();
        for (_, value) in
            cq_storage::stage::segmented::<32, _, _, _>(data.queries.iter().copied(), |key| {
                index.lookup(key)
            })
        {
            answer.add(value);
        }
        answer
    });
    emit(name, "segmented32", data, reps, build_ms, ms, 0.0);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n = args
        .next()
        .map_or(1 << 20, |v| v.parse().expect("distinct key count"));
    let q = args
        .next()
        .map_or(1 << 18, |v| v.parse().expect("probe count"));
    let reps = args.next().map_or(5, |v| v.parse().expect("repetitions"));
    assert!(
        args.next().is_none() && n > 0 && q > 0 && reps > 0,
        "usage: native-index-bench [positive keys] [positive probes] [positive reps]"
    );
    let data = Data::new(n, q);
    let mut sorted = data.queries.clone();
    let mut sorts = Vec::with_capacity(reps);
    for _ in 0..reps {
        sorted.copy_from_slice(&data.queries);
        let start = Instant::now();
        sorted.sort_unstable();
        sorts.push(start.elapsed().as_secs_f64() * 1e3);
    }
    let sort_ms = median(&mut sorts);
    println!(
        "provider,mode,keys,probes,reps,cpu_build_ms,query_ms,probe_sort_ms,ns_per_probe,join_output_count,join_output_checksum"
    );
    let start = Instant::now();
    let radix = RadixRun::from_pairs(data.pairs.iter().copied());
    let build = start.elapsed().as_secs_f64() * 1e3;
    benchmark(
        "RadixRun_radix12_error32",
        &radix,
        &data,
        &sorted,
        reps,
        build,
        sort_ms,
    );
    segmented("RadixRun_radix12_error32", &radix, &data, reps, build);
    drop(radix);
    let start = Instant::now();
    let binary = SortedRun::from_pairs(data.pairs.iter().copied(), NoDense);
    let build = start.elapsed().as_secs_f64() * 1e3;
    benchmark(
        "SortedRun_binary",
        &binary,
        &data,
        &sorted,
        reps,
        build,
        sort_ms,
    );
    segmented("SortedRun_binary", &binary, &data, reps, build);
    drop(binary);
    let start = Instant::now();
    let hash = HashIndex::build(data.pairs.iter().copied());
    assert_eq!(hash.len(), n); // Force the deferred build inside construction timing.
    let build = start.elapsed().as_secs_f64() * 1e3;
    benchmark(
        "HashIndex_ahash",
        &hash,
        &data,
        &sorted,
        reps,
        build,
        sort_ms,
    );
    drop(hash);
    let start = Instant::now();
    let hash = Hashbrown::new(&data);
    let build = start.elapsed().as_secs_f64() * 1e3;
    benchmark(
        "hashbrown_0.15.5_foldhash_group_rank",
        &hash,
        &data,
        &sorted,
        reps,
        build,
        sort_ms,
    );
    let ms = measure(reps, data.expected, || {
        let mut rank = 0;
        let mut answer = Answer::default();
        for &key in &sorted {
            while rank < data.keys.len() && data.keys[rank] < key {
                rank += 1;
            }
            if rank < data.keys.len() && data.keys[rank] == key {
                for &value in &data.payload[data.offsets[rank]..data.offsets[rank + 1]] {
                    answer.add(value);
                }
            }
        }
        answer
    });
    emit("sorted_groups", "merge", &data, reps, 0.0, ms, sort_ms);
    eprintln!(
        "Verified all native modes: {} keys, {} probes, 50% hits (rounded up), two-to-six payload occurrences/key; single-thread warm CPU timings, checksum sink, no GPU timing.",
        n, q
    );
}
