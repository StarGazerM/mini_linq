use std::collections::BTreeSet;

use cq_lang_sequence::{Program, Term};
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use syn::Ident;

/// Remove a group's valuation canonicalizers only when the checker proves
/// positional uniqueness from the author's asserted keys (or existing laws).
pub fn group_without_canon(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    group_without_canon_in(&mut Default::default(), source, arguments)
}

pub fn group_without_canon_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    checker.check_shared(source)?;
    let name: Ident = syn::parse2(arguments)?;
    let index = super::definition(source, &name)?;
    fn group(term: &mut Term) -> Option<&mut Term> {
        match term {
            Term::Group(_, input) => Some(input),
            Term::Store { inner, .. } | Term::Canon(inner) => group(inner),
            _ => None,
        }
    }
    fn canonical(term: &Term, source: &Program) -> bool {
        match term {
            Term::Canon(_) => true,
            Term::Ref(name) => super::definition(source, name)
                .ok()
                .is_some_and(|index| canonical(&source.sequence.definitions[index].term, source)),
            Term::Store { inner, .. } => canonical(inner, source),
            _ => false,
        }
    }
    fn expanding(term: &Term, source: &Program) -> bool {
        match term {
            Term::Lookup { .. } => true,
            Term::Ref(name) => super::definition(source, name)
                .ok()
                .is_some_and(|index| expanding(&source.sequence.definitions[index].term, source)),
            Term::Canon(inner)
            | Term::Map(_, inner)
            | Term::Compact(_, inner)
            | Term::Flatten(inner)
            | Term::Tile { inner, .. } => expanding(inner, source),
            _ => false,
        }
    }
    fn strip(term: &mut Term, pending: &mut Vec<Ident>, source: &Program) {
        match term {
            Term::Ref(name) => pending.push(name.clone()),
            Term::Canon(inner) => {
                *term = *inner.clone();
                strip(term, pending, source);
            }
            Term::Store { inner, .. } if canonical(inner, source) && !expanding(inner, source) => {
                *term = *inner.clone();
                strip(term, pending, source);
            }
            Term::Map(_, inner)
            | Term::Compact(_, inner)
            | Term::Flatten(inner)
            | Term::Store { inner, .. }
            | Term::Tile { inner, .. }
            | Term::Lookup { inner, .. } => strip(inner, pending, source),
            // Do not cross folds, choices or unions: their own multiplicity
            // contracts need independent proofs, not this key transfer.
            _ => {}
        }
    }
    let mut target = source.clone();
    let input = group(&mut target.sequence.definitions[index].term).ok_or_else(|| {
        syn::Error::new_spanned(&name, "group_without_canon requires a group definition")
    })?;
    let mut pending = Vec::new();
    strip(input, &mut pending, source);
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if visited.insert(symbol_name(&name)) {
            let index = super::definition(&target, &name)?;
            strip(
                &mut target.sequence.definitions[index].term,
                &mut pending,
                source,
            );
        }
    }
    super::checked_in(checker, source, target)
}
