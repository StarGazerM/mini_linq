crate::extended_query! {
    $; __extended_negation_twice; "negation_twice"; negation; 1; parameters {  };
    {
struct NegationTwice;
relation A(x:i32); relation B(x:i32,y:i32); relation C(x:i32,y:i32); relation answer(x:i32);
answer(*x) :- A(x), neg!{ B(x, _) }, neg!{ C(x, _) };
    }
}
