crate::extended_query! {
    $; __extended_surface_arithmetic_guard; "surface_arithmetic_guard"; surface; 2; parameters {};
    {
struct SurfaceArithmeticGuard;
relation R(x: i32, y: i32);
relation answer(c0: i32, c1: i32);
answer(*x, *y) :- R(x, y), if *x + 2 * *y >= 3;
    }
}
