//! Exercise the real pilot, not a force-route switch. The small-bag case
//! retains witnesses; both grouped cases must write each supported pair once.
use gpu_datalog_runtime::{Buffer, CandidateArena, DeviceRadixRun, Pool, RadixLayout, RadixRunParams, Result, SortedBatch};
use std::{collections::BTreeSet, sync::Arc};

type Layout = RadixLayout<(i16, i16), (), u64, u64, ()>;
fn layout(pool: Arc<Pool>, runs: Vec<Vec<(i16, i16)>>) -> Result<Layout> {
    let pack = Layout::derive_packing()?;
    let params = RadixRunParams { ratio: 2, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 };
    let mut set = DeviceRadixRun::new_set(pool.clone(), params, pack.bits())?;
    for run in runs {
        let mut keys: Vec<u64> = run.iter().map(|&(a,b)| pack.pack(&[i128::from(a), i128::from(b)]))
            .collect::<Result<_>>()?;
        keys.sort_unstable();
        keys.dedup();
        let data = Buffer::from_host(pool.clone(), &keys)?;
        // SAFETY: complete packed rows sorted above; fixture partitions are disjoint.
        unsafe { set.append_sorted(SortedBatch::from_sorted(data, keys.len())?)?; }
    }
    Layout::publish(&set, pack)
}

#[test]
fn pilot_raw_stripe_and_interned_agree_on_exact_signed_support() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    for (groups, witnesses, children, witness_second, group_first) in [
        (5i16, 4i16, 9i16, false, true),
        (128, 64, 512, true, false), // ~3M witnesses, striped
        (1024, 192, 512, false, false), // >50M, exactly three source sets
    ] {
        let mut edges = Vec::new();
        let mut spans = [Vec::new(), Vec::new()];
        let mut expected = BTreeSet::new();
        let out = Layout::derive_packing()?;
        for i in 0..groups {
            let g = i - groups / 2;
            for j in 0..witnesses {
                let w = j - witnesses / 2;
                let live = i % 3 == 2 || i % 3 == j % 2;
                if live { edges.push(if witness_second { (g,w) } else { (w,g) }); }
            }
            for k in 0..children {
                if i % 3 == 2 || (i % 3 == 0 && k < 3 * children / 4) || (i % 3 == 1 && k >= children / 4) {
                    let m = k - children / 2;
                    let (a,b) = if group_first { (g,m) } else { (m,g) };
                    expected.insert(out.pack::<u64>(&[i128::from(a), i128::from(b)])?);
                }
            }
        }
        for j in 0..witnesses {
            for k in 0..children {
                if (j % 2 == 0 && k < 3 * children / 4) || (j % 2 == 1 && k >= children / 4) {
                    // Both runs contain children of each witness. The smaller
                    // second run remains unmerged under the ratio law.
                    spans[usize::from(k % 8 == 0)].push((j - witnesses / 2, k - children / 2));
                }
            }
        }
        let d = layout(pool.clone(), vec![edges])?;
        let r = layout(pool.clone(), spans.into_iter().collect())?;
        assert_eq!(r.run_count(), 2);
        let mut arena = CandidateArena::new(pool.clone());
        let batch = gpu_datalog_runtime::union_project::execute(&mut arena, &d, &r, witness_second, group_first)?
            .expect("dense coordinates fit");
        if groups > 5 { assert_eq!(batch.len(), expected.len(), "grouped writes once"); }
        else { assert!(batch.len() > expected.len(), "raw route preserves multiple witnesses"); }
        assert_eq!(batch.packing(), &out);
        assert_eq!(batch.buffer().to_host()?.into_iter().collect::<BTreeSet<_>>(), expected);
    }
    Ok(())
}
