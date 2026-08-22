::mini_linq::workload_query! {
    pub struct Cycle4Program;
    relation R(c0: i32, c1: i32);
    relation S(c0: i32, c1: i32);
    relation T(c0: i32, c1: i32);
    relation U(c0: i32, c1: i32);

    cycle4(a, b, c, d) :- R(a, b), S(b, c), T(c, d), U(d, a).
}
