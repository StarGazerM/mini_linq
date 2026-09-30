crate::extended_query! {
    $; __extended_negation_key; "negation_key"; negation; 1; parameters {  };
    {
struct NegationKey;
relation A(x:i32,payload:i32); relation S(x:i32,y:i32); relation answer(x:i32);
answer(*x) :- A(x,payload), neg!{ S(x, _) };
    }
}
