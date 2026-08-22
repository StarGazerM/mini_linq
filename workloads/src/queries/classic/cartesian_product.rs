::mini_linq::workload_query! {
    pub struct CartesianProgram;
    relation A(c0: i32);
    relation B(c0: i32, c1: i32);

    product(x, y, z) :- A(x), B(y, z).
}
