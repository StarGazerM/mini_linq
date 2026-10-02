crate::extended_query! {
    $; __extended_surface_wildcards; "surface_wildcards"; surface; 1; parameters {};
    {
struct SurfaceWildcards;
relation R(x: i32, y: i32, z: i32);
relation answer(c0: i32);
answer(*x) :- R(x, _, _), if *x > 0;
    }
}
