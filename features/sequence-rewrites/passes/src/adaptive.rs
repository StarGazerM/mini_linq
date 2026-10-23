//! Runtime filter selection over an already published count (RPT+ §5).
//!
//! The caller supplies the two complete constructions, including the mandatory
//! exact join in the unfiltered arm. Both are checked independently of evidence.
//! This pass does not estimate cardinalities or move decisions into translators.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cq_lang_sequence::structures::physical::Operator;
use cq_lang_sequence::{Definition, Evidence, Guard, Len, Program, Term};
use cq_syntax::symbol_name;
use quote::format_ident;
use syn::Ident;

use crate::{checked_in, definition};

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
/// Every arm and the final construction pass `check_rewrite`. The final choice
/// is checked directly, without manufacturing and replaying temporary programs.
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
    let analysis = checker.check_shared(source)?;
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
    if same_work(source, &filtered, &unfiltered) {
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
    let decision = fresh(name, "adaptive", &mut occupied);
    let mut target = source.clone();
    let producer = Term::Ref(decision.clone());
    match &mut target.sequence.definitions[index].term {
        Term::Store { inner, .. } => **inner = producer,
        term => *term = producer,
    }
    target.sequence.definitions.insert(
        index,
        Definition {
            name: decision,
            term: Term::Choose(vec![
                (
                    Guard::Lt(Len::Input(symbol_name(published)), threshold),
                    filtered,
                ),
                (Guard::Else, unfiltered),
            ]),
        },
    );
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

// Compare the named DAG without expanding it into a potentially exponential
// tree. Checked programs only contain backwards references. Publications remain
// boundaries: reading a store differs from recomputing its producer.
fn same_work(program: &Program, left: &Term, right: &Term) -> bool {
    fn resolve<'a>(mut term: &'a Term, definitions: &BTreeMap<String, &'a Term>) -> &'a Term {
        while let Term::Ref(name) = term {
            let Some(&producer) = definitions.get(&symbol_name(name)) else {
                break;
            };
            if matches!(producer, Term::Store { .. }) {
                break;
            }
            term = producer;
        }
        term
    }

    fn equal<'a>(
        left: &'a Term,
        right: &'a Term,
        definitions: &BTreeMap<String, &'a Term>,
        memo: &mut HashMap<(*const Term, *const Term), bool>,
    ) -> bool {
        let left = resolve(left, definitions);
        let right = resolve(right, definitions);
        if std::ptr::eq(left, right) {
            return true;
        }
        let key = (std::ptr::from_ref(left), std::ptr::from_ref(right));
        if let Some(&same) = memo.get(&key) {
            return same;
        }
        let same = match (left, right) {
            (Term::Ref(a), Term::Ref(b)) | (Term::Rows(a), Term::Rows(b)) => a == b,
            (Term::Map(a, l), Term::Map(b, r))
            | (Term::Compact(a, l), Term::Compact(b, r))
            | (Term::Group(a, l), Term::Group(b, r)) => a == b && equal(l, r, definitions, memo),
            (Term::Flatten(l), Term::Flatten(r)) | (Term::Canon(l), Term::Canon(r)) => {
                equal(l, r, definitions, memo)
            }
            (Term::Fold(a, l), Term::Fold(b, r)) => a == b && equal(l, r, definitions, memo),
            (
                Term::Lookup {
                    provider: a,
                    keys: ka,
                    inner: l,
                },
                Term::Lookup {
                    provider: b,
                    keys: kb,
                    inner: r,
                },
            ) => a == b && ka == kb && equal(l, r, definitions, memo),
            (
                Term::Tile {
                    n: a,
                    items: ia,
                    inner: l,
                },
                Term::Tile {
                    n: b,
                    items: ib,
                    inner: r,
                },
            ) => a == b && ia == ib && equal(l, r, definitions, memo),
            (
                Term::Store {
                    capacity: a,
                    inner: l,
                },
                Term::Store {
                    capacity: b,
                    inner: r,
                },
            ) => a == b && equal(l, r, definitions, memo),
            (Term::Union(a), Term::Union(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(l, r)| equal(l, r, definitions, memo))
            }
            (Term::Choose(a), Term::Choose(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|((ga, l), (gb, r))| ga == gb && equal(l, r, definitions, memo))
            }
            _ => false,
        };
        memo.insert(key, same);
        same
    }

    let definitions = program
        .sequence
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.name), &definition.term))
        .collect();
    equal(left, right, &definitions, &mut HashMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Program {
        syn::parse_quote! {
            physical {
                relation R(a:i32);
                r = pipe rename R@0 {a -> a};
                return r as answer(a);
            }
            structures { r host scan = Reader []; }
            sequence { left = rows r; right = rows r; }
        }
    }

    #[test]
    fn shared_alias_dags_are_compared_without_expanding_all_paths() {
        let mut program = source();
        let mut left: Ident = syn::parse_quote!(left);
        let mut right: Ident = syn::parse_quote!(right);
        for index in 0..48 {
            let next_left = format_ident!("left_{index}");
            let next_right = format_ident!("right_{index}");
            program
                .sequence
                .definitions
                .push(syn::parse_quote!(#next_left = union [#left, #left];));
            program
                .sequence
                .definitions
                .push(syn::parse_quote!(#next_right = union [#right, #right];));
            left = next_left;
            right = next_right;
        }
        let left = Term::Ref(left);
        let right = Term::Ref(right);
        assert!(same_work(&program, &left, &right));
        program.sequence.definitions[1].term = syn::parse_quote!(canon rows r);
        assert!(!same_work(&program, &left, &right));
    }

    #[test]
    fn alias_comparison_preserves_publication_and_guard_boundaries() {
        let mut program = source();
        program.sequence.definitions[0].term = syn::parse_quote!(store rows r);
        assert!(!same_work(
            &program,
            &syn::parse_quote!(left),
            &syn::parse_quote!(store rows r),
        ));
        assert!(!same_work(
            &program,
            &syn::parse_quote!(choose [len r < 2 => left, else => right]),
            &syn::parse_quote!(choose [len r < 3 => left, else => right]),
        ));
    }
}
