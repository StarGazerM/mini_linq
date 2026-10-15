//! fact-star: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_fact_star {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("fact-star") {
                let mut inputs = cq_workloads::execution::inputs("fact-star", scenario);
                let Fact = host_view!($view, inputs.$load::<(i32, i32, i32,)>("Fact"));
                let DimA = host_view!($view, inputs.$load::<(i32, i32,)>("DimA"));
                let DimB = host_view!($view, inputs.$load::<(i32, i32,)>("DimB"));
                let DimC = host_view!($view, inputs.$load::<(i32, i32,)>("DimC"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(Fact, DimA, DimB, DimC))?);
                    }
                    pub struct FactStarProgram;
                    relation Fact(c0: i32, c1: i32, c2: i32);
                    relation DimA(c0: i32, c1: i32);
                    relation DimB(c0: i32, c1: i32);
                    relation DimC(c0: i32, c1: i32);

                    fact_star(k1, k2, k3, a, b, c) :-
                        Fact(k1, k2, k3),
                        DimA(k1, a),
                        DimB(k2, b),
                        DimC(k3, c);
                }.map(|row| vec![*row.0, *row.1, *row.2, *row.3, *row.4, *row.5]).collect();
                cq_workloads::execution::check("fact-star", scenario, answers);
            }
        }
    };
}

case_configurations!(fact_star, run_fact_star);
