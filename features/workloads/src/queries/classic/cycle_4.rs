//! cycle-4: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_cycle_4 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("cycle-4") {
                let mut inputs = cq_workloads::execution::inputs("cycle-4", scenario);
                let R = host_view!($view, inputs.$load::<(i32, i32,)>("R"));
                let S = host_view!($view, inputs.$load::<(i32, i32,)>("S"));
                let T = host_view!($view, inputs.$load::<(i32, i32,)>("T"));
                let U = host_view!($view, inputs.$load::<(i32, i32,)>("U"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(R, S, T, U))?);
                    }
                    pub struct Cycle4Program;
                    relation R(c0: i32, c1: i32);
                    relation S(c0: i32, c1: i32);
                    relation T(c0: i32, c1: i32);
                    relation U(c0: i32, c1: i32);

                    cycle4(a, b, c, d) :- R(a, b), S(b, c), T(c, d), U(d, a);
                }.map(|row| vec![*row.0, *row.1, *row.2, *row.3]).collect();
                cq_workloads::execution::check("cycle-4", scenario, answers);
            }
        }
    };
}

case_configurations!(cycle_4, run_cycle_4);
