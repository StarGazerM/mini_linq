::mini_linq::workload_query! {
    pub struct RetailH21PositiveProgram;
    relation RetailH21PositiveSupplier(c0: i32, c1: i32);
    relation RetailH21PositiveLineitem(c0: i32, c1: i32, c2: i32, c3: i32);
    relation RetailH21PositiveOrders(c0: i32, c1: i32);
    relation RetailH21PositiveNation(c0: i32, c1: i32);

    retail_h21_positive_result(order_key, waiting_supplier, other_supplier) :-
        RetailH21PositiveSupplier(waiting_supplier, nation_key),
        RetailH21PositiveLineitem(
            order_key,
            _waiting_part,
            waiting_supplier,
            _waiting_line,
        ),
        RetailH21PositiveOrders(order_key, _customer_key),
        RetailH21PositiveNation(nation_key, _region_key),
        RetailH21PositiveLineitem(order_key, _other_part, other_supplier, _other_line).
}
