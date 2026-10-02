crate::extended_query! {
    $; __extended_correlated_parameter; "correlated_parameter"; correlated; 1; parameters { let limit = 2; };
    {
struct CorrelatedParameter; relation A(x:i32,payload:i32); relation answer(x:i32);
answer(*x) :- A(x,payload), if *x > limit;
    }
}
