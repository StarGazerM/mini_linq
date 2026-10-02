crate::extended_query! {
    $; __extended_surface_boolean_guard; "surface_boolean_guard"; surface; 2; parameters {};
    {
struct SurfaceBooleanGuard;
relation R(x: i32, y: i32);
relation answer(c0: i32, c1: i32);
answer(*x, *y) :- R(x, y), if (*x < 0 || *y > 2) && !(*x == *y);
    }
}
