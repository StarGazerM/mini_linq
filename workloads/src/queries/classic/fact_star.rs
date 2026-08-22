::mini_linq::workload_query! {
    pub struct FactStarProgram;
    relation Fact(c0: i32, c1: i32, c2: i32);
    relation DimA(c0: i32, c1: i32);
    relation DimB(c0: i32, c1: i32);
    relation DimC(c0: i32, c1: i32);

    fact_star(k1, k2, k3, a, b, c) :-
        Fact(k1, k2, k3),
        DimA(k1, a),
        DimB(k2, b),
        DimC(k3, c).
}
