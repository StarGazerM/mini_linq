#![recursion_limit = "1024"]

// Run with the CUDA toolchain from experiments/gpu:
// cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin printer-oracle
// target/release/printer-oracle
// target/release/printer-oracle --projection
//
// These are authored Sequence decompositions, not inferred GPU schedules.
// Unsupported legacy compiler oracles are listed below with their rejection;
// none contributes to the successful coverage messages.

fn canonical_projection() {
    let pool = cq_gpu_runtime::Pool::new(0, 0, 0).expect("GPU pool");
    let mut dictionary = cq_gpu_runtime::Dictionary::default();
    macro_rules! projection {
        ($resident:expr) => {
            cq_sequence_rewrites::output! {
                arguments { count };
                pipeline {
                    ::cq_sequence_rewrites::grade(gpu, source at host, result at device),
                    ::cq_gpu_sequence::machine_grade,
                    ::cq_gpu_sequence::translate
                };
                physical {
                    relation Edges(a:i64,b:i64) = ($resident);
                    scan = pipe rename Edges@0 {a -> x,b -> y};
                    output = pipe canon project scan keep {x};
                    return output as answer(x);
                }
                structures {
                    scan host scan = ::cq_gpu_runtime::DeviceColumns<'_, (i64,i64)> [];
                    output canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                }
                sequence {
                    source = store rows scan;
                    result = store canon map output flatten tile 256 source;
                    output count;
                }
            }
        };
    }
    // Duplicate inputs remain valid; canon must deduplicate the projection.
    // Legacy host-set certification lives in the isolated runtime-oracle.
    for rows in [
        vec![],
        vec![(1i64, 2i64)],
        vec![(1, 2), (1, 3), (2, 1)],
        vec![(2, 1), (1, 3), (1, 2)],
        vec![(1, 2), (1, 2), (2, 1)],
        vec![(1, 2), (2, 1), (1, 2)],
        vec![(3, 1), (1, 2), (2, 1), (1, 2)],
        vec![(0, 1), (0, 2), (0, 3), (0, 1)],
    ] {
        let resident = cq_gpu_runtime::DeviceColumns::from_rows(
            pool.clone(),
            &rows,
            &mut dictionary,
            &[cq_gpu_runtime::Codec::Plain; 2],
        )
        .unwrap();
        let mut expected: Vec<_> = rows.iter().map(|row| (row.0,)).collect();
        expected.sort_unstable();
        expected.dedup();
        let answer: cq_gpu_runtime::DeviceColumns<'_, (i64,)> = projection!(&resident);
        assert_eq!(answer.checked_len().unwrap(), expected.len());
        let mut actual = answer.to_host().expect("resident projection download");
        actual.sort_unstable();
        assert_eq!(
            actual, expected,
            "full-heading set does not certify a projection"
        );
    }
    println!("sequence-projection exact=true empty=true sorted=true unsorted=true duplicates=true");
}

fn repeated_head_projection() {
    let rows = [(1i64, 2i64), (1, 3), (2, 4), (1, 2)];
    let pool = cq_gpu_runtime::Pool::new(0, 0, 0).unwrap();
    let resident = cq_gpu_runtime::DeviceColumns::from_rows(
        pool,
        &rows,
        &mut cq_gpu_runtime::Dictionary::default(),
        &[cq_gpu_runtime::Codec::Plain; 2],
    )
    .unwrap();
    let answer: cq_gpu_runtime::DeviceColumns<'_, (i64, i64)> = cq_sequence_rewrites::output! {
        arguments { count };
        pipeline {
            ::cq_sequence_rewrites::grade(gpu, source at host, result at device),
            ::cq_gpu_sequence::machine_grade,
            ::cq_gpu_sequence::translate
        };
        physical {
            relation R(a:i64,b:i64) = (&resident);
            scan = pipe rename R@0 {a -> x,b -> y};
            output = pipe canon project scan keep {x};
            return output as answer(x,x);
        }
        structures {
            scan host scan = ::cq_gpu_runtime::DeviceColumns<'_, (i64,i64)> [];
            output canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
        }
        sequence {
            source = store rows scan;
            result = store canon map output flatten tile 256 source;
            output count;
        }
    };
    let mut actual = answer.to_host().expect("repeated lossy head");
    actual.sort_unstable();
    assert_eq!(actual, [(1, 1), (2, 2)]);
    println!("sequence-repeated-head exact=true projection=true duplicates=true");
}

// The legacy chained_answers, composite_hash_collisions, leveled_host_reads,
// and witness/join-filter expansion oracles have not migrated to authored
// Sequence coverage. Native i64 range payloads are supported, but the legacy
// Fused/Compacting/resume/provider-specific assertions are not assertions about
// PackedCanon or RangeProvider and must not be reported as such.
//
// TODO(Sequence GPU): restore the original two-column typed range filter:
// "T-Gpu: recorded range build selection must be a half-open interval on one source column".
// Restore closed_scalar_captures only when its grading rejection is addressed:
// "BE-Partial: opaque Physical Rust must be evaluated at host before dispatch".
// Even the #[total] Rust predicate is rejected by translation:
// "T-Gpu: opaque Rust map is not #[device]-compatible; evaluate it at host before dispatch".
//
// TODO(Sequence GPU): restore the original (i64::MIN, i64::MAX, String)
// multiword output oracle when the backend supports its key width. Exact
// execution rejection: "packed Sequence equality key exceeds 64 bits".

fn main() {
    if std::env::args().any(|argument| argument == "--leveled") {
        panic!("leveled_host_reads is a legacy maintained-index oracle, not an authored Sequence program");
    }
    canonical_projection();
    if std::env::args().any(|argument| argument == "--projection") {
        return;
    }
    repeated_head_projection();
}
