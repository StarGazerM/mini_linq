//! claw: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_claw {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("claw") {
                let mut inputs = cq_workloads::execution::inputs("claw", scenario);
                let A = host_view!($view, inputs.$load::<(i32, i32,)>("A"));
                let B = host_view!($view, inputs.$load::<(i32, i32,)>("B"));
                let C = host_view!($view, inputs.$load::<(i32, i32,)>("C"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(A, B, C))?);
                    }
                    pub struct ClawProgram;
                    relation A(c0: i32, c1: i32);
                    relation B(c0: i32, c1: i32);
                    relation C(c0: i32, c1: i32);

                    claw(center, a, b, c) :- A(center, a), B(center, b), C(center, c);
                }.map(|row| vec![*row.0, *row.1, *row.2, *row.3]).collect();
                cq_workloads::execution::check("claw", scenario, answers);
            }
        }
    };
}

case_configurations!(claw, run_claw);
