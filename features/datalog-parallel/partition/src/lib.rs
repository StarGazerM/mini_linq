//! Sharded seen admission and exclusive whole-layout publication certificates.
//! Hybrid execution uses a separate wrapper and never extends the base Round
//! language. Native layouts and opaque queues are preserved; a registered
//! standard pull terminal may use the existing registered push realization.
//! Scan substitution renames its discharged host fact and lower packet together;
//! all other facts and the Round admission-owner lists remain unchanged.
use cq_datalog_support::Fresh;
use cq_lang_datalog_round::{self as round, Execution, Program};

mod sharded;
pub use sharded::*;

fn push_path() -> syn::Path {
    syn::parse_quote!(::cq_datalog::passes::print_push)
}

fn can_push(program: &Program, hole: &round::Hole) -> bool {
    let pull: syn::Path = syn::parse_quote!(::mini_linq::passes::print_pull);
    let relative_pull: syn::Path = syn::parse_quote!(mini_linq::passes::print_pull);
    let push = push_path();
    hole.interface == round::HoleInterface::Pull
        && hole.pipeline.passes.last().is_some_and(|terminal| {
            (terminal.path == pull || terminal.path == relative_pull)
                && terminal.arguments.is_empty()
        })
        && program.terminals.0.iter().any(|terminal| {
            terminal.path == push && terminal.interface == round::HoleInterface::Push
        })
}

fn realize_tasks(source: &Program) -> syn::Result<Program> {
    let mut target = source.clone();
    for rule in source
        .context
        .strata
        .iter()
        .flat_map(|s| s.seeds.iter().chain(&s.steps))
    {
        if can_push(source, &source.holes[rule.hole]) {
            let hole = &mut target.holes[rule.hole];
            hole.pipeline
                .passes
                .last_mut()
                .expect("checked terminal")
                .path = push_path();
            hole.interface = round::HoleInterface::Push;
        }
    }
    round::check_rewrite(source, &target)?;
    Ok(target)
}
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
            target.holes[rule.hole].rename_host_fact(&scan.binding, &binding);
            target.execution.push(Execution {
                hole: rule.hole,
                occurrence: 0,
                binding,
                strategy: syn::parse_quote!(rayon_sharded),
                self_dedup: Some(syn::parse_quote!(per_task)),
            });
        }
    }
    target
}
