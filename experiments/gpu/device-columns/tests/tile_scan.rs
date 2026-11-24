//! FOR-native tile laws, executed through the same helpers as generated scans.
use cuda_core::{DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Buffer, Codec, ColumnView, EncodedColumn, Pool, VectorHeader};
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Bounds {
    low: i64,
    high: i64,
    low_inclusive: u32,
    high_inclusive: u32,
    alive: u32,
}
unsafe impl DeviceCopy for Bounds {}

#[cuda_host::cuda_module]
mod kernels {
    use super::Bounds;
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    use gpu_device_columns::{original_position, tile, ColumnView};
    use tile::{CountingPayload, RawPayload};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn scan(
        view: ColumnView,
        native: ColumnView,
        bounds: *const Bounds,
        queries: usize,
        masks: *mut u8,
        decoded: *mut i64,
        companions: *mut i64,
        loads: *mut u64,
    ) {
        let lane = thread::threadIdx_x() as usize;
        let vectors = (view.len as usize).div_ceil(1024).max(1);
        let mut work = thread::blockIdx_x() as usize;
        while work < queries * vectors {
            let query = work / vectors;
            let vector = work % vectors;
            let bound = unsafe { bounds.add(query).read() };
            // Header snapshots and translations are outside the position loop.
            let frame = unsafe { tile::frame(view, vector) };
            let companion = unsafe { tile::frame(native, vector) };
            let window = tile::window(frame, bound.low, bound.low_inclusive != 0, bound.high, bound.high_inclusive != 0);
            let alive = tile::live_mask(frame, lane) & bound.alive as u8;
            let counted = unsafe {
                tile::window_lanes(frame, lane, alive, window, CountingPayload { loads: loads.add(query * 2) })
            };
            let raw = unsafe { tile::window_lanes(frame, lane, alive, window, RawPayload) };
            let mut k = 0;
            while k < 4 {
                let physical = lane + k * 256;
                let row = vector * 1024 + original_position(physical);
                if row < view.len as usize {
                    let index = query * view.len as usize + row;
                    unsafe {
                        masks.add(index).write(if counted != raw { 2 } else { ((counted >> k) & 1) as u8 });
                        if counted & (1 << k) != 0 {
                            decoded.add(index).write(tile::read(frame, physical, CountingPayload { loads: loads.add(query * 2 + 1) }));
                            companions.add(index).write(tile::read(companion, physical, RawPayload));
                        }
                    }
                }
                k += 1;
            }
            work += thread::gridDim_x() as usize;
        }
    }
}

fn inclusive(low: i64, high: i64) -> Bounds {
    Bounds { low, high, low_inclusive: 1, high_inclusive: 1, alive: 15 }
}

fn queries(values: &[i64]) -> Vec<Bounds> {
    let mut bounds = vec![
        inclusive(i64::MIN, i64::MAX),
        inclusive(i64::MIN, i64::MIN),
        inclusive(i64::MAX, i64::MAX),
        inclusive(0, 0),
        inclusive(17, 17),
        inclusive(-1, 1),
        inclusive(100, -100),
        inclusive(i64::MIN, -1),
        inclusive(0, i64::MAX),
        inclusive(123456789, 123456789),
    ];
    for &(low, high) in &[(i64::MIN, i64::MAX), (i64::MIN, i64::MIN), (i64::MAX, i64::MAX), (-1, 1)] {
        for low_inclusive in 0..=1 {
            for high_inclusive in 0..=1 {
                bounds.push(Bounds { low, high, low_inclusive, high_inclusive, alive: 15 });
            }
        }
    }
    for &value in values.iter().step_by(1024).take(65) {
        bounds.push(inclusive(value, value));
    }
    for alive in [0, 1, 2, 4, 8, 5, 10] {
        bounds.push(Bounds { alive, ..inclusive(i64::MIN, i64::MAX) });
        bounds.push(Bounds { alive, ..inclusive(-1, 1) });
    }
    bounds
}

// Independent CPU tuple-order oracle: Figure 6's 04261537 permutation.
fn physical(row: usize) -> usize {
    let tiles = [0, 4, 2, 6, 1, 5, 3, 7];
    (row % 8) * 128 + tiles[(row % 64) / 8] * 16 + row / 64
}

// Build independent bit-by-bit fixtures. A test-defined CUDA module owns this
// executable's image, so calling the library encoder kernels would resolve
// against the wrong image. Real encoder interoperability is covered by the
// TPC-H codec smokes; these fixtures independently exercise the frozen layout.
fn packed_fixture(values: &[i64], codec: Codec) -> (Vec<VectorHeader>, Vec<u64>) {
    let mut headers = Vec::new();
    let mut payload = Vec::new();
    for vector in values.chunks(1024) {
        let delta = codec == Codec::Delta;
        let codes: Vec<i64> = vector.iter().enumerate().filter_map(|(row, &value)| {
            if delta && row % 64 == 0 { None }
            else { Some(if delta { value.wrapping_sub(vector[row - 1]) } else { value }) }
        }).collect();
        let base = if codec == Codec::Plain { 0 } else { codes.iter().copied().min().unwrap_or(0) };
        let maximum = codes.iter().copied().max().unwrap_or(base);
        let width = if codec == Codec::Plain { 64 }
            else { 64 - (maximum as u64).wrapping_sub(base as u64).leading_zeros() };
        let offset = payload.len();
        let prefix = if delta { 16 } else { 0 };
        payload.resize(offset + prefix + width as usize * 16, 0u64);
        for (row, &value) in vector.iter().enumerate() {
            if delta && row % 64 == 0 { payload[offset + row / 64] = value as u64; continue; }
            let value = if delta { value.wrapping_sub(vector[row - 1]) } else { value };
            let residual = (value as u64).wrapping_sub(base as u64);
            let physical = physical(row);
            for bit in 0..width as usize {
                let position = physical / 16 * width as usize + bit;
                let word = offset + prefix + position / 64 * 16 + physical % 16;
                payload[word] |= ((residual >> bit) & 1) << (position % 64);
            }
        }
        headers.push(VectorHeader { offset: offset as u64, base, width,
            codec: codec as u32, rows: vector.len() as u32, padding: 0 });
    }
    (headers, payload)
}

struct Fixture {
    headers: Vec<VectorHeader>,
    device_headers: Buffer<VectorHeader>,
    payload: Buffer<u64>,
    len: usize,
    codec: Codec,
}
impl Fixture {
    fn new(pool: &Arc<Pool>, values: &[i64], codec: Codec) -> Self {
        let (headers, payload) = packed_fixture(values, codec);
        Self {
            device_headers: Buffer::from_host(pool.clone(), &headers).unwrap(),
            payload: Buffer::from_host(pool.clone(), &payload).unwrap(),
            headers, len: values.len(), codec,
        }
    }
    fn view(&self) -> ColumnView {
        ColumnView { headers: self.device_headers.ptr(), data: self.payload.ptr(),
            len: self.len as u64, count: std::ptr::null(), codec: self.codec as u32 }
    }
}

fn packed_loads(width: u32, row: usize) -> u64 {
    if width == 0 {
        0
    } else {
        let shift = ((physical(row) / 16) as u32 * width) % 64;
        1 + u64::from(shift + width > 64)
    }
}

fn scalar_loads(header: Option<&VectorHeader>, row: usize) -> u64 {
    match header {
        None => 1,
        Some(header) if header.codec == Codec::Delta as u32 => {
            1 + (1..=row % 64)
                .map(|i| packed_loads(header.width, row / 64 * 64 + i))
                .sum::<u64>()
        }
        Some(header) => packed_loads(header.width, row),
    }
}

fn interval(bound: Bounds) -> (i128, i128) {
    (bound.low as i128 + i128::from(bound.low_inclusive == 0),
     bound.high as i128 - i128::from(bound.high_inclusive == 0))
}

// Width is an enclosure, not an exact maximum. Count only loads required by
// that certificate, independently using i128 rather than device saturation.
fn predicate_loads(header: Option<&VectorHeader>, row: usize, bound: Bounds) -> u64 {
    let (low, high) = interval(bound);
    if low > high || low > i64::MAX as i128 || high < i64::MIN as i128 {
        return 0;
    }
    let (minimum, maximum) = match header {
        Some(h) if h.codec != Codec::Delta as u32 && h.codec != Codec::Plain as u32 => {
            (h.base as i128, (h.base as i128 + ((1i128 << h.width) - 1)).min(i64::MAX as i128))
        }
        _ => (i64::MIN as i128, i64::MAX as i128),
    };
    if high < minimum || low > maximum || (low <= minimum && high >= maximum) {
        0
    } else {
        scalar_loads(header, row)
    }
}

fn exercise(pool: &Arc<Pool>, view: ColumnView, headers: Option<&[VectorHeader]>, values: &[i64], live: usize) {
    assert_eq!(view.len as usize, values.len());
    let count = Buffer::from_host(pool.clone(), &[live as u64]).unwrap();
    let view = ColumnView { count: count.ptr(), ..view };
    // Always combine an encoded tested column with a native survivor column.
    // It must denote the same row at every transposed physical position.
    let companion_values: Vec<i64> = (0..values.len()).map(|row| (row as i64).wrapping_mul(-7919).wrapping_add(31)).collect();
    let companion = EncodedColumn::from_host_native(pool.clone(), &companion_values).unwrap();
    let bounds = queries(values);
    let query_buffer = Buffer::from_host(pool.clone(), &bounds).unwrap();
    let cells = (values.len() * bounds.len()).max(1);
    let masks = Buffer::<u8>::zeroed(pool.clone(), cells).unwrap();
    let decoded = Buffer::<i64>::zeroed(pool.clone(), cells).unwrap();
    let companions = Buffer::<i64>::zeroed(pool.clone(), cells).unwrap();
    let loads = Buffer::<u64>::zeroed(pool.clone(), bounds.len() * 2).unwrap();
    let module = unsafe { kernels::load(pool.context()).unwrap() };
    // A deliberately tiny grid forces vector-striding across mixed headers.
    let launch = module.prepare_scan(LaunchConfig1D::new(3, 256, 0)).unwrap();
    // All borrowed buffers stay live through the synchronous downloads below.
    unsafe { module.scan(pool.stream(), &launch, view, companion.view(), query_buffer.ptr(), bounds.len(), masks.ptr(), decoded.ptr(), companions.ptr(), loads.ptr()).unwrap(); }
    let masks = masks.to_host().unwrap();
    let decoded = decoded.to_host().unwrap();
    let companions = companions.to_host().unwrap();
    let loads = loads.to_host().unwrap();
    let live = live.min(values.len());
    for (query, &bound) in bounds.iter().enumerate() {
        let (low, high) = interval(bound);
        let mut expected_predicate_loads = 0;
        let mut expected_survivor_loads = 0;
        for (row, &value) in values.iter().enumerate() {
            let local = row % 1024;
            let header = headers.map(|headers| &headers[row / 1024]);
            let alive = row < live && bound.alive & (1 << (physical(local) / 256)) != 0;
            let expected = alive && low <= value as i128 && value as i128 <= high;
            let index = query * values.len() + row;
            assert_eq!(masks[index], u8::from(expected), "query {bound:?}, row {row}, live {live}");
            if alive {
                expected_predicate_loads += predicate_loads(header, local, bound);
            }
            if expected {
                expected_survivor_loads += scalar_loads(header, local);
                assert_eq!(decoded[index], value, "survivor row {row}");
                assert_eq!(companions[index], companion_values[row], "native alignment row {row}");
            } else {
                assert_eq!(decoded[index], 0);
                assert_eq!(companions[index], 0);
            }
        }
        assert_eq!(loads[query * 2], expected_predicate_loads, "predicate loads: {bound:?}, live {live}");
        assert_eq!(loads[query * 2 + 1], expected_survivor_loads, "survivor loads: {bound:?}, live {live}");
    }
}

#[test]
fn codecs_extremes_tails_and_dead_lane_loads() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let mut seed = 0x57bc_9041_a27e_16d3u64;
    let random: Vec<i64> = (0..2048).map(|_| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as i64
    }).collect();
    let corpus = [
        vec![],
        vec![i64::MIN],
        vec![i64::MAX],
        vec![-1; 2048],
        (0..2048).map(|i| 1_000_000 + i * 4).collect(),
        (0..2048).map(|i| -1_000_000 + i * 4 + i % 3).collect(),
        (0..2048).map(|i| [i64::MIN, i64::MAX, -1, 0, 1][i % 5]).collect(),
        random,
    ];
    for values in corpus {
        for codec in [Codec::For, Codec::Plain, Codec::Delta] {
            let column = Fixture::new(&pool, &values, codec);
            let headers = &column.headers;
            for live in [0, 1, 255, 256, 1023, 1025, 1279, 1280, 2047, values.len(), values.len() + 17] {
                exercise(&pool, column.view(), Some(headers), &values, live);
            }
        }
        let native = EncodedColumn::from_host_native(pool.clone(), &values).unwrap();
        for live in [0, 1, 255, 256, 1023, 1025, 1279, 1280, 2047, values.len()] {
            exercise(&pool, unsafe { native.view() }, None, &values, live);
        }
    }
    // Actual partial header rows, not merely a counted prefix of a full vector.
    for tail in [1, 255, 256, 1023] {
        let values: Vec<i64> = (0..1024 + tail).map(|i| i as i64 - 800).collect();
        for codec in [Codec::For, Codec::Plain, Codec::Delta] {
            let column = Fixture::new(&pool, &values, codec);
            exercise(&pool, column.view(), Some(&column.headers), &values, values.len());
        }
    }
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn every_width_and_per_vector_codec_dispatch() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let mut values = Vec::new();
    for width in 0..=64 {
        let span = (1i128 << width) - 1;
        let base = if width % 2 == 0 { i64::MIN as i128 } else { i64::MAX as i128 - span };
        for row in 0..1024 {
            let residual = if row == 0 { 0 } else if row == 1 { span } else { span * row / 1023 };
            values.push((base + residual) as i64);
        }
    }
    values.extend([i64::MIN, 0, i64::MAX]);
    let column = Fixture::new(&pool, &values, Codec::For);
    assert_eq!(column.headers[..65].iter().map(|h| h.width).collect::<Vec<_>>(), (0..=64).collect::<Vec<_>>());
    exercise(&pool, column.view(), Some(&column.headers), &values, values.len());

    // Stitch independent packed vectors into a legal column whose top-level
    // codec intentionally cannot describe every vector. Dispatch must use h.codec.
    let mut mixed_headers = Vec::new();
    let mut mixed_payload = Vec::new();
    let mut mixed_values = Vec::new();
    for (index, codec) in [Codec::For, Codec::Plain, Codec::Delta, Codec::For, Codec::Delta, Codec::Plain].into_iter().enumerate() {
        let rows = if index == 5 { 255 } else { 1024 };
        let values: Vec<i64> = (0..rows).map(|row| match index {
            0 => -7,
            1 => [i64::MIN, i64::MAX, -1, 0, 1][row % 5],
            2 => -500 + row as i64 * 3 + (row % 7) as i64,
            3 => row as i64 - 512,
            4 => 9000 - row as i64 * 7,
            _ => row as i64 * -11,
        }).collect();
        let (mut headers, payload) = packed_fixture(&values, codec);
        assert_eq!(headers.len(), 1);
        headers[0].offset = mixed_payload.len() as u64;
        mixed_headers.extend(headers);
        mixed_payload.extend(payload);
        mixed_values.extend(values);
    }
    let headers = Buffer::from_host(pool.clone(), &mixed_headers).unwrap();
    let payload = Buffer::from_host(pool.clone(), &mixed_payload).unwrap();
    let view = ColumnView {
        headers: headers.ptr(), data: payload.ptr(), len: mixed_values.len() as u64,
        count: std::ptr::null(), codec: Codec::For as u32,
    };
    exercise(&pool, view, Some(&mixed_headers), &mixed_values, mixed_values.len());
    gpu_device_columns::release_cached_kernels();
}
