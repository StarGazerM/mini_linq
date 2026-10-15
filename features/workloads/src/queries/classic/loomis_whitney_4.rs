//! loomis-whitney-4: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_loomis_whitney_4 {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("loomis-whitney-4") {
                let mut inputs = cq_workloads::execution::inputs("loomis-whitney-4", scenario);
                let ABC = host_view!($view, inputs.$load::<(i32, i32, i32,)>("ABC"));
                let BCD = host_view!($view, inputs.$load::<(i32, i32, i32,)>("BCD"));
                let ACD = host_view!($view, inputs.$load::<(i32, i32, i32,)>("ACD"));
                let ABD = host_view!($view, inputs.$load::<(i32, i32, i32,)>("ABD"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(ABC, BCD, ACD, ABD))?);
                    }
                    pub struct LoomisWhitney4Program;
                    relation ABC(c0: i32, c1: i32, c2: i32);
                    relation BCD(c0: i32, c1: i32, c2: i32);
                    relation ACD(c0: i32, c1: i32, c2: i32);
                    relation ABD(c0: i32, c1: i32, c2: i32);

                    lw4(a, b, c, d) :-
                        ABC(a, b, c),
                        BCD(b, c, d),
                        ACD(a, c, d),
                        ABD(a, b, d);
                }.map(|row| vec![*row.0, *row.1, *row.2, *row.3]).collect();
                cq_workloads::execution::check("loomis-whitney-4", scenario, answers);
            }
        }
    };
}

case_configurations!(loomis_whitney_4, run_loomis_whitney_4);
