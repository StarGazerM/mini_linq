::mini_linq::workload_query! {
    pub struct TypedCustomerOrders;
    relation Customer(customerkey: i64, name: String);
    relation Orders(customerkey: i64, orderkey: i64, orderdate: Date, total: Decimal);

    customer_orders(name, orderkey, orderdate, total) :-
        Customer(customerkey, name), Orders(customerkey, orderkey, orderdate, total);
}
