//! barbell: bind validated hosts, compile the literal query, check the SQL golden.
macro_rules! run_barbell {
    (configuration: $configuration:ident;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     pipeline: {$($pipeline:tt)*}) => {
        #[test]
        #[allow(non_snake_case)]
        fn $configuration() {
            for &scenario in cq_workloads::execution::scenarios("barbell") {
                let mut inputs = cq_workloads::execution::inputs("barbell", scenario);
                let R = host_view!($view, inputs.$load::<(i32, i32,)>("R"));
                let S = host_view!($view, inputs.$load::<(i32, i32,)>("S"));
                let T = host_view!($view, inputs.$load::<(i32, i32,)>("T"));
                let Bridge = host_view!($view, inputs.$load::<(i32, i32,)>("Bridge"));
                let U = host_view!($view, inputs.$load::<(i32, i32,)>("U"));
                let V = host_view!($view, inputs.$load::<(i32, i32,)>("V"));
                let W = host_view!($view, inputs.$load::<(i32, i32,)>("W"));
                let answers = mini_linq::compile! {
                    pipeline {
                        $($pipeline)*
                        select = mini_linq::passes::select($($positional(R, S, T, Bridge, U, V, W))?);
                    }
                    pub struct BarbellProgram;
                    relation R(c0: i32, c1: i32);
                    relation S(c0: i32, c1: i32);
                    relation T(c0: i32, c1: i32);
                    relation Bridge(c0: i32, c1: i32);
                    relation U(c0: i32, c1: i32);
                    relation V(c0: i32, c1: i32);
                    relation W(c0: i32, c1: i32);

                    barbell(x, y, z, u, v, w) :-
                        R(x, y),
                        S(y, z),
                        T(x, z),
                        Bridge(x, u),
                        U(u, v),
                        V(v, w),
                        W(u, w);
                }.map(|row| vec![*row.0, *row.1, *row.2, *row.3, *row.4, *row.5]).collect();
                cq_workloads::execution::check("barbell", scenario, answers);
            }
        }
    };
}

case_configurations!(barbell, run_barbell);
