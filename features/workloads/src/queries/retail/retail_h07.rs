//! retail_h07: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_h07 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_h07") {
                let mut inputs = cq_workloads::execution::inputs("retail_h07", scenario);
                let RetailH07Supplier = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH07Supplier"));
                let RetailH07Lineitem = host_view!($view, inputs.$load::<(i32, i32, i32, i32,)>("RetailH07Lineitem"));
                let RetailH07Orders = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH07Orders"));
                let RetailH07Customer = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH07Customer"));
                let RetailH07Nation = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH07Nation"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailH07Supplier, RetailH07Lineitem, RetailH07Orders, RetailH07Customer, RetailH07Nation))?);
                    }
                    pub struct RetailH07Program;
                    relation RetailH07Supplier(c0: i32, c1: i32);
                    relation RetailH07Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);
                    relation RetailH07Orders(c0: i32, c1: i32);
                    relation RetailH07Customer(c0: i32, c1: i32);
                    relation RetailH07Nation(c0: i32, c1: i32);

                    retail_h07_result(order_key, supplier_nation, customer_nation) :-
                        RetailH07Supplier(supplier_key, supplier_nation),
                        RetailH07Lineitem(order_key, _part_key, supplier_key, _line_number),
                        RetailH07Orders(order_key, customer_key),
                        RetailH07Customer(customer_key, customer_nation),
                        RetailH07Nation(supplier_nation, _supplier_region),
                        RetailH07Nation(customer_nation, _customer_region);
                }.map(|row| vec![*row.0, *row.1, *row.2]).collect();
                cq_workloads::execution::check("retail_h07", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_h07, run_retail_h07);
