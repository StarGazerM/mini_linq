::mini_linq::workload_query! {
    pub struct OrientedChainProgram;
    relation Edge(c0: i32, c1: i32);

    chain(a, d) :- Edge(a, b), Edge(b, c), Edge(d, c).
}
