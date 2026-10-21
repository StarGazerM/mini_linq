// Programs intentionally use only the public surface; no Datalog IR is consumed.
// Andersen/CSPA equations adapted from flowlog-rs/flowlog-bench:
// https://github.com/flowlog-rs/flowlog-bench/tree/main/programs/oracle/ascent
/// Instantiate the unchanged corpus with caller-selected compiler and oracle macros.
#[macro_export]
macro_rules! oracle_programs {
    ($module:ident, $engine:ident, $ascent:path, $input:path, pipeline { $($pipeline:tt)* }) => {
    pub mod $module {
    $crate::oracle_engine!(ascent, $engine, {
    use $ascent as ascent_program;
    use $input as supplied_oracle_input;
    macro_rules! oracle_input { ($rows:expr) => { supplied_oracle_input!($rows) }; }
    });
use $crate::{Data, Rows};
use std::time::{Duration, Instant};

pub mod reach {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation seed(i32);
        relation answer(i32);
        answer(x) <-- seed(x);
        answer(y) <-- answer(x), edge(x,y);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            seed: oracle_input!(data.seed),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0,)| vec![x0]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let seed = data.seed.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation seed(c0:i32) = (seed);
                relation answer(c0:i32);
                output answer;
                answer(*x) :- seed(x);
                answer(*y) :- answer(x), edge(x,y);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0,)| vec![x0]).collect()),
            elapsed,
        )
    }
    });
}

pub mod tc_left {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation answer(i32, i32);
        answer(x,y) <-- edge(x,y);
        answer(x,z) <-- answer(x,y), edge(y,z);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation answer(c0:i32, c1:i32);
                output answer;
                answer(*x,*y) :- edge(x,y);
                answer(*x,*z) :- answer(x,y), edge(y,z);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
}

pub mod tc_right {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation answer(i32, i32);
        answer(x,y) <-- edge(x,y);
        answer(x,z) <-- edge(x,y), answer(y,z);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation answer(c0:i32, c1:i32);
                output answer;
                answer(*x,*y) :- edge(x,y);
                answer(*x,*z) :- edge(x,y), answer(y,z);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
}

pub mod tc_nonlinear {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation answer(i32, i32);
        answer(x,y) <-- edge(x,y);
        answer(x,z) <-- answer(x,y), answer(y,z);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation answer(c0:i32, c1:i32);
                output answer;
                answer(*x,*y) :- edge(x,y);
                answer(*x,*z) :- answer(x,y), answer(y,z);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
}

pub mod same_generation {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation answer(i32, i32);
        answer(x,y) <-- edge(p,x), edge(p,y);
        answer(x,y) <-- answer(a,b), edge(a,x), edge(b,y);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation answer(c0:i32, c1:i32);
                output answer;
                answer(*x,*y) :- edge(p,x), edge(p,y);
                answer(*x,*y) :- answer(a,b), edge(a,x), edge(b,y);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
}

pub mod mutual {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation nodes(i32);
        relation even(i32, i32);
        relation odd(i32, i32);
        relation answer(i32, i32, i32);
        even(x,x) <-- nodes(x);
        odd(x,z) <-- even(x,y), edge(y,z);
        even(x,z) <-- odd(x,y), edge(y,z);
        answer(0,x,y) <-- even(x,y);
        answer(1,x,y) <-- odd(x,y);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            nodes: oracle_input!(data.nodes),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(
                p.answer
                    .into_iter()
                    .map(|(x0, x1, x2)| vec![x0, x1, x2])
                    .collect(),
            ),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let nodes = data.nodes.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation nodes(c0:i32) = (nodes);
                relation even(c0:i32, c1:i32);
                relation odd(c0:i32, c1:i32);
                relation answer(c0:i32, c1:i32, c2:i32);
                output answer;
                even(*x,*x) :- nodes(x);
                odd(*x,*z) :- even(x,y), edge(y,z);
                even(*x,*z) :- odd(x,y), edge(y,z);
                answer(0,*x,*y) :- even(x,y);
                answer(1,*x,*y) :- odd(x,y);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(
                result
                    .into_iter()
                    .map(|(x0, x1, x2)| vec![x0, x1, x2])
                    .collect(),
            ),
            elapsed,
        )
    }
    });
}

pub mod multiple_seeds {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation seed(i32);
        relation extra(i32);
        relation answer(i32);
        answer(x) <-- seed(x);
        answer(x) <-- extra(x);
        answer(y) <-- answer(x), edge(x,y);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            seed: oracle_input!(data.seed),
            extra: oracle_input!(data.extra),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0,)| vec![x0]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let seed = data.seed.clone();
        let extra = data.extra.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation seed(c0:i32) = (seed);
                relation extra(c0:i32) = (extra);
                relation answer(c0:i32);
                output answer;
                answer(*x) :- seed(x);
                answer(*x) :- extra(x);
                answer(*y) :- answer(x), edge(x,y);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0,)| vec![x0]).collect()),
            elapsed,
        )
    }
    });
}

pub mod guard {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation seed(i32);
        relation answer(i32);
        answer(x) <-- seed(x);
        answer(y) <-- answer(x), edge(x,y), if *y % 2 == 0;
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            seed: oracle_input!(data.seed),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0,)| vec![x0]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let seed = data.seed.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation seed(c0:i32) = (seed);
                relation answer(c0:i32);
                output answer;
                answer(*x) :- seed(x);
                answer(*y) :- answer(x), edge(x,y), if *y % 2 == 0;
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0,)| vec![x0]).collect()),
            elapsed,
        )
    }
    });
}

pub mod unreachable {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation seed(i32);
        relation nodes(i32);
        relation reach(i32);
        relation answer(i32);
        reach(x) <-- seed(x);
        reach(y) <-- reach(x), edge(x,y);
        answer(x) <-- nodes(x), !reach(x);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            seed: oracle_input!(data.seed),
            nodes: oracle_input!(data.nodes),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0,)| vec![x0]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let seed = data.seed.clone();
        let nodes = data.nodes.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation seed(c0:i32) = (seed);
                relation nodes(c0:i32) = (nodes);
                relation reach(c0:i32);
                relation answer(c0:i32);
                output answer;
                reach(*x) :- seed(x);
                reach(*y) :- reach(x), edge(x,y);
                answer(*x) :- nodes(x), !reach(x);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0,)| vec![x0]).collect()),
            elapsed,
        )
    }
    });
}

pub mod nonedges {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation path(i32, i32);
        relation answer(i32, i32);
        path(x,y) <-- edge(x,y);
        path(x,z) <-- path(x,y), edge(y,z);
        answer(x,y) <-- path(x,y), !edge(x,y);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation path(c0:i32, c1:i32);
                relation answer(c0:i32, c1:i32);
                output answer;
                path(*x,*y) :- edge(x,y);
                path(*x,*z) :- path(x,y), edge(y,z);
                answer(*x,*y) :- path(x,y), !edge(x,y);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
}

pub mod recursive_upper {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation edge(i32, i32);
        relation seed(i32);
        relation nodes(i32);
        relation lower(i32);
        relation answer(i32, i32);
        lower(x) <-- seed(x);
        lower(y) <-- lower(x), edge(x,y);
        answer(x,x) <-- nodes(x), !lower(x);
        answer(x,z) <-- answer(x,y), edge(y,z), !lower(z);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            edge: oracle_input!(data.edge),
            seed: oracle_input!(data.seed),
            nodes: oracle_input!(data.nodes),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let edge = data.edge.clone();
        let seed = data.seed.clone();
        let nodes = data.nodes.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation edge(c0:i32, c1:i32) = (edge);
                relation seed(c0:i32) = (seed);
                relation nodes(c0:i32) = (nodes);
                relation lower(c0:i32);
                relation answer(c0:i32, c1:i32);
                output answer;
                lower(*x) :- seed(x);
                lower(*y) :- lower(x), edge(x,y);
                answer(*x,*x) :- nodes(x), !lower(x);
                answer(*x,*z) :- answer(x,y), edge(y,z), !lower(z);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
}

pub mod andersen {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation address_of(i32, i32);
        relation assign(i32, i32);
        relation load(i32, i32);
        relation store(i32, i32);
        relation answer(i32, i32);
        answer(y,x) <-- address_of(y,x);
        answer(y,x) <-- assign(y,z), answer(z,x);
        answer(y,w) <-- load(y,x), answer(x,z), answer(z,w);
        answer(z,w) <-- store(y,x), answer(y,z), answer(x,w);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            address_of: oracle_input!(data.address_of),
            assign: oracle_input!(data.assign),
            load: oracle_input!(data.load),
            store: oracle_input!(data.store),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(p.answer.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let address_of = data.address_of.clone();
        let assign = data.assign.clone();
        let load = data.load.clone();
        let store = data.store.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation address_of(c0:i32, c1:i32) = (address_of);
                relation assign(c0:i32, c1:i32) = (assign);
                relation load(c0:i32, c1:i32) = (load);
                relation store(c0:i32, c1:i32) = (store);
                relation answer(c0:i32, c1:i32);
                output answer;
                answer(*y,*x) :- address_of(y,x);
                answer(*y,*x) :- assign(y,z), answer(z,x);
                answer(*y,*w) :- load(y,x), answer(x,z), answer(z,w);
                answer(*z,*w) :- store(y,x), answer(y,z), answer(x,w);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(result.into_iter().map(|(x0, x1)| vec![x0, x1]).collect()),
            elapsed,
        )
    }
    });
}

pub mod cspa {
    use super::*;
    $crate::oracle_engine!(ascent, $engine, {
    ascent_program! {
        pub struct Oracle;
        relation assign(i32, i32);
        relation dereference(i32, i32);
        relation valueflow(i32, i32);
        relation memoryalias(i32, i32);
        relation valuealias(i32, i32);
        relation answer(i32, i32, i32);
        valueflow(y,x) <-- assign(y,x);
        valueflow(x,y) <-- assign(x,z), memoryalias(z,y);
        valueflow(x,y) <-- valueflow(x,z), valueflow(z,y);
        memoryalias(x,w) <-- dereference(y,x), valuealias(y,z), dereference(z,w);
        valuealias(x,y) <-- valueflow(z,x), valueflow(z,y);
        valuealias(x,y) <-- valueflow(z,x), memoryalias(z,w), valueflow(w,y);
        valueflow(x,x) <-- assign(x,y);
        valueflow(x,x) <-- assign(y,x);
        memoryalias(x,x) <-- assign(y,x);
        memoryalias(x,x) <-- assign(x,y);
        answer(0,x,y) <-- valueflow(x,y);
        answer(1,x,y) <-- memoryalias(x,y);
        answer(2,x,y) <-- valuealias(x,y);
    }
    pub fn oracle(data: &Data) -> (Rows, Duration) {
        let mut p = Oracle {
            assign: oracle_input!(data.assign),
            dereference: oracle_input!(data.dereference),
            ..Default::default()
        };
        let start = Instant::now();
        p.run();
        let elapsed = start.elapsed();
        (
            $crate::canonical(
                p.answer
                    .into_iter()
                    .map(|(x0, x1, x2)| vec![x0, x1, x2])
                    .collect(),
            ),
            elapsed,
        )
    }
    });
    $crate::oracle_engine!(mini, $engine, {
    pub fn mini(data: &Data) -> (Rows, Duration) {
        let assign = data.assign.clone();
        let dereference = data.dereference.clone();
        let start = Instant::now();
        let result = cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Program;
                relation assign(c0:i32, c1:i32) = (assign);
                relation dereference(c0:i32, c1:i32) = (dereference);
                relation valueflow(c0:i32, c1:i32);
                relation memoryalias(c0:i32, c1:i32);
                relation valuealias(c0:i32, c1:i32);
                relation answer(c0:i32, c1:i32, c2:i32);
                output answer;
                valueflow(*y,*x) :- assign(y,x);
                valueflow(*x,*y) :- assign(x,z), memoryalias(z,y);
                valueflow(*x,*y) :- valueflow(x,z), valueflow(z,y);
                memoryalias(*x,*w) :- dereference(y,x), valuealias(y,z), dereference(z,w);
                valuealias(*x,*y) :- valueflow(z,x), valueflow(z,y);
                valuealias(*x,*y) :- valueflow(z,x), memoryalias(z,w), valueflow(w,y);
                valueflow(*x,*x) :- assign(x,y);
                valueflow(*x,*x) :- assign(y,x);
                memoryalias(*x,*x) :- assign(y,x);
                memoryalias(*x,*x) :- assign(x,y);
                answer(0,*x,*y) :- valueflow(x,y);
                answer(1,*x,*y) :- memoryalias(x,y);
                answer(2,*x,*y) :- valuealias(x,y);
        }
        .into_iter()
        .collect::<Vec<_>>();
        let elapsed = start.elapsed();
        (
            $crate::canonical(
                result
                    .into_iter()
                    .map(|(x0, x1, x2)| vec![x0, x1, x2])
                    .collect(),
            ),
            elapsed,
        )
    }
    });
}

    }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! oracle_engine {
    (mini, mini, { $($items:tt)* }) => { $($items)* };
    (ascent, ascent, { $($items:tt)* }) => { $($items)* };
    ($kind:ident, $selected:ident, { $($items:tt)* }) => {};
}
