crate::extended_query! {
    $; __extended_correlated_exists; "correlated_exists"; correlated; 1; parameters { let limit = 2; };
    {
struct CorrelatedExists; relation A(x:i32); relation S(x:i32,y:i32); relation answer(x:i32);
answer(*x) :- A(x), exists!{ S(x,y), if *y > limit };
    }
}
