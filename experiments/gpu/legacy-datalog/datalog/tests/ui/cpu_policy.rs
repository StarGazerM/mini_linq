fn main() {
    cq_datalog::passes::stratify! {
        pipeline {
            ::cq_datalog::passes::incremental,
            ::cq_datalog::passes::storage,
            ::cq_gpu_datalog::__scheduled,
        };
        struct Policy;
        relation E(x:i64) = (missing);
        relation R(x:i64);
        output R;
        R(*x) :- E(x);
    };
}
