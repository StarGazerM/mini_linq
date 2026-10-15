//! retail_h03: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_h03 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_h03") {
                let mut inputs = cq_workloads::execution::inputs("retail_h03", scenario);
                let RetailH03Customer = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH03Customer"));
                let RetailH03Orders = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH03Orders"));
                let RetailH03Lineitem = host_view!($view, inputs.$load::<(i32, i32, i32, i32,)>("RetailH03Lineitem"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailH03Customer, RetailH03Orders, RetailH03Lineitem))?);
                    }
                    pub struct RetailH03Program;
                    relation RetailH03Customer(c0: i32, c1: i32);
                    relation RetailH03Orders(c0: i32, c1: i32);
                    relation RetailH03Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);

                    retail_h03_result(order_key, customer_key) :-
                        RetailH03Customer(customer_key, _customer_nation),
                        RetailH03Orders(order_key, customer_key),
                        RetailH03Lineitem(order_key, _part_key, _supplier_key, _line_number);
                }.map(|row| vec![*row.0, *row.1]).collect();
                cq_workloads::execution::check("retail_h03", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_h03, run_retail_h03);
