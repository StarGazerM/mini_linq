//! oriented-chain: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_oriented_chain {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("oriented-chain") {
                let mut inputs = cq_workloads::execution::inputs("oriented-chain", scenario);
                let Edge = host_view!($view, inputs.$load::<(i32, i32,)>("Edge"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(Edge))?);
                    }
                    pub struct OrientedChainProgram;
                    relation Edge(c0: i32, c1: i32);

                    chain(a, d) :- Edge(a, b), Edge(b, c), Edge(d, c);
                }.map(|row| vec![*row.0, *row.1]).collect();
                cq_workloads::execution::check("oriented-chain", scenario, answers);
            }
        }
    };
}

case_configurations!(oriented_chain, run_oriented_chain);
