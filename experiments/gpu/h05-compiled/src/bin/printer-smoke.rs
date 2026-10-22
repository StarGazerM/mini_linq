#![recursion_limit = "1024"]

// Run with the CUDA toolchain from experiments/gpu:
// cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin printer-smoke
// target/release/printer-smoke
fn main() {
    let pool = cq_gpu_runtime::Pool::new(0, 0, 0).expect("GPU pool");
    let mut dictionary = cq_gpu_runtime::Dictionary::default();
    for input in [vec![(3i64,), (1,), (3,), (-7,), (0,)], Vec::new()] {
        let resident = cq_gpu_runtime::DeviceColumns::from_rows(
            pool.clone(),
            &input,
            &mut dictionary,
            &[cq_gpu_runtime::Codec::Plain],
        )
        .expect("GPU smoke upload");
        // The decomposition explicitly publishes the resident source before
        // dispatch. Grading validates these placements; it does not plan them.
        let answer: cq_gpu_runtime::DeviceColumns<'_, (i64,)> = cq_sequence_rewrites::output! {
            arguments { count };
            pipeline {
                ::cq_sequence_rewrites::grade(gpu, source at host, result at device),
                ::cq_gpu_sequence::machine_grade,
                ::cq_gpu_sequence::translate
            };
            physical {
                relation Input(value:i64) = (&resident);
                scan = pipe rename Input@0 {value -> value};
                answer = pipe canon project scan keep {value};
                return answer as answer(value);
            }
            structures {
                scan host scan = ::cq_gpu_runtime::DeviceColumns<'_, (i64,)> [];
                answer canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                source = store rows scan;
                result = store canon map answer flatten tile 256 source;
                output count;
            }
        };
        let mut actual = answer.to_host().expect("GPU smoke download");
        let mut expected: Vec<_> = mini_linq::compile! {
            pipeline {}
            struct PullSmoke;
            relation Input(value:i64) = (&input);
            answer(value) :- Input(value);
        }
        .map(|(value,)| (*value,))
        .collect();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected, "GPU smoke differs from Sequence pull");
        println!(
            "smoke exact=true input_rows={} answer_rows={}",
            input.len(),
            actual.len()
        );
    }
    // TODO(Sequence GPU): restore the i64 duplicate-fanout/fresh-execution
    // oracle when its recorded range payload is supported. The backend rejects
    // that original plan with:
    // "T-Gpu: range payload needs a recorded u32/i32 dense code".
    // Do not report that legacy oracle as passing or substitute a different plan.
}
