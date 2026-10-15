//! retail_d27: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_d27 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_d27") {
                let mut inputs = cq_workloads::execution::inputs("retail_d27", scenario);
                let RetailD27StoreSales = host_view!($view, inputs.$load::<(i32, i32, i32, i32, i32,)>("RetailD27StoreSales"));
                let RetailD27DateDim = host_view!($view, inputs.$load::<(i32,)>("RetailD27DateDim"));
                let RetailD27Item = host_view!($view, inputs.$load::<(i32,)>("RetailD27Item"));
                let RetailD27Store = host_view!($view, inputs.$load::<(i32,)>("RetailD27Store"));
                let RetailD27CustomerDemo = host_view!($view, inputs.$load::<(i32,)>("RetailD27CustomerDemo"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailD27StoreSales, RetailD27DateDim, RetailD27Item, RetailD27Store, RetailD27CustomerDemo))?);
                    }
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
                        RetailD27CustomerDemo(customer_demo);
                }.map(|row| vec![*row.0, *row.1, *row.2]).collect();
                cq_workloads::execution::check("retail_d27", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_d27, run_retail_d27);
