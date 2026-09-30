crate::extended_query! {
    $; __extended_surface_let_sum; "surface_let_sum"; surface; 2; parameters {};
    {
struct SurfaceLetSum;
relation R(x: i32, y: i32);
relation answer(c0: i32, c1: i32);
answer(*x, *total) :- R(x, y), let total: i32 = *x + *y, if *total > 0;
    }
}
