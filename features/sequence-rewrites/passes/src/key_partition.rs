//! Checked key-disjoint decomposition; routing choices remain explicit advice.
//! A single-use named Canon feeding Group moves inside the same key-disjoint
//! parts as an inline Canon. Shared producers and explicit stores stay intact.
use cq_lang_sequence::{Definition, Guard, Len, Program, Term};
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, Token, bracketed};

/// Replayable routing advice. Empty keys select contiguous tiles only for a
/// globally unique unkeyed Group; other groups route complete valuation keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPartition {
    pub name: Ident,
    pub input: Ident,
    pub keys: Vec<Ident>,
    pub parts: usize,
    /// Runtime row-count cutoff for provider builds; absent means fixed routing.
    pub threshold: Option<usize>,
}

impl Parse for KeyPartition {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![,]>()?;
        let producer = input.parse()?;
        input.parse::<Token![,]>()?;
        let keys;
        bracketed!(keys in input);
        let keys = keys.parse_terminated(Ident::parse, Token![,])?.into_iter().collect();
        input.parse::<Token![,]>()?;
        let parts = input.parse::<LitInt>()?.base10_parse()?;
        let threshold = if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            let keyword: Ident = input.parse()?;
            if keyword != "threshold" {
                return Err(syn::Error::new_spanned(keyword, "expected threshold"));
            }
            Some(input.parse::<LitInt>()?.base10_parse()?)
        } else { None };
        Ok(Self { name, input: producer, keys, parts, threshold })
    }
}

impl ToTokens for KeyPartition {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { name, input, keys, parts, threshold } = self;
        let parts = LitInt::new(&parts.to_string(), name.span());
        let threshold = threshold.map(|n| {
            let n = LitInt::new(&n.to_string(), name.span());
            quote!(, threshold #n)
        });
        tokens.extend(quote!(#name, #input, [#(#keys),*], #parts #threshold));
    }
}
fn references(term: &Term, counts: &mut std::collections::BTreeMap<String, usize>) {
    match term {
        Term::Ref(name) => *counts.entry(symbol_name(name)).or_default() += 1,
        Term::Map(_, inner) | Term::Compact(_, inner) | Term::Flatten(inner)
        | Term::Fold(_, inner) | Term::Group(_, inner) | Term::Canon(inner)
        | Term::Tile { inner, .. } | Term::PartitionK { inner, .. }
        | Term::Store { inner, .. } | Term::Lookup { inner, .. } => references(inner, counts),
        Term::Union(arms) => {
            for term in arms { references(term, counts); }
        }
        Term::Choose(arms) => {
            for (_, term) in arms { references(term, counts); }
        }
        Term::Rows(_) => {}
    }
}

// The raw publication preserves the old flat carrier and ancestor scope.
// Ordinary row/length consumers reuse it; Lookup provider identifiers retain
// the retyped routed token. Never expose the new cover level to old row users.
fn redirect_references(term: &mut Term, name: &Ident, publication: &Ident) {
    fn length(len: &mut Len, name: &str, publication: &Ident) {
        match len {
            Len::Input(reference) if reference == name => *reference = symbol_name(publication),
            Len::Add(left, right) => {
                length(left, name, publication);
                length(right, name, publication);
            }
            _ => {}
        }
    }
    match term {
        Term::Ref(reference) if reference == name => *reference = publication.clone(),
        Term::Map(_, inner) | Term::Compact(_, inner) | Term::Flatten(inner)
        | Term::Fold(_, inner) | Term::Group(_, inner) | Term::Canon(inner)
        | Term::Tile { inner, .. } | Term::PartitionK { inner, .. }
        | Term::Store { inner, .. } | Term::Lookup { inner, .. } => {
            redirect_references(inner, name, publication);
        }
        Term::Union(arms) => {
            for arm in arms { redirect_references(arm, name, publication); }
        }
        Term::Choose(arms) => {
            for (guard, arm) in arms {
                if let Guard::Lt(len, _) | Guard::Ge(len, _) = guard {
                    length(len, &symbol_name(name), publication);
                }
                redirect_references(arm, name, publication);
            }
        }
        Term::Rows(_) | Term::Ref(_) => {}
    }
}

/// Move, rather than duplicate, a pure single-use canonicalizer. Keeping its
/// named raw producer avoids changing other pipeline names or their grades.
/// A retained or shared canonical result is an existing reuse boundary.
fn canonical_input(
    term: &Term,
    source: &Program,
    counts: &std::collections::BTreeMap<String, usize>,
    updates: &mut Vec<(usize, Term)>,
) -> Option<Term> {
    match term {
        Term::Canon(inner) => Some(*inner.clone()),
        Term::Ref(name) if counts.get(&symbol_name(name)) == Some(&1) => {
            let index = super::definition(source, name).ok()?;
            let definition = &source.sequence.definitions[index];
            let raw = canonical_input(&definition.term, source, counts, updates)?;
            if raw != definition.term {
                updates.push((index, raw));
            }
            Some(term.clone())
        }
        _ => None,
    }
}

pub fn key_partition(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    key_partition_in(&mut Default::default(), source, &syn::parse2(arguments)?)
}

/// Introduce a named publication before routing. Grading separately proves that
/// publication is at an ancestor of the fixed-width separated pool.
pub fn key_partition_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    advice: &KeyPartition,
) -> syn::Result<Program> {
    checker.check_shared(source)?;
    let index = super::definition(source, &advice.name)?;
    if advice.parts == 0 {
        return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPart: positive partition geometry required"));
    }
    if advice.threshold == Some(0) {
        return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: runtime threshold must be positive"));
    }
    if advice.threshold.is_some() && !matches!(
        &source.sequence.definitions[index].term,
        Term::Store { inner, .. } if !matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _))
    ) {
        return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: runtime threshold requires a provider Store"));
    }
    if source.sequence.definitions.iter().any(|d| d.name == advice.input) {
        return Err(syn::Error::new_spanned(&advice.input, "key partition publication name already exists"));
    }
    let mut counts = std::collections::BTreeMap::new();
    for definition in &source.sequence.definitions {
        references(&definition.term, &mut counts);
    }
    fn split(
        term: &Term,
        advice: &KeyPartition,
        source: &Program,
        counts: &std::collections::BTreeMap<String, usize>,
        updates: &mut Vec<(usize, Term)>,
    ) -> syn::Result<(Term, Term)> {
        let routed = || Term::PartitionK {
            keys: advice.keys.clone(),
            parts: advice.parts,
            inner: Box::new(Term::Ref(advice.input.clone())),
        };
        match term {
            Term::Canon(inner) if !advice.keys.is_empty() => Ok((
                *inner.clone(),
                Term::Flatten(Box::new(Term::Canon(Box::new(routed())))),
            )),
            Term::Group(group, inner) => {
                let global = source.structures.physical.program.definitions.iter()
                    .find(|definition| definition.name == *group)
                    .is_some_and(|definition| matches!(&definition.operator,
                        cq_lang_sequence::structures::physical::Operator::Group(group) if group.keys.is_empty()));
                if global && advice.keys.is_empty() {
                    return Ok((*inner.clone(), Term::Group(group.clone(), Box::new(Term::Tile {
                        n: advice.parts, items: None,
                        inner: Box::new(Term::Ref(advice.input.clone())),
                    }))));
                }
                if advice.keys.is_empty() {
                    return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartGroup: keyed groups require routing keys"));
                }
                let (input, routed) = if let Some(input) = canonical_input(inner, source, counts, updates) {
                    (input, Term::Canon(Box::new(routed())))
                } else {
                    (*inner.clone(), if global { Term::Canon(Box::new(routed())) } else { routed() })
                };
                let grouped = Term::Group(group.clone(), Box::new(routed));
                Ok((input, if global { grouped } else { Term::Flatten(Box::new(grouped)) }))
            }
            Term::Store { capacity, inner } => {
                let (input, output) = if matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)) {
                    split(inner, advice, source, counts, updates)?
                } else {
                    if advice.keys.is_empty() {
                        return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: provider routing keys must be nonempty"));
                    }
                    let output = if let Some(threshold) = advice.threshold {
                        Term::Choose(vec![
                            (Guard::Lt(Len::Input(symbol_name(&advice.input)), threshold),
                                Term::PartitionK {
                                    keys: advice.keys.clone(), parts: 1,
                                    inner: Box::new(Term::Ref(advice.input.clone())),
                                }),
                            (Guard::Else, routed()),
                        ])
                    } else { routed() };
                    (*inner.clone(), output)
                };
                Ok((input, Term::Store { capacity: *capacity, inner: Box::new(output) }))
            }
            _ => Err(syn::Error::new_spanned(&advice.name, "key_partition requires Canon with routing keys, Group, or provider Store")),
        }
    }
    let mut updates = Vec::new();
    let (input, output) = split(
        &source.sequence.definitions[index].term, advice, source, &counts, &mut updates,
    )?;
    let output = if matches!(output, Term::Store { .. }) {
        output
    } else {
        // Pool publication evaluates the collective at its task child.
        Term::Store { capacity: None, inner: Box::new(output) }
    };
    // A bare Store is only a provider candidate when the physical result is
    // consumed by exact lookup or membership. The checker records every read.
    let mut provider = None;
    if matches!(&source.sequence.definitions[index].term, Term::Store { inner, .. }
        if !matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)))
    {
        let analysis = checker.check_shared(source)?;
        let result = analysis.results.get(&symbol_name(&advice.name));
        let keys: Vec<_> = advice.keys.iter().map(symbol_name).collect();
        let reads: Vec<_> = analysis.obligations.iter().filter_map(|obligation| {
            match obligation {
                cq_lang_sequence::Obligation::Provider { provider, keys, capability, .. }
                    if Some(provider) == result => Some((keys, capability)),
                _ => None,
            }
        }).collect();
        if reads.is_empty() || reads.iter().any(|(lookup, capability)| **lookup != keys
            || !matches!(capability, cq_lang_sequence::Capability::Exact
                | cq_lang_sequence::Capability::Membership))
        {
            return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: Store requires complete-key exact lookup or membership reads"));
        }
        if let Some(result) = result
            && source.sequence.definitions.iter().take(index + 1).any(|definition|
                analysis.provider_reads.get(&symbol_name(&definition.name))
                    .is_some_and(|reads| reads.contains(result)))
        {
            return Err(syn::Error::new_spanned(&advice.name,
                "BE-KeyPartProvider: publication must precede every keyed read of its physical provider"));
        }
        provider = result.cloned();
    }
    let mut target = source.clone();
    for (index, term) in updates {
        target.sequence.definitions[index].term = term;
    }
    if let Some(provider) = provider {
        let mut found = false;
        for (entry_index, entry) in source.structures.structures.entries.iter().enumerate() {
            if symbol_name(&entry.name) != provider { continue; }
            let cq_lang_sequence::structures::Structure::Built { keyed, .. } = &entry.structure else { continue };
            let keys: Vec<_> = advice.keys.iter().map(symbol_name).collect();
            if keyed.level_names().first() != Some(&keys) {
                return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: routing keys must equal the complete outer provider key"));
            }
            let ty = &keyed.ty;
            if matches!(ty, syn::Type::Path(path) if path.qself.is_none()
                && path.path.segments.len() == 3
                && path.path.segments[0].ident == "cq_storage"
                && path.path.segments[1].ident == "sharded"
                && path.path.segments[2].ident == "Sharded")
            {
                return Err(syn::Error::new_spanned(&advice.name,
                    "BE-KeyPartProvider: physical provider is already routed"));
            }
            let entry_index = LitInt::new(&entry_index.to_string(), advice.name.span());
            target.structures = cq_rewrite_passes::retype(
                &target.structures, quote!(entry(#entry_index) => ::cq_storage::sharded::Sharded<#ty>),
            )?;
            found = true;
        }
        if !found {
            return Err(syn::Error::new_spanned(&advice.name, "BE-KeyPartProvider: no built provider layout"));
        }
        for definition in target.sequence.definitions.iter_mut().skip(index + 1) {
            redirect_references(&mut definition.term, &advice.name, &advice.input);
        }
    }
    target.sequence.definitions[index].term = output;
    target.sequence.definitions.insert(index, Definition {
        name: advice.input.clone(),
        term: Term::Store { capacity: None, inner: Box::new(input) },
    });
    super::checked_in(checker, source, target)
}
