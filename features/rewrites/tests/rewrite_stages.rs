//! R1 and C1 through the Physical rewrites: the core stage driver of
//! `facade/mini-linq/tests/stage_support` plus six rewrite stages applied
//! cumulatively after lowering. Every stage must denote the reference answer;
//! every one-fact Physical mutant must be rejected by its stage's checker or
//! keep the answer. Opt-in target (`test = false`): run with
//! `cargo test --manifest-path features/rewrites/Cargo.toml --test rewrite_stages`
//! (`CQ_STAGE_CASES`, `CQ_MUTANT_CASES`, `CQ_FUZZ_SEED`, `CQ_FUZZ_PROFILE`).
#[path = "../../../facade/mini-linq/tests/stage_support/mod.rs"]
mod stage_support;

use std::collections::BTreeMap;

use stage_support::{PassError, pass};

#[test]
fn every_stage_denotes_the_reference_answer() {
    stage_support::differential::every_stage_denotes_the_reference_answer(rewrites);
}

#[test]
fn authored_records_reject_mutants_or_keep_the_answer() {
    stage_support::mutants::authored_records_reject_mutants_or_keep_the_answer(rewrites);
}

#[test]
fn generated_records_reject_mutants_or_keep_the_answer() {
    stage_support::mutants::generated_records_reject_mutants_or_keep_the_answer(rewrites);
}

/// Physical rewrites applied cumulatively after lowering, each with `all`
/// selection (or every eligible name when the rewrite requires names).
fn rewrites(
    lowered: &cq_lang_physical::Program,
) -> Result<Vec<(String, cq_lang_physical::Program)>, PassError> {
    use cq_lang_physical::{Mode, Operator, symbol_name};
    use cq_rewrite_passes as rw;
    let all = quote::quote!(all);
    let mut out = Vec::new();
    let mut current = lowered.clone();
    type Rewrite = fn(
        &cq_lang_physical::Program,
        proc_macro2::TokenStream,
    ) -> syn::Result<cq_lang_physical::Program>;
    let steps: [(&str, Rewrite); 4] = [
        ("physical:merge_joins", rw::merge_joins),
        ("physical:canon_prefixes", rw::canon_prefixes),
        ("physical:smallest", rw::smallest_intersections),
        ("physical:lazy_semijoins", rw::lazy_semijoins),
    ];
    for (name, rewrite) in steps {
        current = pass(name, rewrite(&current, all.clone()))?;
        out.push((name.to_owned(), current.clone()));
    }
    let pipes: Vec<syn::Ident> = current
        .program
        .definitions
        .iter()
        .filter(|d| matches!(d.mode, Mode::Pipe(_)) && !matches!(d.operator, Operator::Rename(_)))
        .map(|d| d.name.clone())
        .collect();
    // Retaining a stream can make a recorded `smallest` inadmissible; such a
    // rewrite is rejected by its own check, so retain each admissible stream.
    let mut retained = false;
    for name in pipes {
        if let Ok(next) = rw::retain_streams(&current, quote::quote!(#name)) {
            current = next;
            retained = true;
        }
    }
    if retained {
        out.push(("physical:retain_streams".into(), current.clone()));
    }
    // Original identifiers keep raw spellings (`r#in`).
    let hosts: Vec<syn::Ident> = current
        .program
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            Operator::Rename(r) => Some((symbol_name(&r.relation), r.relation.clone())),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect();
    if !hosts.is_empty() {
        current = pass(
            "physical:host_sets",
            rw::host_sets(&current, quote::quote!(#(#hosts),*)),
        )?;
        out.push(("physical:host_sets".into(), current.clone()));
    }
    Ok(out)
}
