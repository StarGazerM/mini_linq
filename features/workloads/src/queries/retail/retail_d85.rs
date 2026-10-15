//! retail_d85: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_d85 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_d85") {
                let mut inputs = cq_workloads::execution::inputs("retail_d85", scenario);
                let RetailD85WebSales = host_view!($view, inputs.$load::<(i32, i32, i32, i32,)>("RetailD85WebSales"));
                let RetailD85WebReturns = host_view!($view, inputs.$load::<(i32, i32, i32, i32, i32, i32,)>("RetailD85WebReturns"));
                let RetailD85WebPage = host_view!($view, inputs.$load::<(i32,)>("RetailD85WebPage"));
                let RetailD85CustomerDemo = host_view!($view, inputs.$load::<(i32,)>("RetailD85CustomerDemo"));
                let RetailD85CustomerAddress = host_view!($view, inputs.$load::<(i32,)>("RetailD85CustomerAddress"));
                let RetailD85DateDim = host_view!($view, inputs.$load::<(i32,)>("RetailD85DateDim"));
                let RetailD85Reason = host_view!($view, inputs.$load::<(i32,)>("RetailD85Reason"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailD85WebSales, RetailD85WebReturns, RetailD85WebPage, RetailD85CustomerDemo, RetailD85CustomerAddress, RetailD85DateDim, RetailD85Reason))?);
                    }
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
                        RetailD85Reason(reason_key);
                }.map(|row| vec![*row.0, *row.1, *row.2]).collect();
                cq_workloads::execution::check("retail_d85", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_d85, run_retail_d85);
