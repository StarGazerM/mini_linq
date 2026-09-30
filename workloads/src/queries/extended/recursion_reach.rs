crate::extended_query! {
    $; __extended_recursion_reach; "recursion_reach"; recursion; 1; parameters {  };
    {
struct RecursionReach; relation Seed(x:i32); relation Edge(x:i32,y:i32); relation reach(x:i32);
reach(*y) :- Seed(x), Edge(x,y);
reach(*z) :- reach(y), Edge(y,z);
    }
}
