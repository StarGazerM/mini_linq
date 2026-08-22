::mini_linq::workload_query! {
    pub struct RetailD72InnerProgram;
    relation RetailD72InnerCatalogSales(c0: i32, c1: i32, c2: i32, c3: i32, c4: i32, c5: i32);
    relation RetailD72InnerDateDim(c0: i32, c1: i32);
    relation RetailD72InnerInventory(c0: i32, c1: i32, c2: i32);
    relation RetailD72InnerWarehouse(c0: i32);
    relation RetailD72InnerItem(c0: i32);
    relation RetailD72InnerCustomerDemo(c0: i32);
    relation RetailD72InnerHouseholdDemo(c0: i32);

    retail_d72_inner_result(order_number, item_key, warehouse_key, week_sequence) :-
        RetailD72InnerCatalogSales(
            order_number,
            item_key,
            customer_demo,
            household_demo,
            sold_date,
            ship_date,
        ),
        RetailD72InnerDateDim(sold_date, week_sequence),
        RetailD72InnerInventory(item_key, inventory_date, warehouse_key),
        RetailD72InnerDateDim(inventory_date, week_sequence),
        RetailD72InnerWarehouse(warehouse_key),
        RetailD72InnerItem(item_key),
        RetailD72InnerCustomerDemo(customer_demo),
        RetailD72InnerHouseholdDemo(household_demo),
        RetailD72InnerDateDim(ship_date, _ship_week_sequence).
}
