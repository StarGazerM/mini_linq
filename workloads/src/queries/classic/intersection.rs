::mini_linq::workload_query! {
    pub struct UnaryIntersectionProgram;
    relation Left(c0: i32);
    relation Right(c0: i32);

    intersection(x) :- Left(x), Right(x).
}
