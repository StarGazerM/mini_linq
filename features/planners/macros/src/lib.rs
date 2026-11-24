//! CQ→RA′ macros on the public cq-stage protocol.
#![forbid(unsafe_code)]

use cq_planners_plan::{Planner, cq, ra};
use proc_macro::TokenStream;

/// `order!(arguments { prefix greedy, fine }; pipeline { next, ... }; <CQ>)`.
/// The selected order is elaborated into the emitted RA′ program.
#[proc_macro]
pub fn order(input: TokenStream) -> TokenStream {
    cq_stage::pass_with_planning::<cq::Program, ra::Program>(input.into(), |source, arguments, planning| {
        let planner: Planner = syn::parse2(arguments)?;
        let (target, normal, valuation, _, steps) = cq_planners_plan::compile_with_steps(source, planner)?;
        let reads = cq_planning::first_reads(&normal, &valuation);
        *planning = Some(cq_stage::planning::Planning { reads, steps: steps.0 });
        Ok(target)
    })
    .into()
}
