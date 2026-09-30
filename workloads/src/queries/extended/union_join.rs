crate::extended_query! {
    $; __extended_union_join; "union_join"; union; 1; parameters {  };
    {
struct UnionJoin;
relation A(x:i32); relation B(x:i32); relation C(x:i32); relation D(x:i32); relation answer(x:i32);
answer(*x) :- A(x), B(x); answer(*x) :- C(x), D(x);
    }
}
