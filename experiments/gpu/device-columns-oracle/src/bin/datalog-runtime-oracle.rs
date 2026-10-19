use cq_gpu_runtime::{
    datalog::{Relation, RoundStatus},
    *,
};
use cuda_core::LaunchConfig1D;
use std::{collections::BTreeSet, sync::Arc};

#[cuda_device::cuda_module]
mod checks {
    use cq_gpu_runtime::{
        ExactMembershipView, OutputView, TableView, column, exact_contains, table_len,
    };
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn copy(source: TableView, output: OutputView) {
        let row = thread::index_1d().get() as u64;
        if row < unsafe { table_len(source) } {
            let destination = unsafe { cq_gpu_runtime::reserve(output, 1) };
            unsafe {
                cq_gpu_runtime::write_output(output, 0, destination, column(source, 0, row));
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn probe(source: TableView, set: ExactMembershipView, output: *mut u32) {
        let mut row = thread::index_1d().get() as u64;
        while row < unsafe { table_len(source) } {
            let keys = [unsafe { column(source, 0, row) as u64 }, unsafe {
                column(source, 1, row) as u64
            }];
            unsafe {
                output
                    .add(row as usize)
                    .write(exact_contains(set, &keys) as u32);
            }
            row += thread::gridDim_x() as u64 * 256;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn overflow(output: OutputView) {
        if thread::index_1d().get() == 0 {
            unsafe {
                output.count.write(3);
            }
        }
    }
}
fn exact<T: Ord + std::fmt::Debug>(actual: Vec<T>, expected: impl IntoIterator<Item = T>) {
    let count = actual.len();
    let actual: BTreeSet<_> = actual.into_iter().collect();
    assert_eq!(
        count,
        actual.len(),
        "duplicate full rows escaped canonical admission"
    );
    assert_eq!(actual, expected.into_iter().collect());
}
fn status<Row, C>(input: &DeviceColumns<'_, Row, C>) -> Result<bool> {
    let mut status = RoundStatus::new(input.pool().clone());
    status.include(input)?;
    Ok(status.check()?.any())
}
fn prepare<'data, Row: RowCodec<'data>, C>(
    relation: &Relation<'data, Row>,
    candidates: &[DeviceColumns<'data, Row, C>],
) -> Result<Relation<'data, Row>> {
    let mut round = RoundStatus::new(relation.full().pool().clone());
    let pending = relation.prepare(candidates, &mut round)?;
    pending.finish(&round.check()?)
}

fn batched_rounds(runtime: &Runtime, dictionary: &mut Dictionary<'static>) -> Result<()> {
    let input = runtime.upload_owned_rows(&[(17i64,)], dictionary, &[Codec::Plain])?;
    let module = unsafe { checks::load(runtime.context())? };
    let launch = module.prepare_copy(LaunchConfig1D::new(1, 256, 0))?;
    for members in [1, 2, 8, 32] {
        for holes in [1, 2, 8] {
            let empty =
                Relation::<(i64,)>::empty(runtime.pool().clone(), Arc::new(dictionary.clone()), 1)?;
            let mut relations = vec![empty; members];
            reset_host_wait_count();
            for round_index in 0..2 {
                let mut round = RoundStatus::new(runtime.pool().clone());
                let pending = relations
                    .iter()
                    .map(|relation| {
                        let candidates = (0..holes)
                            .map(|_| {
                                let mut execution = runtime.execution();
                                let source = execution.bind_columns(&input)?;
                                let source = execution.table_view(source)?;
                                let output = execution.execute_bounded(
                                    &[Bounds::singleton(17)],
                                    1,
                                    |stream, output| {
                                        unsafe {
                                            module.copy(stream, &launch, source, output)?;
                                        }
                                        Ok(())
                                    },
                                )?;
                                execution.into_columns::<(i64,), Plain>(output, &[0])
                            })
                            .collect::<Result<Vec<_>>>()?;
                        relation.prepare(&candidates, &mut round)
                    })
                    .collect::<Result<Vec<_>>>()?;
                assert_eq!(
                    host_wait_count(),
                    round_index,
                    "holes/members must not wait"
                );
                let checked = round.check()?;
                assert_eq!(checked.any(), round_index == 0);
                let next = pending
                    .into_iter()
                    .map(|member| member.finish(&checked))
                    .collect::<Result<Vec<_>>>()?;
                assert_eq!(
                    host_wait_count(),
                    round_index + 1,
                    "one wait for H={holes}, M={members}"
                );
                relations = next;
            }
            assert_eq!(control_readback_count(), 2);
            assert_eq!(resume_readback_count(), 0);
            for relation in relations {
                exact(relation.full().to_host()?, [(17,)]);
                exact(relation.delta().to_host()?, []);
            }
        }
    }
    println!("batched rounds exact=true holes_per_member=1,2,8 members=1,2,8,32 waits_per_round=1");
    Ok(())
}

fn persistent_growth(runtime: &Runtime, dictionary: &mut Dictionary<'static>) -> Result<()> {
    let mut relation = Relation::<(i64, i64)>::empty(
        runtime.pool().clone(), Arc::new(dictionary.clone()), 2,
    )?;
    let mut expected = BTreeSet::new();
    let mut snapshots = Vec::new();
    for round in 0..10 {
        let end = 1i64 << round;
        let rows: Vec<_> = (0..end).map(|value| (value, !value)).collect();
        let candidates = runtime.upload_owned_rows(&rows, dictionary, &[Codec::Plain; 2])?;
        snapshots.push((relation.full().clone(), expected.clone()));
        let next = prepare(&relation, &[candidates.clone(), candidates])?;
        let delta: BTreeSet<_> = rows.iter().copied()
            .filter(|row| !expected.contains(row)).collect();
        exact(next.delta().to_host()?, delta);
        expected.extend(rows);
        exact(next.full().to_host()?, expected.iter().copied());
        if round == 5 {
            // Branch from an old Relation after its successor appended. Its
            // lazy state must detach rather than overwrite the successor tail.
            let branch_rows = runtime.upload_owned_rows(
                &[(-1, 7), (-1, 7)], dictionary, &[Codec::Plain; 2],
            )?;
            let branch = prepare(&relation, &[branch_rows])?;
            let mut branch_expected = snapshots.last().unwrap().1.clone();
            branch_expected.insert((-1, 7));
            exact(branch.full().to_host()?, branch_expected);
            exact(next.full().to_host()?, expected.iter().copied());
        }
        relation = next;
    }
    for (snapshot, expected) in snapshots {
        exact(snapshot.to_host()?, expected);
    }
    println!("persistent admission exact=true: geometric full/set growth, duplicate holes, existing candidates, immutable prefixes and detached old branches");
    Ok(())
}

fn mismatched_batch(runtime: &Runtime, dictionary: &Dictionary<'_>) -> Result<()> {
    let relation =
        Relation::<(i64,)>::empty(runtime.pool().clone(), Arc::new(dictionary.clone()), 1)?;
    let mut first = RoundStatus::new(runtime.pool().clone());
    let pending = relation.prepare::<Plain>(&[], &mut first)?;
    let mut second = RoundStatus::new(runtime.pool().clone());
    let _other = relation.prepare::<Plain>(&[], &mut second)?;
    assert!(
        pending
            .finish(&second.check()?)
            .err()
            .unwrap()
            .to_string()
            .contains("CQ_GPU_ROUND")
    );
    Ok(())
}
fn main() -> Result<()> {
    let runtime = Runtime::with_pool(0, 64 << 20, 512 << 20)?;
    let mut dictionary = Dictionary::default();
    batched_rounds(&runtime, &mut dictionary)?;
    mismatched_batch(&runtime, &dictionary)?;
    persistent_growth(&runtime, &mut dictionary)?;
    // Force 32 different full rows into the same initial hash bucket. Each
    // appears twice; neither hash collisions nor row-ID races may lose rows.
    let colliding: Vec<_> = (0..i64::MAX)
        .filter(|&value| {
            let mut hash = 0x9e37_79b9_7f4a_7c15u64;
            for word in [i64::MIN as u64, value as u64, i64::MAX as u64] {
                hash ^= word;
                hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                hash ^= hash >> 31;
            }
            hash & 127 == 0
        })
        .take(32)
        .map(|value| (i64::MIN, value, i64::MAX))
        .collect();
    let doubled: Vec<_> = colliding.iter().flat_map(|&row| [row, row]).collect();
    let collision_input =
        runtime.upload_owned_rows(&doubled, &mut dictionary, &[Codec::Plain; 3])?;
    let collisions = Relation::from_columns(&collision_input)?;
    exact(collisions.full().to_host()?, colliding);
    assert!(!status(prepare(&collisions, &[collision_input])?.delta())?);
    let codes = [Codec::For, Codec::Dictionary];
    let rows: Vec<_> = (0..4096i64)
        .flat_map(|i| [(i, i ^ 31), (i, i ^ 31), (i, i ^ 63)])
        .collect();
    let input = runtime.upload_owned_rows(&rows, &mut dictionary, &codes)?;
    let mut state = Relation::from_columns(&input)?;
    exact(state.full().to_host()?, rows.clone());
    let original = state.full().clone();
    let mut expected: BTreeSet<_> = rows.into_iter().collect();
    for cycle in 0..8i64 {
        let rows = [
            (cycle, cycle ^ 31),
            (5000 + cycle, -cycle),
            (5000 + cycle, -cycle),
        ];
        let candidate = runtime.upload_owned_rows(&rows, &mut dictionary, &codes)?;
        let old = state.full().clone();
        let next = prepare(&state, &[candidate.clone(), candidate])?;
        exact(old.to_host()?, expected.iter().copied());
        exact(next.delta().to_host()?, [(5000 + cycle, -cycle)]);
        assert_eq!(next.delta().capacity(), 1, "checked deltas must not retain candidate capacity");
        assert!(status(next.delta())?);
        expected.insert((5000 + cycle, -cycle));
        exact(next.full().to_host()?, expected.iter().copied());
        assert_eq!(next.full().capacity(), expected.len());
        state = next;
    }
    assert_eq!(original.to_host()?.len(), 8192);
    state = prepare::<_, Plain>(&state, &[])?;
    assert!(!status(state.delta())?);
    assert_eq!(state.delta().capacity(), 0);
    exact(state.full().to_host()?, expected);
    let empty =
        Relation::<(i64, i64)>::empty(runtime.pool().clone(), Arc::new(dictionary.clone()), 2)?;
    assert!(!status(empty.delta())?);
    let empty = prepare::<_, Plain>(&empty, &[])?;
    exact(empty.full().to_host()?, []);

    // Lossy head projection must be canonicalized after projection.
    let projection = {
        let mut execution = runtime.execution();
        let source = execution.bind_columns(&input)?;
        execution.into_columns::<(i64,), Plain>(source, &[0])?
    };
    let projected = Relation::from_columns(&projection)?;
    exact(projected.full().to_host()?, (0..4096).map(|x| (x,)));

    // Different SCC member deltas, including an empty member on each round.
    let mut a = Relation::<(i64,)>::empty(runtime.pool().clone(), Arc::new(dictionary.clone()), 1)?;
    let mut b = a.clone();
    for cycle in 0..6i64 {
        let value = runtime.upload_owned_rows(&[(cycle,)], &mut dictionary, &[Codec::Plain])?;
        let mut summary = RoundStatus::new(runtime.pool().clone());
        let next_a = if cycle % 2 == 0 {
            a.prepare(&[value.clone()], &mut summary)?
        } else {
            a.prepare::<Plain>(&[], &mut summary)?
        };
        let next_b = if cycle % 2 != 0 {
            b.prepare(&[value], &mut summary)?
        } else {
            b.prepare::<Plain>(&[], &mut summary)?
        };
        let checked = summary.check()?;
        assert!(checked.any());
        let next_a = next_a.finish(&checked)?;
        let next_b = next_b.finish(&checked)?;
        assert_eq!(status(next_a.delta())?, cycle % 2 == 0);
        assert_eq!(status(next_b.delta())?, cycle % 2 != 0);
        a = next_a;
        b = next_b;
    }
    exact(a.full().to_host()?, [(0,), (2,), (4,)]);
    exact(b.full().to_host()?, [(1,), (3,), (5,)]);

    let source = runtime.upload_owned_rows(
        &[(1i64, 1i64), (1, 2), (2, 2), (3, 4)],
        &mut dictionary,
        &[Codec::Plain; 2],
    )?;
    let keys = runtime.upload_owned_rows(
        &[(1i64, 1i64), (2, 7), (3, 4), (3, 4)],
        &mut dictionary,
        &[Codec::For; 2],
    )?;
    let no_keys =
        runtime.upload_owned_rows::<(i64, i64)>(&[], &mut dictionary, &[Codec::Plain; 2])?;
    for (left, right, expected) in [
        (vec![0, 1], vec![0, 1], vec![(1, 2), (2, 2)]),
        (vec![0, 0], vec![0, 1], vec![(2, 2), (3, 4)]),
        (vec![1], vec![0], vec![(3, 4)]),
        (vec![], vec![], vec![]),
    ] {
        let mut execution = runtime.execution();
        let s = execution.bind_columns(&source)?;
        let k = execution.bind_columns(&keys)?;
        let result = execution.anti_filter(s, k, &left, &right)?;
        let output: DeviceColumns<'_, (i64, i64)> = execution.into_columns(result, &[0, 1])?;
        exact(output.to_host()?, expected);
    }
    let resident = {
        let mut execution = runtime.execution();
        let s = execution.bind_columns(&source)?;
        let k = execution.bind_columns(&no_keys)?;
        let result = execution.anti_filter(s, k, &[], &[])?;
        execution.into_columns::<(i64, i64), Plain>(result, &[0, 1])?
    };
    exact(resident.to_host()?, [(1, 1), (1, 2), (2, 2), (3, 4)]);
    let owner = Arc::downgrade(&resident.storage().metadata);
    let retained = resident.clone();
    drop(resident);
    assert!(owner.upgrade().is_some());
    drop(retained);
    assert!(owner.upgrade().is_none());

    // Exercise the same exact view consumed by generated fused negative guards.
    let module = unsafe { checks::load(runtime.context())? };
    let mut execution = runtime.execution();
    let s = execution.bind_columns(&source)?;
    let k = execution.bind_columns(&keys)?;
    let set = execution.prepare_exact_membership(k, &[0, 1])?;
    let output = unsafe { Buffer::uninitialized(runtime.pool().clone(), 4)? };
    let launch = module.prepare_probe(LaunchConfig1D::new(1, 256, 0))?;
    unsafe {
        module.probe(
            runtime.stream(),
            &launch,
            execution.table_view(s)?,
            execution.exact_membership_view(set)?,
            output.ptr(),
        )?;
    }
    assert_eq!(output.to_host()?, [1, 0, 0, 1]);

    // Zero-column rows represent existence, including duplicate unit tuples.
    let units = runtime.upload_owned_rows(&[(), (), ()], &mut dictionary, &[])?;
    let units = Relation::from_columns(&units)?;
    exact(units.full().to_host()?, [()]);
    assert!(!status(prepare(&units, &[units.full().clone()])?.delta())?);

    // Dictionary equality is ID equality only inside a consistent dictionary.
    let mut dict_a = Dictionary::default();
    let mut dict_b = Dictionary::default();
    let text_a =
        runtime.upload_owned_rows(&[("alpha".to_owned(),)], &mut dict_a, &[Codec::Dictionary])?;
    let text_b =
        runtime.upload_owned_rows(&[("beta".to_owned(),)], &mut dict_b, &[Codec::Dictionary])?;
    let text = Relation::from_columns(&text_a)?;
    assert!(prepare(&text, &[text_b]).is_err());
    let compatible = runtime.upload_owned_rows(
        &[("beta".to_owned(),), ("alpha".to_owned(),)],
        &mut dict_a,
        &[Codec::Dictionary],
    )?;
    exact(
        prepare(&text, &[compatible])?.full().to_host()?,
        [("alpha".to_owned(),), ("beta".to_owned(),)],
    );
    // Install one common literal domain without touching resident columns.
    let mut common = dict_a.clone();
    assert_eq!(common.intern("alpha")?, 0);
    common.intern("gamma")?;
    let rebound = text_a.with_dictionary(Arc::new(common))?;
    assert!(Arc::ptr_eq(
        &text_a.storage().columns[0],
        &rebound.storage().columns[0]
    ));
    assert!(Arc::ptr_eq(
        &text_a.storage().metadata,
        &rebound.storage().metadata
    ));
    assert!(Arc::ptr_eq(text_a.pool(), rebound.pool()));
    let borrowed = {
        let mut execution = runtime.execution();
        let table = execution.bind_columns(&rebound)?;
        execution.into_columns::<(&str,), Plain>(table, &[0])?
    };
    exact(borrowed.to_host()?, [("alpha",)]);
    assert!(
        text_a
            .with_dictionary(Arc::new(Dictionary::default()))
            .is_err()
    );
    assert!(text_a.with_dictionary(Arc::new(dict_b.clone())).is_err());
    assert!(borrowed.with_dictionary(Arc::new(dict_a.clone())).is_err());
    let foreign_runtime = Runtime::new()?;
    let foreign =
        foreign_runtime.upload_owned_rows(&[(1i64, 2i64)], &mut dictionary, &[Codec::Plain; 2])?;
    assert!(
        prepare(&state, &[foreign])
            .err()
            .unwrap()
            .to_string()
            .contains("CQ_GPU_CONTEXT")
    );

    // Failed ancestors must not report successful empty convergence.
    let mut execution = runtime.execution();
    let launch = module.prepare_overflow(LaunchConfig1D::new(1, 256, 0))?;
    let bad = execution.execute_bounded(&[Bounds::singleton(0)], 1, |stream, output| {
        unsafe {
            module.overflow(stream, &launch, output)?;
        }
        Ok(())
    })?;
    let bad: DeviceColumns<'_, (i64,)> = execution.into_columns(bad, &[0])?;
    reset_host_wait_count();
    let mut failed_batch = RoundStatus::new(runtime.pool().clone());
    let _good_empty = b.prepare::<Plain>(&[], &mut failed_batch)?;
    let _failed = a.prepare(&[bad.clone()], &mut failed_batch)?;
    assert_eq!(
        host_wait_count(),
        0,
        "failed statuses stay resident until the round check"
    );
    assert!(
        failed_batch
            .check()
            .err()
            .unwrap()
            .to_string()
            .contains("CQ_GPU_OUTPUT_CAPACITY")
    );
    assert_eq!(
        host_wait_count(),
        1,
        "all member errors use the same readback"
    );
    datalog::reset_round_count();
    assert!(
        status(&bad)
            .unwrap_err()
            .to_string()
            .contains("CQ_GPU_OUTPUT_CAPACITY")
    );
    assert!(
        prepare(&a, &[bad])
            .err()
            .unwrap()
            .to_string()
            .contains("CQ_GPU_OUTPUT_CAPACITY")
    );
    assert_eq!(
        datalog::round_count(),
        0,
        "failed checks are not publications"
    );
    assert!(!status(empty.delta())?);
    assert_eq!(datalog::round_count(), 0, "empty batches are excluded");
    assert!(status(a.full())?);
    assert_eq!(datalog::round_count(), 1);
    datalog::reset_round_count();
    assert_eq!(datalog::round_count(), 0);
    println!(
        "datalog runtime oracle exact=true: 8192 multiword rows, forced 32-row hash collision chain, duplicate holes/projections, eight publications, immutable snapshots, alternating SCC deltas, exact/empty/repeated negative keys, fused membership, nullary existence, dictionaries, contexts, owner lifetime, overflow propagation"
    );
    Ok(())
}
