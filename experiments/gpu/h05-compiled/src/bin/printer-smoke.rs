#![recursion_limit = "1024"]

fn main() {
    for input in [vec![(3i64,), (1,), (3,), (-7,), (0,)], Vec::new()] {
        let mut actual: Vec<_> = mini_linq::compile! {
            pipeline {
                structures = [::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)];
                print = ::cq_gpu::print;
            }
            struct GpuSmoke;
            relation Input(value:i64) = (&input);
            answer(value) :- Input(value);
        }
        .collect();
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
        assert_eq!(actual, expected, "GPU smoke differs from pull printer");
        println!(
            "smoke exact=true input_rows={} answer_rows={}",
            input.len(),
            actual.len()
        );
    }
    // Unstaged traversal must exhaust duplicates rather than interpreting
    // Physical's `first` choice as a uniqueness/cardinality certificate.
    let left = vec![(7i64,), (9,)];
    let mut right: Vec<_> = (0..4097i64).map(|value| (7i64, value)).collect();
    for delta in [0i64, 10_000] {
        for (index, row) in right.iter_mut().enumerate() {
            row.1 = index as i64 + delta;
        }
        let mut actual: Vec<_> = mini_linq::compile! {
            pipeline {
                plan = ::cq_planners::order(le);
                physical = [::cq_rewrites::build_host_reads];
                structures = [::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups,
                    ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)];
                print = ::cq_gpu::print;
            }
            struct GpuFanoutSmoke;
            relation Left(key:i64) = (&left);
            relation Right(key:i64,value:i64) = (&right);
            answer(value) :- Left(key), Right(key,value);
        }
        .collect();
        actual.sort_unstable();
        let expected: Vec<_> = (delta..delta + 4097).map(|value| (value,)).collect();
        assert_eq!(
            actual, expected,
            "unstaged fanout or fresh execution failed"
        );
        println!(
            "unstaged-fanout exact=true rows={} delta={delta}",
            actual.len()
        );
    }
}
