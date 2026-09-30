crate::extended_query! {
    $; __extended_aggregate_join; "aggregate_join"; aggregate; 3; parameters {  };
    {
by (d); s = sum(a); lo = min(a);
struct AggregateJoin; relation Sale(d:i32,id:i32,a:i32); relation Allowed(d:i32);
rows(d,id,a) :- Sale(d,id,a), Allowed(d);
    }
}
