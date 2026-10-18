mod common;
use common::Edges;
use cuda_core::{CudaContext, DeviceBuffer};
use gpu_blocked_run::{Buffer, MemoryResource};
use gpu_triangles::{ptr, Error, Graph, Triple, Workspace};
use rccl_memory::{CudaMemoryResource, PoolMemoryResource};

fn check(
    name: &str,
    rows: Vec<(i64, i64)>,
    materialize: bool,
    require_steals: bool,
) -> Result<(), Error> {
    let edges = Edges::canonical(rows);
    let expected = edges.cpu_count();
    let ctx = CudaContext::new(0)?;
    let stream = ctx.default_stream();
    let keys = DeviceBuffer::from_host(&stream, &edges.sources)?;
    let neighbors = DeviceBuffer::from_host(&stream, &edges.targets)?;
    let extra = if materialize {
        expected as usize * 24
    } else {
        0
    };
    let reserved = edges.pool_bytes(extra + (4 << 20));
    let memory = MemoryResource::new(PoolMemoryResource::new(
        CudaMemoryResource::new(ctx.clone()),
        reserved,
        reserved,
    )?);
    let graph = Graph::build(&ctx, &stream, &memory, &keys, &neighbors, edges.domain())?;
    let output = Buffer::<Triple>::zeroed_in(
        &memory,
        &stream,
        if materialize { expected as usize } else { 0 },
    )?;
    let mut total_steals = 0;
    let mut total_full = 0;
    // Zero capacity exercises the pool-full depth-first path, one slot forces
    // publication contention, and 64 slots exercises reuse with many owners.
    for capacity in [0, 1, 64] {
        let workspace = Workspace::new(&memory, &stream, capacity)?;
        for k in [0, 1, 8] {
            for _ in 0..3 {
                workspace.launch(
                    &stream,
                    graph.view(),
                    k,
                    142,
                    if materialize {
                        ptr(&output)
                    } else {
                        core::ptr::null_mut()
                    },
                    expected,
                    0,
                )?;
                let stats = workspace.result()?;
                assert_eq!(stats.count, expected, "{name}: capacity={capacity} K={k}");
                assert_eq!(stats.remaining, 0, "{name}: termination counter");
                total_steals += stats.steals;
                total_full += stats.pool_full;
                let slots = workspace.pool.to_host_vec()?;
                for slot in slots.chunks(gpu_triangles::SLOT_WORDS) {
                    assert_eq!(slot[0], 0, "{name}: live slot after termination");
                }
                if materialize {
                    let mut actual = output.to_host_vec()?;
                    actual.sort_unstable();
                    assert!(
                        actual.windows(2).all(|w| w[0] != w[1]),
                        "{name}: duplicate output"
                    );
                    for Triple { a, b, c } in actual {
                        assert!(edges.targets[edges.range(a)].binary_search(&b).is_ok());
                        assert!(edges.targets[edges.range(b)].binary_search(&c).is_ok());
                        assert!(edges.targets[edges.range(a)].binary_search(&c).is_ok());
                    }
                }
            }
        }
    }
    if materialize && expected > 0 {
        let workspace = Workspace::new(&memory, &stream, 1)?;
        workspace.launch(&stream, graph.view(), 1, 142, ptr(&output), expected - 1, 0)?;
        assert!(
            workspace.result().is_err(),
            "{name}: output overflow must be reported"
        );
    }
    if require_steals {
        assert!(total_steals > 0, "{name}: no steal exercised");
        assert!(total_full > 0, "{name}: full fallback not exercised");
    }
    println!("{name}: edges={} expected={expected} steals={total_steals} pool_full={total_full} exact=true",edges.sources.len());
    Ok(())
}
fn main() -> Result<(), Error> {
    check("empty", vec![], true, false)?;
    check(
        "self_loops_duplicates",
        vec![(0, 0), (0, 0), (0, 1), (1, 0), (1, 1), (2, 0)],
        true,
        false,
    )?;
    check(
        "sparse_i64_fences",
        vec![
            (i64::MIN, i64::MAX),
            (i64::MIN, 5),
            (5, i64::MAX),
            (i64::MAX, i64::MAX),
        ],
        true,
        false,
    )?;
    let mut sparse = Vec::new();
    for b in 1..=8193 {
        sparse.push((i64::MIN, b));
        sparse.push((0, b));
    }
    sparse.extend([(i64::MIN, 0), (i64::MAX, 0), (i64::MAX, i64::MAX)]);
    check("sparse_repeated_fences", sparse, true, true)?;
    let mut state = 31u64;
    let mut random = Vec::new();
    for a in 0..113 {
        for b in 0..113 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            if state >> 60 < 3 {
                random.push((a, b));
            }
        }
    }
    check("random_partial_tile", random, true, false)?;
    let mut clique = Vec::new();
    for a in 0..129 {
        for b in 0..129 {
            clique.push((a, b));
        }
    }
    check("clique_repeated_offsets", clique, true, true)?;
    // A geometric degree tail: the first node has exactly one million distinct
    // outgoing neighbors. High-degree hub-to-hub edges create >T chunks even
    // when probing the smaller range, rather than a star with zero triangles.
    let mut power = Vec::new();
    for b in 1..=1_000_000 {
        power.push((0, b));
    }
    for a in 1..20 {
        let degree = 1_000_000i64 >> (a - 1);
        for b in (a + 1)..=(a + degree) {
            power.push((a, b));
        }
    }
    check("power_law_degree_1000000", power, false, true)?;
    println!(
        "all intersection, materialization, termination, saturation and slot-reuse oracles passed"
    );
    Ok(())
}
