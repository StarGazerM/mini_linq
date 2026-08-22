::mini_linq::workload_query! {
    pub struct RetailH03Program;
    relation RetailH03Customer(c0: i32, c1: i32);
    relation RetailH03Orders(c0: i32, c1: i32);
    relation RetailH03Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);

    retail_h03_result(order_key, customer_key) :-
        RetailH03Customer(customer_key, _customer_nation),
        RetailH03Orders(order_key, customer_key),
        RetailH03Lineitem(order_key, _part_key, _supplier_key, _line_number).
}
