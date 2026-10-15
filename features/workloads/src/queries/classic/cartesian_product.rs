//! cartesian-product: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_cartesian_product {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("cartesian-product") {
                let mut inputs = cq_workloads::execution::inputs("cartesian-product", scenario);
                let A = host_view!($view, inputs.$load::<(i32,)>("A"));
                let B = host_view!($view, inputs.$load::<(i32, i32,)>("B"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(A, B))?);
                    }
                    pub struct CartesianProgram;
                    relation A(c0: i32);
                    relation B(c0: i32, c1: i32);

                    product(x, y, z) :- A(x), B(y, z);
                }.map(|row| vec![*row.0, *row.1, *row.2]).collect();
                cq_workloads::execution::check("cartesian-product", scenario, answers);
            }
        }
    };
}

case_configurations!(cartesian_product, run_cartesian_product);
