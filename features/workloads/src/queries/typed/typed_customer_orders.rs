use crate::typed::{self, Source};

mini_linq::interned_relation_loaders! {
    relation Customer(customerkey: i64, name: cq_load::StringCode);
}

#[allow(non_snake_case)]
pub fn run(source: Source) {
    let mut dictionaries = cq_load::Dictionaries::default();
    let Customer = &<relation_loader!(Customer)>::intern(
        typed::load::<typed::Customer>("typed_customer_orders", source),
        &mut dictionaries,
    )[..];
    let Orders = &typed::load::<typed::Orders>("typed_customer_orders", source)[..];
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(Customer, Orders)); }
        pub struct TypedCustomerOrders;
        relation Customer(customerkey: i64, name: cq_load::StringCode);
        relation Orders(customerkey: i64, orderkey: i64, orderdate: cq_load::Date, total: cq_load::Decimal);

        customer_orders(name, orderkey, orderdate, total) :-
            Customer(customerkey, name), Orders(customerkey, orderkey, orderdate, total);
    }.map(|(a,b,c,d)| vec![
        // DECODE P: publish the customer name in the typed customer/orders answer.
        dictionaries.publish("name", *a),
        b.to_string(), c.to_string(), d.to_string(),
    ]).collect();
    typed::check("typed_customer_orders", rows);
}
