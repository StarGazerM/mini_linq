//! retail_h21_positive: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_h21_positive {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_h21_positive") {
                let mut inputs = cq_workloads::execution::inputs("retail_h21_positive", scenario);
                let RetailH21PositiveSupplier = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH21PositiveSupplier"));
                let RetailH21PositiveLineitem = host_view!($view, inputs.$load::<(i32, i32, i32, i32,)>("RetailH21PositiveLineitem"));
                let RetailH21PositiveOrders = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH21PositiveOrders"));
                let RetailH21PositiveNation = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH21PositiveNation"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailH21PositiveSupplier, RetailH21PositiveLineitem, RetailH21PositiveOrders, RetailH21PositiveNation))?);
                    }
                    pub struct RetailH21PositiveProgram;
                    relation RetailH21PositiveSupplier(c0: i32, c1: i32);
                    relation RetailH21PositiveLineitem(c0: i32, c1: i32, c2: i32, c3: i32);
                    relation RetailH21PositiveOrders(c0: i32, c1: i32);
                    relation RetailH21PositiveNation(c0: i32, c1: i32);

                    retail_h21_positive_result(order_key, waiting_supplier, other_supplier) :-
                        RetailH21PositiveSupplier(waiting_supplier, nation_key),
                        RetailH21PositiveLineitem(
                            order_key,
                            _waiting_part,
                            waiting_supplier,
                            _waiting_line,
                        ),
                        RetailH21PositiveOrders(order_key, _customer_key),
                        RetailH21PositiveNation(nation_key, _region_key),
                        RetailH21PositiveLineitem(order_key, _other_part, other_supplier, _other_line);
                }.map(|row| vec![*row.0, *row.1, *row.2]).collect();
                cq_workloads::execution::check("retail_h21_positive", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_h21_positive, run_retail_h21_positive);
