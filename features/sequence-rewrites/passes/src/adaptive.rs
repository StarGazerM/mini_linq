//! Runtime filter selection over an already published count (RPT+ §5).
//!
//! The caller supplies the two complete constructions, including the mandatory
//! exact join in the unfiltered arm. Both are checked independently of evidence.
//! This pass does not estimate cardinalities or move decisions into translators.

use std::collections::BTreeSet;

use cq_lang_sequence::structures::physical::Operator;
use cq_lang_sequence::{Definition, Evidence, Program, Term};
use cq_syntax::symbol_name;
use quote::{format_ident, quote};
use syn::Ident;

use crate::{checked_in, choose_in, definition};

#[path = "adaptive_candidates.rs"]
mod candidates;
pub use candidates::{
    AdaptiveCandidate, adaptive, adaptive_candidates, adaptive_candidates_in, apply_adaptive,
    apply_adaptive_in,
};

/// Select a static filter policy only when selectivity advice is present.
/// RPT+'s creation threshold is 35%; advice is a performance hint, not a premise.
pub fn select_filter(evidence: &Evidence, relation: &str) -> Option<bool> {
    evidence.selectivity(relation).map(|selectivity| {
        u128::from(selectivity.numerator) * 20 <= u128::from(selectivity.denominator) * 7
    })
}

/// Install an evidence-selected arm, or generate a runtime choice when absent.
///
/// `filtered` and `unfiltered` describe the producer of `name`; an outer store
/// already authored on `name` is retained. Even a statically discarded arm must
/// pass the same construction checker, so incorrect evidence cannot change rows.
#[allow(clippy::too_many_arguments)]
pub fn adaptive_filter(
    source: &Program,
    name: &Ident,
    published: &Ident,
    threshold: usize,
    filtered: Term,
    unfiltered: Term,
    evidence: &Evidence,
    relation: &str,
) -> syn::Result<Program> {
    let checker = &mut Default::default();
    let Some(use_filter) = select_filter(evidence, relation) else {
        return adaptive_choose(source, name, published, threshold, filtered, unfiltered);
    };
    let index = definition(source, name)?;
    let yes = replace_producer(checker, source, index, filtered)?;
    let no = replace_producer(checker, source, index, unfiltered)?;
    Ok(if use_filter { yes } else { no })
}

/// Choose genuinely different, equal-support constructions using a published
/// filtered-store count. `threshold` is explicit advice, not an estimated count.
///
/// Alternatives are inlined beneath the choice, so a store in one arm cannot
/// become an eager, unconditional publication merely by naming that arm.
/// The existing `choose` rewrite checks exhaustiveness and arm equivalence; every
/// staging step also passes `check_rewrite`. No Structures entry is modified.
// Formal rule EXT-SeqAdaptive: Published counts select independently checked equal-support filter constructions.
pub fn adaptive_choose(
    source: &Program,
    name: &Ident,
    published: &Ident,
    threshold: usize,
    filtered: Term,
    unfiltered: Term,
) -> syn::Result<Program> {
    adaptive_choose_in(
        &mut Default::default(),
        source,
        name,
        published,
        threshold,
        filtered,
        unfiltered,
    )
}

fn adaptive_choose_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    name: &Ident,
    published: &Ident,
    threshold: usize,
    filtered: Term,
    unfiltered: Term,
) -> syn::Result<Program> {
    let analysis = checker.check(source)?;
    let index = definition(source, name)?;
    let count_index = definition(source, published)?;
    if count_index >= index
        || !matches!(
            source.sequence.definitions[count_index].term,
            Term::Store { .. }
        )
        || !analysis.stores.contains_key(&symbol_name(published))
    {
        return Err(syn::Error::new_spanned(
            published,
            "adaptive choose requires an earlier published store count",
        ));
    }
    let filtered_result = analysis.results.get(&symbol_name(published));
    if !filtered_result.is_some_and(|result| has_filter(source, result)) {
        return Err(syn::Error::new_spanned(
            published,
            "adaptive choose count must come from a filtered store",
        ));
    }
    if threshold == 0 {
        return Err(syn::Error::new_spanned(
            name,
            "adaptive choose threshold must be positive",
        ));
    }

    // Check each candidate against the original output, not just against the
    // other candidate. Evidence and the guard cannot supply a correctness fact.
    replace_producer(checker, source, index, filtered.clone())?;
    replace_producer(checker, source, index, unfiltered.clone())?;
    if expand_refs(source, &filtered) == expand_refs(source, &unfiltered) {
        return Err(syn::Error::new_spanned(
            name,
            "adaptive choose requires different work in its alternatives",
        ));
    }

    let mut occupied: BTreeSet<_> = source
        .sequence
        .definitions
        .iter()
        .map(|definition| symbol_name(&definition.name))
        .chain(
            source
                .structures
                .physical
                .program
                .definitions
                .iter()
                .map(|definition| symbol_name(&definition.name)),
        )
        .collect();
    let yes = fresh(name, "filtered", &mut occupied);
    let no = fresh(name, "unfiltered", &mut occupied);
    let decision = fresh(name, "adaptive", &mut occupied);
    let original = match &source.sequence.definitions[index].term {
        Term::Store { inner, .. } => inner.as_ref().clone(),
        term => term.clone(),
    };
    let mut staged = source.clone();
    staged.sequence.definitions.splice(
        index..index,
        [
            Definition {
                name: yes.clone(),
                term: filtered,
            },
            Definition {
                name: no.clone(),
                term: unfiltered,
            },
            Definition {
                name: decision.clone(),
                term: original,
            },
        ],
    );
    let staged = checked_in(checker, source, staged)?;
    let threshold = syn::LitInt::new(&threshold.to_string(), name.span());
    let selected = choose_in(
        checker,
        &staged,
        quote!(#decision, len #published < #threshold => #yes, else => #no),
    )?;
    // `choose` accepts named arms. Inline those temporary definitions afterwards
    // so translators cannot execute an arm's publication before its guard.
    let mut inlined = selected.clone();
    let yes = inlined.sequence.definitions.remove(index).term;
    let no = inlined.sequence.definitions.remove(index).term;
    let Term::Choose(arms) = &mut inlined.sequence.definitions[index].term else {
        unreachable!("the existing choose rewrite constructs a choice")
    };
    arms[0].1 = yes;
    arms[1].1 = no;
    let inlined = checked_in(checker, &selected, inlined)?;
    let target_index = definition(&inlined, name)?;
    let target = replace_producer(checker, &inlined, target_index, Term::Ref(decision))?;
    checked_in(checker, source, target)
}

fn replace_producer(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    index: usize,
    producer: Term,
) -> syn::Result<Program> {
    let mut target = source.clone();
    match &mut target.sequence.definitions[index].term {
        Term::Store { inner, .. } => **inner = producer,
        term => *term = producer,
    }
    checked_in(checker, source, target)
}

fn has_filter(program: &Program, result: &str) -> bool {
    let mut current = result.to_owned();
    while let Some(definition) = program
        .structures
        .physical
        .program
        .definitions
        .iter()
        .find(|definition| symbol_name(&definition.name) == current)
    {
        if matches!(
            definition.operator,
            Operator::Select(_)
                | Operator::Point(_, _)
                | Operator::Range(_, _)
                | Operator::Guard(_)
                | Operator::Semi(_, _)
                | Operator::Anti(_, _)
        ) {
            return true;
        }
        if let Operator::Rename(rename) = &definition.operator
            && rename.constants.is_some()
        {
            return true;
        }
        let Some(input) = definition.operator.input() else {
            break;
        };
        current = symbol_name(input);
    }
    false
}

fn fresh(owner: &Ident, suffix: &str, occupied: &mut BTreeSet<String>) -> Ident {
    let stem = symbol_name(owner);
    for serial in 0usize.. {
        let name = format_ident!("__cq_{}_{}_{}", stem, suffix, serial, span = owner.span());
        if occupied.insert(symbol_name(&name)) {
            return name;
        }
    }
    unreachable!("finite programs leave an available identifier")
}

// Resolve aliases before rejecting placebo arms. Checked programs only contain
// backwards references, so this walk terminates without a separate cycle policy.
fn expand_refs(program: &Program, term: &Term) -> Term {
    let mut expanded = term.clone();
    match &mut expanded {
        Term::Ref(name) => {
            if let Some(definition) = program
                .sequence
                .definitions
                .iter()
                .find(|definition| definition.name == *name)
            {
                // A named publication is a real boundary; do not confuse reading
                // it with recomputing its producer or adding a second store.
                if !matches!(definition.term, Term::Store { .. }) {
                    return expand_refs(program, &definition.term);
                }
            }
        }
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Lookup { inner, .. }
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::Store { inner, .. } => {
            **inner = expand_refs(program, inner);
        }
        Term::Union(arms) => {
            for arm in arms {
                *arm = expand_refs(program, arm);
            }
        }
        Term::Choose(arms) => {
            for (_, arm) in arms {
                *arm = expand_refs(program, arm);
            }
        }
        Term::Rows(_) => {}
    }
    expanded
}
