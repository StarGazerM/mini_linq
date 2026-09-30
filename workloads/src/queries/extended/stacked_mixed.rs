crate::extended_query! {
    $; __extended_stacked_mixed; "stacked_mixed"; stacked; 1; parameters {  };
    {
struct StackedMixed; relation A(x:i32); relation B(x:i32); relation Blocked(x:i32,y:i32); relation Hidden(x:i32,y:i32); relation answer(x:i32);
answer(*x) :- A(x), neg!{ Blocked(x, _) }, neg!{ Hidden(x, _) };
answer(*x) :- B(x), neg!{ Hidden(x, _) };
    }
}
