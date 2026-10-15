//! retail_d72_inner: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_d72_inner {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_d72_inner") {
                let mut inputs = cq_workloads::execution::inputs("retail_d72_inner", scenario);
                let RetailD72InnerCatalogSales = host_view!($view, inputs.$load::<(i32, i32, i32, i32, i32, i32,)>("RetailD72InnerCatalogSales"));
                let RetailD72InnerDateDim = host_view!($view, inputs.$load::<(i32, i32,)>("RetailD72InnerDateDim"));
                let RetailD72InnerInventory = host_view!($view, inputs.$load::<(i32, i32, i32,)>("RetailD72InnerInventory"));
                let RetailD72InnerWarehouse = host_view!($view, inputs.$load::<(i32,)>("RetailD72InnerWarehouse"));
                let RetailD72InnerItem = host_view!($view, inputs.$load::<(i32,)>("RetailD72InnerItem"));
                let RetailD72InnerCustomerDemo = host_view!($view, inputs.$load::<(i32,)>("RetailD72InnerCustomerDemo"));
                let RetailD72InnerHouseholdDemo = host_view!($view, inputs.$load::<(i32,)>("RetailD72InnerHouseholdDemo"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailD72InnerCatalogSales, RetailD72InnerDateDim, RetailD72InnerInventory, RetailD72InnerWarehouse, RetailD72InnerItem, RetailD72InnerCustomerDemo, RetailD72InnerHouseholdDemo))?);
                    }
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
                        RetailD72InnerDateDim(ship_date, _ship_week_sequence);
                }.map(|row| vec![*row.0, *row.1, *row.2, *row.3]).collect();
                cq_workloads::execution::check("retail_d72_inner", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_d72_inner, run_retail_d72_inner);
