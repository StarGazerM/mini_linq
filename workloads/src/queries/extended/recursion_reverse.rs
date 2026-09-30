crate::extended_query! {
    $; __extended_recursion_reverse; "recursion_reverse"; recursion; 1; parameters {  };
    {
struct RecursionReverse; relation Goal(x:i32); relation Edge(x:i32,y:i32); relation reach(x:i32);
reach(*y) :- Goal(x), Edge(y,x);
reach(*z) :- reach(y), Edge(z,y);
    }
}
