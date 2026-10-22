#![recursion_limit = "1024"]
fn unsupported(input: &cq_gpu_runtime::DeviceColumns<'_, (i64,)>) {
    cq_gpu_datalog::datalog! {
        pipeline { print = ::cq_gpu::print(completion(query)); }
        struct Copy;
        relation E(x:i64) = (input);
        relation R(x:i64);
        output R;
        R(*x) :- E(x);
    };
}
fn main() {}
