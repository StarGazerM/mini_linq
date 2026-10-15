//! retail_d19: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_d19 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_d19") {
                let mut inputs = cq_workloads::execution::inputs("retail_d19", scenario);
                let RetailD19StoreSales = host_view!($view, inputs.$load::<(i32, i32, i32, i32, i32,)>("RetailD19StoreSales"));
                let RetailD19DateDim = host_view!($view, inputs.$load::<(i32,)>("RetailD19DateDim"));
                let RetailD19Item = host_view!($view, inputs.$load::<(i32,)>("RetailD19Item"));
                let RetailD19Customer = host_view!($view, inputs.$load::<(i32, i32,)>("RetailD19Customer"));
                let RetailD19CustomerAddress = host_view!($view, inputs.$load::<(i32,)>("RetailD19CustomerAddress"));
                let RetailD19Store = host_view!($view, inputs.$load::<(i32,)>("RetailD19Store"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailD19StoreSales, RetailD19DateDim, RetailD19Item, RetailD19Customer, RetailD19CustomerAddress, RetailD19Store))?);
                    }
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
                        RetailD19Store(store_key);
                }.map(|row| vec![*row.0, *row.1, *row.2, *row.3]).collect();
                cq_workloads::execution::check("retail_d19", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_d19, run_retail_d19);
