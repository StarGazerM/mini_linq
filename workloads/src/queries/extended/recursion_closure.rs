crate::extended_query! {
    $; __extended_recursion_closure; "recursion_closure"; recursion; 2; parameters {  };
    {
struct RecursionClosure; relation Edge(x:i32,y:i32); relation reach(x:i32,y:i32);
reach(*x,*y) :- Edge(x,y), if *x >= 0;
reach(*x,*z) :- reach(x,y), Edge(y,z);
    }
}
