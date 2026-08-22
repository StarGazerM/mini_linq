::mini_linq::workload_query! {
    pub struct RetailH09Program;
    relation RetailH09Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);
    relation RetailH09PartSupp(c0: i32, c1: i32);
    relation RetailH09Part(c0: i32);
    relation RetailH09Supplier(c0: i32, c1: i32);
    relation RetailH09Orders(c0: i32, c1: i32);
    relation RetailH09Nation(c0: i32, c1: i32);

    retail_h09_result(order_key, part_key, supplier_key) :-
        RetailH09Lineitem(order_key, part_key, supplier_key, _line_number),
        RetailH09PartSupp(part_key, supplier_key),
        RetailH09Part(part_key),
        RetailH09Supplier(supplier_key, nation_key),
        RetailH09Orders(order_key, _customer_key),
        RetailH09Nation(nation_key, _region_key).
}
