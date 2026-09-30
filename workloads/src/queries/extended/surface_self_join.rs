crate::extended_query! {
    $; __extended_surface_self_join; "surface_self_join"; surface; 2; parameters {};
    {
struct SurfaceSelfJoin;
relation R(x: i32, y: i32);
relation answer(c0: i32, c1: i32);
answer(*x, *z + 1) :- R(x, y), R(y, z), if *x < *z;
    }
}
