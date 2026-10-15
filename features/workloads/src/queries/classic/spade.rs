//! spade: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_spade {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("spade") {
                let mut inputs = cq_workloads::execution::inputs("spade", scenario);
                let R = host_view!($view, inputs.$load::<(i32, i32,)>("R"));
                let S = host_view!($view, inputs.$load::<(i32, i32,)>("S"));
                let T = host_view!($view, inputs.$load::<(i32, i32,)>("T"));
                let H = host_view!($view, inputs.$load::<(i32, i32,)>("H"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(R, S, T, H))?);
                    }
                    pub struct SpadeProgram;
                    relation R(c0: i32, c1: i32);
                    relation S(c0: i32, c1: i32);
                    relation T(c0: i32, c1: i32);
                    relation H(c0: i32, c1: i32);

                    spade(x, y, z, w) :- R(x, y), S(y, z), T(x, z), H(x, w);
                }.map(|row| vec![*row.0, *row.1, *row.2, *row.3]).collect();
                cq_workloads::execution::check("spade", scenario, answers);
            }
        }
    };
}

case_configurations!(spade, run_spade);
