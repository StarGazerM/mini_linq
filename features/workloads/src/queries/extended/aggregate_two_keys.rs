crate::extended_query! {
    $; __extended_aggregate_two_keys; "aggregate_two_keys"; aggregate; 4; parameters {  };
    {
by (d,k); n = count(); s = sum(a);
struct AggregateTwoKeys; relation Sale(d:i32,k:i32,id:i32,a:i32);
rows(d,k,id,a) :- Sale(d,k,id,a), if *d > 0;
    }
}
