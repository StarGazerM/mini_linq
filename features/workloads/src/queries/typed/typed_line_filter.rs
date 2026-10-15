use crate::typed::{self, Source};

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let Lineitem = &typed::load::<typed::Lineitem>("typed_line_filter", source)[..];
    let Limits = &typed::load::<typed::Limits>("typed_line_filter", source)[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Lineitem, Limits)); }
        pub struct TypedLineFilter;
        relation Lineitem(orderkey: i64, linenumber: i32, price: cq_load::Decimal, shipdate: cq_load::Date, comment: String);
        relation Limits(minimum: cq_load::Decimal, earliest: cq_load::Date);

        line_filter(orderkey, price, shipdate, comment) :-
            Lineitem(orderkey, linenumber, price, shipdate, comment),
            Limits(minimum, earliest), if price >= minimum, if shipdate >= earliest,
            if *linenumber > 1;
    }.map(|(a,b,c,d)| vec![a.to_string(), b.to_string(), c.to_string(), d.to_string()]).collect();
    typed::check("typed_line_filter", rows);
}
