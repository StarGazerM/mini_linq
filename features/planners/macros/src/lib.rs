//! CQ→RA′ macros on the public cq-stage protocol.
#![forbid(unsafe_code)]

use cq_planners_plan::{Planner, cq, ra};
use proc_macro::TokenStream;

/// `order!(arguments { prefix greedy, fine }; pipeline { next, ... }; <CQ>)`.
/// The selected order is elaborated into the emitted RA′ program.
#[proc_macro]
pub fn order(input: TokenStream) -> TokenStream {
    cq_stage::pass_with_calibration::<cq::Program, ra::Program>(input.into(), |source, arguments, enabled, planning| {
        let planner: Planner = syn::parse2(arguments)?;
        use cq_stage::calibration::{Seed, Stage};
        let (target, valuation, lowering, steps) = cq_planners_plan::compile_with_steps(source, planner)?;
        let reads = cq_planning::first_reads(&valuation);
        if !enabled {
            *planning = Some(cq_stage::planning::Planning { reads, steps: steps.0 });
            return Ok((target, None));
        }
        let seed = Seed {
            valuation: Stage::of(&valuation),
            lowering: Stage::of(&lowering),
            steps: steps.0.clone(),
        };
        *planning = Some(cq_stage::planning::Planning { reads, steps: steps.0 });
        Ok((target, Some(seed)))
    })
    .into()
}
