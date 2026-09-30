crate::extended_query! {
    $; __extended_surface_computed_head; "surface_computed_head"; surface; 2; parameters {};
    {
struct SurfaceComputedHead;
relation R(x: i32, y: i32);
relation answer(c0: i32, c1: i32);
answer(*x - *y, *x * 2 + *y) :- R(x, y), if *x >= 0;
    }
}
