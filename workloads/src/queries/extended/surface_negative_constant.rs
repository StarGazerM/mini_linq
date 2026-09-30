crate::extended_query! {
    $; __extended_surface_negative_constant; "surface_negative_constant"; surface; 1; parameters {};
    {
struct SurfaceNegativeConstant;
relation R(tag: i32, x: i32);
relation answer(c0: i32);
answer(*x) :- R(-2, x);
    }
}
