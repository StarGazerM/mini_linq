//! Shared by the sampler and its generated proc-macro crate. SQL does not use it.

pub fn plan(
    source: &cq_lang_cq::Program,
    planner: usize,
    fine: bool,
    reduce: usize,
) -> syn::Result<cq_lang_ra::Program> {
    use cq_order::{Heuristic, Ranking, Split};
    let normal = cq_lang_cq::normal::normalize(source)?;
    let options = cq_order::Options {
        heuristic: match planner {
            0 => Heuristic::Written,
            1 => Heuristic::Prefix(Ranking::Connected),
            2 => Heuristic::Prefix(Ranking::GreedyWidth),
            3 => Heuristic::Elimination,
            _ => unreachable!("sampled planner"),
        },
        split: if fine { Split::Fine } else { Split::Coarse },
    };
    let plan = cq_order::compile(&normal, &options)?;
    let plan = cq_reduce::compile(
        &plan,
        &cq_reduce::Options {
            guards: if reduce == 0 {
                cq_reduce::Pushdown::None
            } else {
                cq_reduce::Pushdown::Atoms
            },
            semijoins: if reduce == 2 && cq_lang_cq::hypergraph::Analysis::of(&normal).acyclic {
                cq_reduce::Reductions::Tree
            } else {
                cq_reduce::Reductions::None
            },
        },
    )?;
    cq_elaborate::compile(&plan)
}
