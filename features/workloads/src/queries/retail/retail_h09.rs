//! retail_h09: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_retail_h09 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("retail_h09") {
                let mut inputs = cq_workloads::execution::inputs("retail_h09", scenario);
                let RetailH09Lineitem = host_view!($view, inputs.$load::<(i32, i32, i32, i32,)>("RetailH09Lineitem"));
                let RetailH09PartSupp = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH09PartSupp"));
                let RetailH09Part = host_view!($view, inputs.$load::<(i32,)>("RetailH09Part"));
                let RetailH09Supplier = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH09Supplier"));
                let RetailH09Orders = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH09Orders"));
                let RetailH09Nation = host_view!($view, inputs.$load::<(i32, i32,)>("RetailH09Nation"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(RetailH09Lineitem, RetailH09PartSupp, RetailH09Part, RetailH09Supplier, RetailH09Orders, RetailH09Nation))?);
                    }
                    pub struct RetailH09Program;
                    relation RetailH09Lineitem(c0: i32, c1: i32, c2: i32, c3: i32);
                    relation RetailH09PartSupp(c0: i32, c1: i32);
                    relation RetailH09Part(c0: i32);
                    relation RetailH09Supplier(c0: i32, c1: i32);
                    relation RetailH09Orders(c0: i32, c1: i32);
                    relation RetailH09Nation(c0: i32, c1: i32);

                    retail_h09_result(order_key, part_key, supplier_key) :-
                        RetailH09Lineitem(order_key, part_key, supplier_key, _line_number),
                        RetailH09PartSupp(part_key, supplier_key),
                        RetailH09Part(part_key),
                        RetailH09Supplier(supplier_key, nation_key),
                        RetailH09Orders(order_key, _customer_key),
                        RetailH09Nation(nation_key, _region_key);
                }.map(|row| vec![*row.0, *row.1, *row.2]).collect();
                cq_workloads::execution::check("retail_h09", scenario, answers);
            }
        }
    };
}

case_configurations!(retail_h09, run_retail_h09);
