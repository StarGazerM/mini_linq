//! retail_h05: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_h05 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_h05") {
                let mut inputs = cq_workloads::execution::inputs("retail_h05", scenario);
                let RetailH05Customer = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH05Customer"));
                let RetailH05Orders = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH05Orders"));
                let RetailH05Lineitem = host_view!($view, inputs.$load::<(i32, i32, i32, i32,)>("RetailH05Lineitem"));
                let RetailH05Supplier = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH05Supplier"));
                let RetailH05Nation = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH05Nation"));
                let RetailH05Region = host_view!($view, inputs.$load::<(i32,)>("RetailH05Region"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailH05Customer, RetailH05Orders, RetailH05Lineitem, RetailH05Supplier, RetailH05Nation, RetailH05Region))?);
                    }
                    pub struct RetailH05Program;
                    relation RetailH05Customer(c0: i32, c1: i32);
                    relation RetailH05Orders(c0: i32, c1: i32);
                    relation RetailH05Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);
                    relation RetailH05Supplier(c0: i32, c1: i32);
                    relation RetailH05Nation(c0: i32, c1: i32);
                    relation RetailH05Region(c0: i32);

                    retail_h05_result(order_key, supplier_key, nation_key) :-
                        RetailH05Customer(customer_key, nation_key),
                        RetailH05Orders(order_key, customer_key),
                        RetailH05Lineitem(order_key, _part_key, supplier_key, _line_number),
                        RetailH05Supplier(supplier_key, nation_key),
                        RetailH05Nation(nation_key, region_key),
                        RetailH05Region(region_key);
                }.map(|row| vec![*row.0, *row.1, *row.2]).collect();
                cq_workloads::execution::check("retail_h05", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_h05, run_retail_h05);
