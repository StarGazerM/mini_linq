fn main() {
    cq_gpu_datalog::datalog! {
        struct Policy;
        relation E(x:i64) = (missing);
        relation R(x:i64);
        output R;
        R(*x) :- E(x);
        strata { component [R]; }
        seminaive {
            insert_new hash_set_all_columns;
            negation_keys hash_set_borrowed;
            delta R = Recent;
            layout E [0] key 0;
            variant 0 seed { R(*x) :- E(x); }
        }
    };
}
