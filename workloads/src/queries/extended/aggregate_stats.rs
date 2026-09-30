crate::extended_query! {
    $; __extended_aggregate_stats; "aggregate_stats"; aggregate; 5; parameters {  };
    {
by (d); n = count(); s = sum(a); lo = min(a); hi = max(a);
struct AggregateStats; relation Sale(d:i32,id:i32,a:i32);
rows(d,id,a) :- Sale(d,id,a), if *d > 0;
    }
}
