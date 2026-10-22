use crate::typed::{self, Source};

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let Calendar = &typed::load::<typed::Calendar>("typed_calendar_shipping", source)[..];
    let Shipment = &typed::load::<typed::Shipment>("typed_calendar_shipping", source)[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Calendar, Shipment)); }
        pub struct TypedCalendarShipping;
        relation Calendar(shipdate: cq_load::Date, label: String);
        relation Shipment(shipdate: cq_load::Date, orderkey: i64, price: cq_load::Decimal);

        calendar_shipping(label, orderkey, price, shipdate) :-
            Calendar(shipdate, label), Shipment(shipdate, orderkey, price);
    }
    .map(|(a, b, c, d)| vec![a.to_string(), b.to_string(), c.to_string(), d.to_string()])
    .collect();
    typed::check("typed_calendar_shipping", rows);
}
