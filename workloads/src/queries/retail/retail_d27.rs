::mini_linq::workload_query! {
    pub struct RetailD27Program;
    relation RetailD27StoreSales(c0: i32, c1: i32, c2: i32, c3: i32, c4: i32);
    relation RetailD27DateDim(c0: i32);
    relation RetailD27Item(c0: i32);
    relation RetailD27Store(c0: i32);
    relation RetailD27CustomerDemo(c0: i32);

    retail_d27_result(ticket_number, item_key, store_key) :-
        RetailD27StoreSales(ticket_number, sold_date, item_key, store_key, customer_demo),
        RetailD27DateDim(sold_date),
        RetailD27Item(item_key),
        RetailD27Store(store_key),
        RetailD27CustomerDemo(customer_demo).
}
