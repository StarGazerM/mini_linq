//! retail_h08: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_h08 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_h08") {
                let mut inputs = cq_workloads::execution::inputs("retail_h08", scenario);
                let RetailH08Lineitem = host_view!($view, inputs.$load::<(i32, i32, i32, i32,)>("RetailH08Lineitem"));
                let RetailH08Part = host_view!($view, inputs.$load::<(i32,)>("RetailH08Part"));
                let RetailH08Supplier = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH08Supplier"));
                let RetailH08Orders = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH08Orders"));
                let RetailH08Customer = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH08Customer"));
                let RetailH08Nation = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH08Nation"));
                let RetailH08Region = host_view!($view, inputs.$load::<(i32,)>("RetailH08Region"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailH08Lineitem, RetailH08Part, RetailH08Supplier, RetailH08Orders, RetailH08Customer, RetailH08Nation, RetailH08Region))?);
                    }
                    pub struct RetailH08Program;
                    relation RetailH08Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);
                    relation RetailH08Part(c0: i32);
                    relation RetailH08Supplier(c0: i32, c1: i32);
                    relation RetailH08Orders(c0: i32, c1: i32);
                    relation RetailH08Customer(c0: i32, c1: i32);
                    relation RetailH08Nation(c0: i32, c1: i32);
                    relation RetailH08Region(c0: i32);

                    retail_h08_result(order_key, part_key, supplier_key) :-
                        RetailH08Lineitem(order_key, part_key, supplier_key, _line_number),
                        RetailH08Part(part_key),
                        RetailH08Supplier(supplier_key, supplier_nation),
                        RetailH08Orders(order_key, customer_key),
                        RetailH08Customer(customer_key, customer_nation),
                        RetailH08Nation(customer_nation, customer_region),
                        RetailH08Region(customer_region),
                        RetailH08Nation(supplier_nation, _supplier_region);
                }.map(|row| vec![*row.0, *row.1, *row.2]).collect();
                cq_workloads::execution::check("retail_h08", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_h08, run_retail_h08);
