//! Sharded seen admission and exclusive whole-layout publication certificates.
//! Hybrid execution uses a separate wrapper and never extends the base Round
//! language. Native full/delta layout providers and opaque CQ queues are preserved.
use cq_datalog_support::Fresh;
use cq_lang_datalog_round::{self as round, Execution, Program};

mod sharded;
pub use sharded::*;

fn partition_scans(source: &Program) -> Program {
    let mut target = source.clone();
    let mut fresh = Fresh::new(source);
    for stratum in &source.context.strata {
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            let Some(scan) = &rule.scan else { continue };
            let binding = fresh.id("parallel_chunk");
            let hole = &mut target.holes[rule.hole].cq;
            let declaration = hole
                .declarations
                .iter_mut()
                .find(|declaration| declaration.name == scan.binding)
                .expect("checked scan declaration");
            declaration.name = binding.clone();
            declaration.value = Some(syn::parse_quote!(= (#binding)));
            let round::cq::BodyItem::Positive { atom } = &mut hole.query.body[0] else {
                unreachable!("checked leading scan")
            };
            atom.relation = binding.clone();
            target.execution.push(Execution {
                hole: rule.hole,
                occurrence: 0,
                binding,
                strategy: syn::parse_quote!(rayon_sharded),
            });
        }
    }
    target
}
