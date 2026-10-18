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
}
