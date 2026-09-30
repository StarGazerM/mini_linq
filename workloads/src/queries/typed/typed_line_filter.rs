::mini_linq::workload_query! {
    pub struct TypedLineFilter;
    relation Lineitem(orderkey: i64, linenumber: i32, price: Decimal, shipdate: Date, comment: String);
    relation Limits(minimum: Decimal, earliest: Date);

    line_filter(orderkey, price, shipdate, comment) :-
        Lineitem(orderkey, linenumber, price, shipdate, comment),
        Limits(minimum, earliest), if price >= minimum, if shipdate >= earliest,
        if *linenumber > 1;
}
