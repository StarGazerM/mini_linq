::mini_linq::workload_query! {
    pub struct TypedCalendarShipping;
    relation Calendar(shipdate: Date, label: String);
    relation Shipment(shipdate: Date, orderkey: i64, price: Decimal);

    calendar_shipping(label, orderkey, price, shipdate) :-
        Calendar(shipdate, label), Shipment(shipdate, orderkey, price);
}
