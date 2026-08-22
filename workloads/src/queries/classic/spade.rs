::mini_linq::workload_query! {
    pub struct SpadeProgram;
    relation R(c0: i32, c1: i32);
    relation S(c0: i32, c1: i32);
    relation T(c0: i32, c1: i32);
    relation H(c0: i32, c1: i32);

    spade(x, y, z, w) :- R(x, y), S(y, z), T(x, z), H(x, w).
}
