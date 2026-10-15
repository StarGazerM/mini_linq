//! intersection: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_intersection {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("intersection") {
                let mut inputs = cq_workloads::execution::inputs("intersection", scenario);
                let Left = host_view!($view, inputs.$load::<(i32,)>("Left"));
                let Right = host_view!($view, inputs.$load::<(i32,)>("Right"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(Left, Right))?);
                    }
                    pub struct UnaryIntersectionProgram;
                    relation Left(c0: i32);
                    relation Right(c0: i32);

                    intersection(x) :- Left(x), Right(x);
                }.map(|row| vec![*row.0]).collect();
                cq_workloads::execution::check("intersection", scenario, answers);
            }
        }
    };
}

case_configurations!(intersection, run_intersection);
