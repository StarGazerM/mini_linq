::mini_linq::workload_query! {
    pub struct RetailH08Program;
    relation RetailH08Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);
    relation RetailH08Part(c0: i32);
    relation RetailH08Supplier(c0: i32, c1: i32);
    relation RetailH08Orders(c0: i32, c1: i32);
    relation RetailH08Customer(c0: i32, c1: i32);
    relation RetailH08Nation(c0: i32, c1: i32);
    relation RetailH08Region(c0: i32);

    retail_h08_result(order_key, part_key, supplier_key) :-
        RetailH08Lineitem(order_key, part_key, supplier_key, _line_number),
        RetailH08Part(part_key),
        RetailH08Supplier(supplier_key, supplier_nation),
        RetailH08Orders(order_key, customer_key),
        RetailH08Customer(customer_key, customer_nation),
        RetailH08Nation(customer_nation, customer_region),
        RetailH08Region(customer_region),
        RetailH08Nation(supplier_nation, _supplier_region).
}
