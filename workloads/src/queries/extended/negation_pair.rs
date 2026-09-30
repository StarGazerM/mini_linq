crate::extended_query! {
    $; __extended_negation_pair; "negation_pair"; negation; 2; parameters {  };
    {
struct NegationPair;
relation A(x:i32,y:i32); relation S(x:i32,y:i32); relation answer(x:i32,y:i32);
answer(*x,*y) :- A(x,y), neg!{ S(x, y) };
    }
}
