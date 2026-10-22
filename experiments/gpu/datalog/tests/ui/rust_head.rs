#![recursion_limit = "1024"]
fn unsupported(input: &cq_gpu_runtime::DeviceColumns<'_, (i64,)>) {
    cq_gpu_datalog::datalog! {
        struct Opaque;
        relation E(x:i64) = (input);
        relation R(x:i64);
        output R;
        R(x.wrapping_add(1)) :- E(x);
    };
}
fn main() {}
