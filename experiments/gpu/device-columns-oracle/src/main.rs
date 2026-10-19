use gpu_device_columns::{Buffer, Codec, EncodedColumn, Pool, Result};
use std::sync::Arc;

// Afroozeh & Boncz, FastLanes Figure 6: enumerate the UTL tuple order
// directly from the published 04261537 tile order, not the library's
// forward/inverse permutation or its word-at-a-time packer.
fn paper_order() -> Vec<usize> {
    let mut order = Vec::with_capacity(1024);
    for within_tile in 0..8 {
        for tile in [0, 4, 2, 6, 1, 5, 3, 7] {
            for lane in 0..16 {
                order.push(lane * 64 + tile * 8 + within_tile);
            }
        }
    }
    assert_eq!(
        &order[..16],
        &[0, 64, 128, 192, 256, 320, 384, 448, 512, 576, 640, 704, 768, 832, 896, 960]
    );
    assert_eq!(&order[16..20], &[32, 96, 160, 224]);
    assert_eq!(&order[128..132], &[1, 65, 129, 193]);
    assert_eq!(order[1023], 1023);
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..1024).collect::<Vec<_>>());
    order
}

fn inspect(
    label: &str,
    column: &EncodedColumn,
    input: &[i64],
    codec: Codec,
    capacity: usize,
    order: &[usize],
) -> Result<()> {
    let (headers, actual) = column.inspect()?;
    assert_eq!(
        headers.len(),
        capacity.div_ceil(1024),
        "{label}: header count"
    );
    let mut expected = Vec::new();
    for (vector, header) in headers.iter().enumerate() {
        let start = (vector * 1024).min(input.len());
        let end = (start + 1024).min(input.len());
        let values = &input[start..end];
        let transformed: Vec<i64> = if codec == Codec::Delta {
            values
                .iter()
                .enumerate()
                .filter_map(|(i, &v)| (i % 64 != 0).then(|| v.wrapping_sub(values[i - 1])))
                .collect()
        } else {
            values.to_vec()
        };
        let low = transformed.iter().copied().min().unwrap_or(0);
        let high = transformed.iter().copied().max().unwrap_or(0);
        // i128 subtraction independently represents the entire signed-i64 range.
        let range = (high as i128 - low as i128) as u128;
        let width = if codec == Codec::Plain {
            64
        } else {
            (0..64).filter(|&bit| range >> bit != 0).count()
        };
        let base = if codec == Codec::Plain { 0 } else { low };
        assert_eq!(
            header.offset as usize,
            expected.len(),
            "{label}: offset vector {vector}"
        );
        assert_eq!(header.base, base, "{label}: base vector {vector}");
        assert_eq!(
            header.width as usize, width,
            "{label}: width vector {vector}"
        );
        assert_eq!(header.codec, codec as u32, "{label}: codec vector {vector}");
        assert_eq!(
            header.rows as usize,
            values.len(),
            "{label}: rows vector {vector}"
        );
        assert_eq!(header.padding, 0, "{label}: header padding");
        if codec == Codec::Delta {
            for lane in 0..16 {
                expected.push(values.get(lane * 64).copied().unwrap_or(0) as u64);
            }
        }
        let payload_start = expected.len();
        expected.resize(payload_start + width * 16, 0);
        // Paper model: sixteen independent T=64 bit streams, interleaved
        // at 64-bit word boundaries. Set each bit independently, including
        // fields straddling words; no production packing helper is called.
        for (position, &row) in order.iter().enumerate() {
            if row >= values.len() || (codec == Codec::Delta && row % 64 == 0) {
                continue;
            }
            let value = if codec == Codec::Delta {
                values[row].wrapping_sub(values[row - 1])
            } else {
                values[row]
            };
            let residual = value.wrapping_sub(base) as u64;
            for bit in 0..width {
                if residual & (1u64 << bit) != 0 {
                    let stream_bit = (position / 16) * width + bit;
                    expected[payload_start + (stream_bit / 64) * 16 + position % 16] |=
                        1u64 << (stream_bit % 64);
                }
            }
        }
    }
    assert_eq!(actual.len(), expected.len(), "{label}: payload extent");
    for (word, (&got, &want)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(got, want, "{label}: payload word {word}, codec {codec:?}");
    }
    assert_eq!(column.to_host()?, input, "{label}: GPU decode {codec:?}");
    Ok(())
}

fn check(
    pool: &Arc<Pool>,
    label: &str,
    values: &[i64],
    codec: Codec,
    order: &[usize],
) -> Result<()> {
    let encoded = EncodedColumn::from_host(pool.clone(), values, codec)?;
    inspect(label, &encoded, values, codec, values.len(), order)
}

fn device_output(pool: &Arc<Pool>, codec: Codec, len: usize, order: &[usize]) -> Result<()> {
    let capacity = 3073;
    let mut input: Vec<i64> = (0..capacity).map(|i| (i as i64 * 17 % 97) - 48).collect();
    input[len..].fill(i64::MAX); // Poison inactive rows; tight bounds exclude poison.
    let raw = Buffer::from_host(pool.clone(), &input)?;
    let count = Buffer::from_host(pool.clone(), &[len as u64])?;
    // From here until assertions, only device encoders/decoders are used:
    // no host upload, host-derived replacement column, or count download.
    let encoded = unsafe {
        EncodedColumn::from_device(
            pool.clone(),
            raw.ptr(),
            capacity,
            count.ptr(),
            codec,
            (-48, 48),
        )?
    };
    let decoded = encoded.decoded()?;
    let chained = unsafe {
        EncodedColumn::from_device(
            pool.clone(),
            decoded.ptr(),
            capacity,
            count.ptr(),
            codec,
            (-48, 48),
        )?
    };
    inspect(
        "device-output",
        &encoded,
        &input[..len],
        codec,
        capacity,
        order,
    )?;
    inspect(
        "device-output chained",
        &chained,
        &input[..len],
        codec,
        capacity,
        order,
    )?;
    let scratch = decoded.to_host()?;
    assert_eq!(&scratch[..len], &input[..len]);
    assert!(
        scratch[len..].iter().all(|&v| v == 0),
        "decode writes outside logical count"
    );
    Ok(())
}

fn main() -> Result<()> {
    let pool = Pool::new(0, 8 << 20, 256 << 20)?;
    let order = paper_order();
    let codecs = [Codec::Plain, Codec::For, Codec::Delta, Codec::Dictionary];
    let mut cases = 0usize;
    for codec in codecs {
        for n in [0, 1, 63, 64, 65, 1023, 1024, 1025, 2049] {
            let patterns = [
                ("all-equal", vec![-71; n]),
                (
                    "negative",
                    (0..n).map(|i| -1 - (i * 37 % 101) as i64).collect(),
                ),
                (
                    "full-width",
                    (0..n)
                        .map(|i| if i % 2 == 0 { i64::MIN } else { i64::MAX })
                        .collect(),
                ),
                ("sorted", (0..n).map(|i| i as i64 * 3 - 9000).collect()),
                (
                    "near-sorted",
                    (0..n)
                        .map(|i| i as i64 * 3 - 9000 - if i % 31 == 0 { 7 } else { 0 })
                        .collect(),
                ),
                (
                    "lane-restarts",
                    (0..n)
                        .map(|i| (i / 64) as i64 * -100003 + (i % 64) as i64 * 11)
                        .collect(),
                ),
                ("identity-UTL", (0..n).map(|i| i as i64).collect()),
            ];
            for (label, values) in patterns {
                check(&pool, label, &values, codec, &order)?;
                cases += 1;
            }
        }
        for width in 0..=64 {
            // Signed endpoints whose difference is exactly 2^width - 1.
            let low = if width == 64 { i64::MIN } else { 0 };
            let high = if width == 64 {
                i64::MAX
            } else {
                ((1u64 << width) - 1) as i64
            };
            let mut values: Vec<i64> = Vec::with_capacity(2049);
            for i in 0..2049 {
                if codec == Codec::Delta {
                    let value = if i % 64 == 0 {
                        (i as i64).wrapping_mul(-99991)
                    } else {
                        values[i - 1].wrapping_add(if i % 3 == 0 { high } else { low })
                    };
                    values.push(value);
                } else {
                    values.push(if i % 3 == 0 { high } else { low });
                }
            }
            check(&pool, &format!("width-{width}"), &values, codec, &order)?;
            cases += 1;
        }
        for len in [0, 1, 65, 1025, 2049] {
            device_output(&pool, codec, len, &order)?;
            cases += 1;
        }
    }
    // IDs belong to one shared dictionary across independently encoded
    // columns: repeated IDs and different local subsets must remain global.
    let dictionary = ["alpha", "beta", "delta", "omega", "zeta"];
    let left = [4i64, 0, 4, 2, 0, 2, 4];
    let right = [2i64, 4, 1, 2, 1, 4];
    let a = EncodedColumn::from_host(pool.clone(), &left, Codec::Dictionary)?;
    let b = EncodedColumn::from_host(pool, &right, Codec::Dictionary)?;
    inspect(
        "shared dictionary left",
        &a,
        &left,
        Codec::Dictionary,
        left.len(),
        &order,
    )?;
    inspect(
        "shared dictionary right",
        &b,
        &right,
        Codec::Dictionary,
        right.len(),
        &order,
    )?;
    let decoded_a = a.to_host()?;
    let decoded_b = b.to_host()?;
    for (i, &id_a) in decoded_a.iter().enumerate() {
        for (j, &id_b) in decoded_b.iter().enumerate() {
            assert_eq!(
                id_a == id_b,
                dictionary[left[i] as usize] == dictionary[right[j] as usize]
            );
        }
    }
    println!("gpu-device-columns exactness: {} cases passed; CPU-paper headers/UTL/packing, GPU decode, shared IDs, device-only re-encode", cases + 2);
    Ok(())
}
