::mini_linq::workload_query! {
    pub struct RetailD85Program;
    relation RetailD85WebSales(c0: i32, c1: i32, c2: i32, c3: i32);
    relation RetailD85WebReturns(c0: i32, c1: i32, c2: i32, c3: i32, c4: i32, c5: i32);
    relation RetailD85WebPage(c0: i32);
    relation RetailD85CustomerDemo(c0: i32);
    relation RetailD85CustomerAddress(c0: i32);
    relation RetailD85DateDim(c0: i32);
    relation RetailD85Reason(c0: i32);

    retail_d85_result(order_number, item_key, reason_key) :-
        RetailD85WebSales(order_number, item_key, web_page, sold_date),
        RetailD85WebReturns(
            order_number,
            item_key,
            refunded_demo,
            returning_demo,
            address_key,
            reason_key,
        ),
        RetailD85WebPage(web_page),
        RetailD85CustomerDemo(refunded_demo),
        RetailD85CustomerDemo(returning_demo),
        RetailD85CustomerAddress(address_key),
        RetailD85DateDim(sold_date),
        RetailD85Reason(reason_key).
}
