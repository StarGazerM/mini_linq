::mini_linq::workload_query! {
    pub struct BarbellProgram;
    relation R(c0: i32, c1: i32);
    relation S(c0: i32, c1: i32);
    relation T(c0: i32, c1: i32);
    relation Bridge(c0: i32, c1: i32);
    relation U(c0: i32, c1: i32);
    relation V(c0: i32, c1: i32);
    relation W(c0: i32, c1: i32);

    barbell(x, y, z, u, v, w) :-
        R(x, y),
        S(y, z),
        T(x, z),
        Bridge(x, u),
        U(u, v),
        V(v, w),
        W(u, w).
}
