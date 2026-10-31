use cq_gpu_runtime::{Codec, DeviceColumns, Dictionary, Pool};
use cq_gpu_sequence_runtime::Result;

#[test]
fn recorded_for_order_survives_packed_flat_and_terminal_publications() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    for raw in [
        vec![],
        vec![(-1i64, u32::MAX); 5],
        vec![
            (i64::MIN, 0),
            (i64::MAX, u32::MAX),
            (-7, 2),
            (-7, 2),
            (0, 1),
            (i64::MIN, u32::MAX),
        ],
    ] {
        let data = DeviceColumns::from_rows(
            pool.clone(),
            &raw,
            &mut Dictionary::default(),
            &[Codec::For, Codec::For],
        )?;
        let output = cq_gpu_sequence_test_macros::sequence! {
            physical {
                relation R(a:i64,b:u32) = (&data);
                r = let canon rename R@0 {a -> a,b -> b};
                return r as answer(b,a);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,u32)> [];
                r rows = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                r canon = rows;
            }
            sequence {
                input = store rows r;
                packed_feed = canon flatten tile 256 input;
                packed = store packed_feed;
                raw_feed = flatten tile 256 packed;
                raw_rows = store raw_feed;
                terminal_feed = flatten tile 256 raw_rows;
                result = store terminal_feed;
                output count;
            }
            graded gpu {
                input at host;
                packed_feed at device.block;
                packed at host;
                raw_feed at device.block;
                raw_rows at host;
                terminal_feed at device.block;
                result at host;
            }
        };
        let mut actual: Vec<(u32, i64)> = output.to_host()?;
        actual.sort_unstable();
        let mut expected: Vec<_> = raw.iter().map(|&(a, b)| (b, a)).collect();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(actual, expected);
    }
    Ok(())
}
