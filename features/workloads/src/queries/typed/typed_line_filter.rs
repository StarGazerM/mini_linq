use crate::typed::{self, Source};

mini_linq::interned_relation_loaders! {
    relation Lineitem(orderkey: i64, linenumber: i32, price: cq_load::Decimal, shipdate: cq_load::Date, comment: cq_load::StringCode);
}

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let mut dictionaries = cq_load::Dictionaries::default();
    let Lineitem = &<relation_loader!(Lineitem)>::intern(
        typed::load::<typed::Lineitem>("typed_line_filter", source),
        &mut dictionaries,
    )[..];
    let Limits = &typed::load::<typed::Limits>("typed_line_filter", source)[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Lineitem, Limits)); }
        pub struct TypedLineFilter;
        relation Lineitem(orderkey: i64, linenumber: i32, price: cq_load::Decimal, shipdate: cq_load::Date, comment: cq_load::StringCode);
        relation Limits(minimum: cq_load::Decimal, earliest: cq_load::Date);

        line_filter(orderkey, price, shipdate, comment) :-
            Lineitem(orderkey, linenumber, price, shipdate, comment),
            Limits(minimum, earliest), where price >= minimum, where shipdate >= earliest,
            where linenumber > 1;
    }.map(|(a,b,c,d)| vec![
        a.to_string(), b.to_string(), c.to_string(),
        // DECODE P: publish the line comment in the typed line-filter answer.
        dictionaries.publish("comment", *d),
    ]).collect();
    typed::check("typed_line_filter", rows);
}
