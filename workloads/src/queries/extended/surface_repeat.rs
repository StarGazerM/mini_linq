crate::extended_query! {
    $; __extended_surface_repeat; "surface_repeat"; surface; 1; parameters {};
    {
struct SurfaceRepeat;
relation R(x: i32, y: i32);
relation answer(c0: i32);
answer(*x) :- R(x, x);
    }
}
