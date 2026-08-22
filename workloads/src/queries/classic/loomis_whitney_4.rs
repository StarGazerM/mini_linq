::mini_linq::workload_query! {
    pub struct LoomisWhitney4Program;
    relation ABC(c0: i32, c1: i32, c2: i32);
    relation BCD(c0: i32, c1: i32, c2: i32);
    relation ACD(c0: i32, c1: i32, c2: i32);
    relation ABD(c0: i32, c1: i32, c2: i32);

    lw4(a, b, c, d) :-
        ABC(a, b, c),
        BCD(b, c, d),
        ACD(a, c, d),
        ABD(a, b, d).
}
