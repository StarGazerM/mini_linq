//! CQ→RA′ macros on the public cq-stage protocol.
#![forbid(unsafe_code)]

use cq_planners_plan::{Planner, cq, ra};
use proc_macro::TokenStream;

/// `order!(arguments { prefix greedy, fine }; pipeline { next, ... }; <CQ>)`.
/// The selected order is elaborated into the emitted RA′ program.
#[proc_macro]
pub fn order(input: TokenStream) -> TokenStream {
    cq_stage::pass_with_calibration::<cq::Program, ra::Program>(input.into(), |source, arguments, enabled| {
        let planner: Planner = syn::parse2(arguments)?;
        if !enabled {
            return cq_planners_plan::compile(source, planner).map(|target| (target, None));
        }
        use cq_stage::{Language, calibration::{Coverage, Seed}};
        let (target, valuation, lowering, origins) = cq_planners_plan::compile_with_origins(source, planner)?;
        let seed = Seed {
            valuation: valuation.calibration_snapshot().expect("VO adapter"),
            lowering: lowering.calibration_snapshot().expect("RA adapter"),
            origins: origins.definitions.into_iter().map(|(node, origins)| Coverage {node, origins}).collect(),
            elided: origins.elided,
        };
        Ok((target, Some(seed)))
    })
    .into()
}
