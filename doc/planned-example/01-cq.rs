::mini_linq::planned_query! {
    pub struct PlannedExample; relation R(a : i32, b : i32); relation S(b : i32, c :
    i32); answer(a) : - R(a, b), S(b, c), if (c > 0i32).
}
