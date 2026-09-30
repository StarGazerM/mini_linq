crate::extended_query! {
    $; __extended_surface_compare; "surface_compare"; surface; 2; parameters {};
    {
struct SurfaceCompare;
relation R(x: i32, y: i32);
relation answer(c0: i32, c1: i32);
answer(*x, *y) :- R(x, y), if *x < *y, if *x >= -2;
    }
}
