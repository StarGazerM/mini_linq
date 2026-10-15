//! triangle: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_triangle {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("triangle") {
                let mut inputs = cq_workloads::execution::inputs("triangle", scenario);
                let R = host_view!($view, inputs.$load::<(i32, i32,)>("R"));
                let S = host_view!($view, inputs.$load::<(i32, i32,)>("S"));
                let T = host_view!($view, inputs.$load::<(i32, i32,)>("T"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(R, S, T))?);
                    }
                    pub struct CorpusTriangleProgram;
                    relation R(c0: i32, c1: i32);
                    relation S(c0: i32, c1: i32);
                    relation T(c0: i32, c1: i32);

                    triangle(x, y, z) :- R(x, y), S(y, z), T(z, x);
                }.map(|row| vec![*row.0, *row.1, *row.2]).collect();
                cq_workloads::execution::check("triangle", scenario, answers);
            }
        }
    };
}
