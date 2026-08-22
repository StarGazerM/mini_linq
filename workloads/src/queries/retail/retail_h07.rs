::mini_linq::workload_query! {
    pub struct RetailH07Program;
    relation RetailH07Supplier(c0: i32, c1: i32);
    relation RetailH07Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);
    relation RetailH07Orders(c0: i32, c1: i32);
    relation RetailH07Customer(c0: i32, c1: i32);
    relation RetailH07Nation(c0: i32, c1: i32);

    retail_h07_result(order_key, supplier_nation, customer_nation) :-
        RetailH07Supplier(supplier_key, supplier_nation),
        RetailH07Lineitem(order_key, _part_key, supplier_key, _line_number),
        RetailH07Orders(order_key, customer_key),
        RetailH07Customer(customer_key, customer_nation),
        RetailH07Nation(supplier_nation, _supplier_region),
        RetailH07Nation(customer_nation, _customer_region).
}
