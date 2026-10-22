fn main() {
    cq_gpu_datalog::datalog! {
        struct Unsafe;
        relation E(x:i64) = (missing);
        relation R(x:i64);
        output R;
        R(*x) :- E(x), !E(y);
    };
}
