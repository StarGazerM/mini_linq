use crate::typed::{self, Source};

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let Customer = &typed::load::<typed::Customer>("typed_customer_orders", source)[..];
    let Orders = &typed::load::<typed::Orders>("typed_customer_orders", source)[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Customer, Orders)); }
        pub struct TypedCustomerOrders;
        relation Customer(customerkey: i64, name: String);
        relation Orders(customerkey: i64, orderkey: i64, orderdate: cq_load::Date, total: cq_load::Decimal);

        customer_orders(name, orderkey, orderdate, total) :-
            Customer(customerkey, name), Orders(customerkey, orderkey, orderdate, total);
    }.map(|(a,b,c,d)| vec![a.to_string(), b.to_string(), c.to_string(), d.to_string()]).collect();
    typed::check("typed_customer_orders", rows);
}
