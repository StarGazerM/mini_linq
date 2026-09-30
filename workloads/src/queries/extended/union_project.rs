crate::extended_query! {
    $; __extended_union_project; "union_project"; union; 1; parameters {  };
    {
struct UnionProject;
relation A(x:i32,y:i32); relation B(x:i32,y:i32); relation answer(x:i32);
answer(*x) :- A(x,y), if *y > 0; answer(*x) :- B(x,y), if *y > 0;
    }
}
