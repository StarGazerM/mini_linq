//! Sharded or sorted-run admission and exclusive whole-layout publication certificates.
//! Hybrid execution uses a separate wrapper and never extends the base Round
//! language. Layout ownership and opaque queues are preserved; keyed IDB layout
//! providers may opt into sharding or sorted runs independently of admission.
//! Run layouts require the default storage hasher for exact checked erasure;
//! run admission retains its source hasher. The standard Sequence tail may use push.
//! Scan substitution renames its discharged host fact and lower packet together;
//! all other facts and the Round admission-owner lists remain unchanged.
use cq_datalog_support::Fresh;
use cq_lang_datalog_round::{self as round, Execution, Program};

mod sharded;
pub use sharded::*;

fn push_path() -> syn::Path {
    syn::parse_quote!(::cq_datalog::passes::print_push)
}

/// Only the standard, argument-free lowering/grade/terminal tail is erasable.
/// Authored Sequence rewrites and custom grades keep their original semantics.
fn push_tail(program: &Program, hole: &round::Hole) -> Option<usize> {
    if hole.interface != round::HoleInterface::Pull
        || !program.terminals.0.iter().any(|terminal| {
            terminal.path == push_path() && terminal.interface == round::HoleInterface::Push
        })
    {
        return None;
    }
    let start = hole.pipeline.passes.len().checked_sub(3)?;
    if hole.lower >= start {
        return None;
    }
    let paths: [syn::Path; 3] = [
        syn::parse_quote!(mini_linq::passes::sequence),
        syn::parse_quote!(mini_linq::passes::sequential),
        syn::parse_quote!(mini_linq::passes::translate_pull),
    ];
    hole.pipeline.passes[start..]
        .iter()
        .zip(paths)
        .all(|(pass, path)| pass.path.segments == path.segments && pass.arguments.is_empty())
        .then_some(start)
}

fn realize_tasks(source: &Program) -> syn::Result<Program> {
    let mut target = source.clone();
    for rule in source
        .context
        .strata
        .iter()
        .flat_map(|s| s.seeds.iter().chain(&s.steps))
    {
        if let Some(start) = push_tail(source, &source.holes[rule.hole]) {
            let hole = &mut target.holes[rule.hole];
            hole.pipeline.passes.truncate(start);
            let path = push_path();
            hole.pipeline.passes.push(syn::parse_quote!(#path));
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
