use crate::typed::{self, Source};

mini_linq::interned_relation_loaders! {
    relation Calendar(shipdate: cq_load::Date, label: cq_load::StringCode);
}

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let mut dictionaries = cq_load::Dictionaries::default();
    let Calendar = &<relation_loader!(Calendar)>::intern(
        typed::load::<typed::Calendar>("typed_calendar_shipping", source),
        &mut dictionaries,
    )[..];
    let Shipment = &typed::load::<typed::Shipment>("typed_calendar_shipping", source)[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Calendar, Shipment)); }
        pub struct TypedCalendarShipping;
        relation Calendar(shipdate: cq_load::Date, label: cq_load::StringCode);
        relation Shipment(shipdate: cq_load::Date, orderkey: i64, price: cq_load::Decimal);

        calendar_shipping(label, orderkey, price, shipdate) :-
            Calendar(shipdate, label), Shipment(shipdate, orderkey, price);
    }
    .map(|(a, b, c, d)| vec![
        // DECODE P: publish the calendar label in the typed calendar/shipping answer.
        dictionaries.publish("label", *a),
        b.to_string(), c.to_string(), d.to_string(),
    ])
    .collect();
    typed::check("typed_calendar_shipping", rows);
}
