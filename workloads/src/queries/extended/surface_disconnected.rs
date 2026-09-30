crate::extended_query! {
    $; __extended_surface_disconnected; "surface_disconnected"; surface; 1; parameters {};
    {
struct SurfaceDisconnected;
relation R(x: i32, unused: i32);
relation S(y: i32);
relation answer(c0: i32);
answer(*difference) :- R(x, _), S(y), let difference: i32 = *x - *y,
    if *x > *y;
    }
}
