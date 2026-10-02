crate::extended_query! {
    $; __extended_union_two; "union_two"; union; 1; parameters {  };
    {
struct UnionTwo;
relation A(x:i32); relation B(x:i32); relation answer(x:i32);
answer(*x) :- A(x), if *x >= 0;
answer(*x) :- B(x), if *x >= 0;
    }
}
