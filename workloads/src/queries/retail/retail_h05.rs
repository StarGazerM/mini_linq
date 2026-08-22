::mini_linq::workload_query! {
    pub struct RetailH05Program;
    relation RetailH05Customer(c0: i32, c1: i32);
    relation RetailH05Orders(c0: i32, c1: i32);
    relation RetailH05Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);
    relation RetailH05Supplier(c0: i32, c1: i32);
    relation RetailH05Nation(c0: i32, c1: i32);
    relation RetailH05Region(c0: i32);

    retail_h05_result(order_key, supplier_key, nation_key) :-
        RetailH05Customer(customer_key, nation_key),
        RetailH05Orders(order_key, customer_key),
        RetailH05Lineitem(order_key, _part_key, supplier_key, _line_number),
        RetailH05Supplier(supplier_key, nation_key),
        RetailH05Nation(nation_key, region_key),
        RetailH05Region(region_key).
}
