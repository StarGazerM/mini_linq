crate::extended_query! {
    $; __extended_surface_constant; "surface_constant"; surface; 1; parameters {};
    {
struct SurfaceConstant;
relation R(x: i32, tag: i32);
relation answer(c0: i32);
answer(*x) :- R(x, 7);
    }
}
