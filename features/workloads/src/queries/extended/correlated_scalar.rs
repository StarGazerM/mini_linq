crate::extended_query! {
    $; __extended_correlated_scalar; "correlated_scalar"; correlated; 2; parameters { let limit = 2; };
    {
struct CorrelatedScalar; relation A(x:i32); relation S(x:i32,y:i32,id:i32); relation answer(x:i32,v:i32);
answer(*x,v) :- A(x), scalar!{ v:i32=(0); select (*y); S(x,y,id) }, if *v > limit;
    }
}
