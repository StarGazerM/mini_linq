::mini_linq::workload_query! {
    pub struct CorpusTriangleProgram;
    relation R(c0: i32, c1: i32);
    relation S(c0: i32, c1: i32);
    relation T(c0: i32, c1: i32);

    triangle(x, y, z) :- R(x, y), S(y, z), T(z, x).
}
