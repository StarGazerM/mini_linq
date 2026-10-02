crate::extended_query! {
    $; __extended_stacked_antijoin; "stacked_antijoin"; stacked; 1; parameters {  };
    {
struct StackedAntijoin; relation A(x:i32); relation B(x:i32); relation Blocked(x:i32,y:i32); relation answer(x:i32);
answer(*x) :- A(x), neg!{ Blocked(x, _) };
answer(*x) :- B(x), neg!{ Blocked(x, _) };
    }
}
