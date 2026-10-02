crate::extended_query! {
    $; __extended_surface_let_chain; "surface_let_chain"; surface; 2; parameters {};
    {
struct SurfaceLetChain;
relation R(x: i32, y: i32);
relation answer(c0: i32, c1: i32);
answer(*sum, *shifted) :- R(x, y), let sum: i32 = *x + *y,
    let shifted: i32 = *sum * 2 - 1, if *shifted > 0;
    }
}
