fn main() {
    let grouped = std::collections::HashMap::<i64, Vec<i64>>::new();
    cq_gpu_datalog::datalog! {
        struct Grouped;
        relation E(x:i64,y:i64) = (&grouped);
        output E;
    };
}
