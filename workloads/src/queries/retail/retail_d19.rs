::mini_linq::workload_query! {
    pub struct RetailD19Program;
    relation RetailD19StoreSales(c0: i32, c1: i32, c2: i32, c3: i32, c4: i32);
    relation RetailD19DateDim(c0: i32);
    relation RetailD19Item(c0: i32);
    relation RetailD19Customer(c0: i32, c1: i32);
    relation RetailD19CustomerAddress(c0: i32);
    relation RetailD19Store(c0: i32);

    retail_d19_result(ticket_number, item_key, customer_key, store_key) :-
        RetailD19StoreSales(ticket_number, sold_date, item_key, customer_key, store_key),
        RetailD19DateDim(sold_date),
        RetailD19Item(item_key),
        RetailD19Customer(customer_key, address_key),
        RetailD19CustomerAddress(address_key),
        RetailD19Store(store_key).
}
