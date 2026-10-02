//! CQ→RA′ macros on the public cq-stage protocol.
#![forbid(unsafe_code)]

use cq_planners_plan::{Planner, cq, ra};
use proc_macro::TokenStream;

/// `order!(arguments { prefix greedy, fine }; pipeline { next, ... }; <CQ>)`.
/// The selected order is elaborated into the emitted RA′ program.
#[proc_macro]
pub fn order(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq::Program, ra::Program>(input.into(), |source, arguments| {
        let planner: Planner = syn::parse2(arguments)?;
        cq_planners_plan::compile(source, planner)
    })
    .into()
}
