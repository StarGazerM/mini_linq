//! Structural Sequence laws and construction-based rewrite equality.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_structures::physical::{self, ModeKind, Operator};
use cq_syntax::symbol_name;
use quote::ToTokens;
use syn::Ident;

use crate::{Analysis, Capability, Guard, Len, Monoid, Obligation, OutputProtocol, Program, Term};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Meaning {
    Result(String),
    Host(String),
    Lookup(String, Vec<String>, Box<Meaning>),
    Fold(Monoid, Box<Meaning>),
}

#[derive(Clone)]
struct Fact {
    len: Len,
    meaning: Meaning,
    depth: usize,
    max_depth: usize,
    materializes: bool,
    reads: Vec<(String, Vec<String>)>,
}

struct Checker<'a> {
    program: &'a Program,
    analysis: Analysis,
    facts: BTreeMap<String, Fact>,
}

fn fail<T>(term: &impl ToTokens, rule: &str, message: &str) -> syn::Result<T> {
    Err(syn::Error::new_spanned(term, format!("{rule}: {message}")))
}

/// Check the complete Structures prefix and derive the Sequence facts.
pub fn check(program: &Program) -> syn::Result<Analysis> {
    analyze(program).map(|checker| checker.analysis)
}

fn analyze(program: &Program) -> syn::Result<Checker<'_>> {
    let structures = cq_lang_structures::contract::check(&program.structures)?;
    let mut checker = Checker {
        program,
        analysis: Analysis {
            structures,
            lengths: BTreeMap::new(),
            views: BTreeSet::new(),
            stores: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            levels: BTreeMap::new(),
            dispatches: Vec::new(),
            collectives: Vec::new(),
            output: None,
            obligations: Vec::new(),
        },
        facts: BTreeMap::new(),
    };
    for definition in &program.sequence.definitions {
        let name = symbol_name(&definition.name);
        if checker.facts.contains_key(&name) {
            return fail(
                &definition.name,
                "BE-Alpha",
                "duplicate Sequence definition",
            );
        }
        let fact = checker.term(&definition.term, &name)?;
        checker
            .analysis
            .lengths
            .insert(name.clone(), fact.len.clone());
        if !fact.materializes {
            checker.analysis.views.insert(name.clone());
        }
        checker.facts.insert(name, fact);
    }
    if let Some(last) = program.sequence.definitions.last() {
        let name = symbol_name(&last.name);
        if checker.analysis.stores.contains_key(&name) {
            let protocol = program
                .sequence
                .output
                .clone()
                .unwrap_or(OutputProtocol::Count);
            if protocol == OutputProtocol::Bounded && !has_capacity(&last.term) {
                return fail(
                    &last.term,
                    "BE-Length",
                    "bounded output requires a declared capacity",
                );
            }
            checker.analysis.output = Some((name, protocol));
        } else if program.sequence.output.is_some() {
            return fail(
                &last.term,
                "BE-Length",
                "output protocol requires a final store",
            );
        }
    } else if program.sequence.output.is_some() {
        return fail(
            program,
            "BE-Length",
            "output protocol requires a final store",
        );
    }
    Ok(checker)
}

fn has_capacity(term: &Term) -> bool {
    matches!(
        term,
        Term::Store {
            capacity: Some(_),
            ..
        }
    )
}

impl Checker<'_> {
    fn physical(&self, name: &Ident) -> syn::Result<&physical::Definition> {
        self.program
            .structures
            .physical
            .program
            .definitions
            .iter()
            .find(|definition| symbol_name(&definition.name) == symbol_name(name))
            .ok_or_else(|| syn::Error::new_spanned(name, "BE-Alpha: unknown Physical result"))
    }

    fn heading(&self, meaning: &Meaning) -> Vec<String> {
        match meaning {
            Meaning::Result(name) => self.analysis.structures.physical.headings[name].clone(),
            Meaning::Host(name) => self
                .analysis
                .structures
                .host_layouts
                .get(name)
                .cloned()
                .unwrap_or_default(),
            Meaning::Lookup(provider, _, inner) => {
                let mut heading = self.heading(inner);
                heading.extend(
                    self.analysis.structures.physical.headings[provider]
                        .iter()
                        .cloned(),
                );
                heading.sort();
                heading.dedup();
                heading
            }
            Meaning::Fold(_, _) => Vec::new(),
        }
    }

    // Formal rule BE-Alpha: Validate named total segments and their input provenance.
    fn term(&mut self, term: &Term, owner: &str) -> syn::Result<Fact> {
        match term {
            Term::Rows(name) => {
                let name_string = symbol_name(name);
                let physical = &self.analysis.structures.physical;
                let meaning = if physical.headings.contains_key(&name_string) {
                    if !physical.hosts.contains_key(&name_string)
                        && physical.modes[&name_string] != ModeKind::Let
                    {
                        return fail(name, "BE-Alpha", "rows requires a retained result or host");
                    }
                    Meaning::Result(name_string.clone())
                } else if self
                    .analysis
                    .structures
                    .host_layouts
                    .contains_key(&name_string)
                {
                    Meaning::Host(name_string.clone())
                } else {
                    return fail(name, "BE-Alpha", "rows names no retained result or host");
                };
                Ok(Fact {
                    len: Len::Input(name_string),
                    meaning,
                    depth: 0,
                    max_depth: 0,
                    materializes: false,
                    reads: Vec::new(),
                })
            }
            Term::Ref(name) => self
                .facts
                .get(&symbol_name(name))
                .cloned()
                .map(|mut fact| {
                    fact.materializes = false;
                    fact
                })
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        name,
                        "BE-Alpha: reference must name an earlier Sequence definition",
                    )
                }),
            Term::Map(name, inner) | Term::Compact(name, inner) => {
                let mut fact = self.term(inner, owner)?;
                if fact.max_depth != 0 {
                    return fail(
                        term,
                        "BE-Alpha",
                        "map/compact requires rows, not unflattened buckets",
                    );
                }
                if matches!(term, Term::Compact(_, _))
                    && !matches!(self.physical(name)?.operator, Operator::Select(_))
                {
                    return fail(name, "BE-Alpha", "compact must name a Physical select");
                }
                let source = fact.meaning.clone();
                let mut current = symbol_name(name);
                let mut changes_length = false;
                let mut ordered = false;
                let mut nested = false;
                let mut provider_reads = Vec::new();
                loop {
                    if source == Meaning::Result(current.clone()) {
                        break;
                    }
                    let definition = self
                        .program
                        .structures
                        .physical
                        .program
                        .definitions
                        .iter()
                        .find(|definition| symbol_name(&definition.name) == current)
                        .ok_or_else(|| {
                            syn::Error::new_spanned(
                                name,
                                "BE-Alpha: map does not extend its input result",
                            )
                        })?;
                    let operator = &definition.operator;
                    if matches!(
                        operator,
                        Operator::Bind(_) | Operator::Guard(_) | Operator::Union(_)
                    ) {
                        return fail(
                            name,
                            "BE-Alpha",
                            "map segment contains an opaque or non-total operation",
                        );
                    }
                    changes_length |= !matches!(
                        operator,
                        Operator::Rename(_) | Operator::Project(_) | Operator::Unit(_)
                    );
                    nested |= matches!(operator, Operator::Probe(_, _) | Operator::Merge(_, _));
                    ordered |= matches!(operator, Operator::Range(_, _) | Operator::Merge(_, _));
                    match operator {
                        Operator::Probe(_, keyed)
                        | Operator::Merge(_, keyed)
                        | Operator::Semi(_, keyed)
                        | Operator::Anti(_, keyed) => {
                            let mut keys: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
                            keys.sort();
                            let capability = match operator {
                                Operator::Merge(_, _) => Capability::Ordered,
                                Operator::Semi(_, _) | Operator::Anti(_, _) => {
                                    Capability::Membership
                                }
                                _ => Capability::Exact,
                            };
                            provider_reads.push((keyed.right.clone(), keys, capability));
                        }
                        _ => {}
                    }
                    if let Operator::Rename(rename) = operator {
                        if source == Meaning::Host(symbol_name(&rename.relation)) {
                            break;
                        }
                    }
                    let Some(input) = operator.input() else {
                        return fail(
                            name,
                            "BE-Alpha",
                            "map segment does not start at its supplied input",
                        );
                    };
                    current = symbol_name(input);
                }
                // Even identity applications must name a total operation.
                if matches!(
                    self.physical(name)?.operator,
                    Operator::Bind(_) | Operator::Guard(_) | Operator::Union(_)
                ) {
                    return fail(name, "BE-Alpha", "map must name a total Physical segment");
                }
                for (provider, keys, capability) in provider_reads {
                    let enumerated = matches!(capability, Capability::Exact | Capability::Ordered);
                    self.provider(&provider, &keys, capability)?;
                    if enumerated {
                        fact.reads.push((symbol_name(&provider), keys));
                    }
                }
                fact.meaning = Meaning::Result(symbol_name(name));
                self.analysis.obligations.push(Obligation::Codec {
                    columns: self.heading(&fact.meaning),
                    ordered,
                });
                if nested {
                    fact.depth = 1;
                    fact.max_depth = 1;
                    fact.len = Len::Unknown(None);
                } else if changes_length || matches!(term, Term::Compact(_, _)) {
                    fact.len = Len::Scan(owner.into());
                }
                Ok(fact)
            }
            Term::Tile { n, items, inner } => {
                // Formal rule BE-Partition: Positive contiguous tiles are a disjoint covering view.
                if *n == 0 || *items == Some(0) {
                    return fail(
                        term,
                        "BE-Partition",
                        "tile and items sizes must be positive",
                    );
                }
                let mut fact = self.term(inner, owner)?;
                fact.depth += 1;
                fact.max_depth += 1;
                Ok(fact)
            }
            Term::Lookup {
                provider,
                keys,
                inner,
            } => {
                let mut fact = self.term(inner, owner)?;
                if fact.max_depth != 0 {
                    return fail(term, "BE-Alpha", "lookup requires a row domain");
                }
                let provider_name = symbol_name(provider);
                self.physical(provider)?;
                let mut names: Vec<_> = keys.iter().map(symbol_name).collect();
                names.sort();
                let heading = &self.analysis.structures.physical.headings[&provider_name];
                let input_heading = self.heading(&fact.meaning);
                if names.windows(2).any(|pair| pair[0] == pair[1])
                    || names
                        .iter()
                        .any(|key| !heading.contains(key) || !input_heading.contains(key))
                {
                    return fail(
                        term,
                        "BE-Alpha",
                        "lookup keys must be distinct columns of provider and input",
                    );
                }
                let capability = if self
                    .analysis
                    .structures
                    .membership
                    .contains(&(provider_name.clone(), names.clone()))
                {
                    Capability::Membership
                } else {
                    Capability::Exact
                };
                self.provider(provider, &names, capability)?;
                fact.reads.push((provider_name.clone(), names.clone()));
                fact.meaning = Meaning::Lookup(provider_name, names, Box::new(fact.meaning));
                fact.depth = 1;
                fact.max_depth = 1;
                fact.len = Len::Unknown(None);
                Ok(fact)
            }
            Term::Flatten(inner) => {
                // Formal rule BE-Offsets: Flatten requires nested positions and records their exclusive scan.
                let mut fact = self.term(inner, owner)?;
                if fact.depth == 0 {
                    return fail(term, "BE-Offsets", "flatten requires nested positions");
                }
                for (provider, keys) in &fact.reads {
                    let name = Ident::new(provider, proc_macro2::Span::call_site());
                    self.provider(&name, keys, Capability::Range)?;
                }
                fact.depth -= 1;
                fact.max_depth -= 1;
                fact.len = Len::Scan(owner.into());
                if let Meaning::Lookup(provider, keys, inner) = &fact.meaning {
                    if let Meaning::Result(left) = inner.as_ref() {
                        if let Some(definition) = self.program.structures.physical.program.definitions.iter().find(|definition| {
                            matches!(&definition.operator, Operator::Probe(_, keyed)
                                if symbol_name(&keyed.left) == *left && symbol_name(&keyed.right) == *provider
                                    && { let mut columns: Vec<_> = keyed.columns.iter().map(symbol_name).collect(); columns.sort(); columns == *keys })
                        }) {
                            fact.meaning = Meaning::Result(symbol_name(&definition.name));
                        }
                    }
                }
                Ok(fact)
            }
            Term::Fold(monoid, inner) => {
                let mut fact = self.term(inner, owner)?;
                if fact.max_depth != 0 {
                    return fail(term, "BE-Partition", "fold requires a flat row domain");
                }
                if let Monoid::Sum(column) = monoid {
                    if !self.heading(&fact.meaning).contains(&symbol_name(column)) {
                        return fail(
                            column,
                            "BE-Partition",
                            "sum column is absent from the input heading",
                        );
                    }
                }
                fact.meaning = Meaning::Fold(monoid.clone(), Box::new(fact.meaning));
                fact.len = Len::Lit(1);
                Ok(fact)
            }
            Term::Canon(inner) => {
                // Formal rule BE-Canon: Support is global over flat rows; providers remain trait obligations.
                let mut fact = self.term(inner, owner)?;
                if fact.max_depth != 0 {
                    return fail(term, "BE-Canon", "global canon requires flattened rows");
                }
                let ty = match &fact.meaning {
                    Meaning::Result(name) => self.analysis.structures.dedup.get(name).cloned(),
                    _ => None,
                };
                self.analysis.obligations.push(Obligation::Canon {
                    result: owner.into(),
                    ty,
                });
                self.analysis.obligations.push(Obligation::Codec {
                    columns: self.heading(&fact.meaning),
                    ordered: false,
                });
                fact.len = Len::Scan(owner.into());
                Ok(fact)
            }
            Term::Store { capacity, inner } => {
                // Formal rule BE-Length: Every materialization has an exact size expression or a capacity.
                let mut fact = self.term(inner, owner)?;
                if matches!(fact.len, Len::Unknown(_)) {
                    let carried = match fact.len {
                        Len::Unknown(value) => value,
                        _ => None,
                    };
                    let capacity = capacity.or(carried);
                    if capacity.is_none() {
                        return fail(term, "BE-Length", "unknown store length requires capacity");
                    }
                    fact.len = Len::Unknown(capacity);
                }
                fact.materializes = true;
                self.analysis.stores.insert(owner.into(), fact.len.clone());
                Ok(fact)
            }
            Term::Choose(alternatives) => {
                // Formal rule BE-Choose: Length guards cover all sizes and alternatives have equal support.
                if alternatives.is_empty() || !self.exhaustive(alternatives)? {
                    return fail(term, "BE-Choose", "length guards are not exhaustive");
                }
                let mut result: Option<Fact> = None;
                for (_, alternative) in alternatives {
                    let next = self.term(alternative, owner)?;
                    if let Some(previous) = &mut result {
                        if previous.meaning != next.meaning {
                            return fail(
                                term,
                                "BE-Choose",
                                "alternatives must have the same Structures result",
                            );
                        }
                        previous.depth = previous.depth.min(next.depth);
                        previous.max_depth = previous.max_depth.max(next.max_depth);
                        if previous.len != next.len {
                            previous.len = Len::Unknown(None);
                        }
                        previous.materializes |= next.materializes;
                        previous.reads.extend(next.reads);
                    } else {
                        result = Some(next);
                    }
                }
                Ok(result.expect("nonempty alternatives checked"))
            }
        }
    }

    // Formal rule BE-Bind: Flattened lookups require range cursors, never membership-only providers.
    fn provider(
        &mut self,
        provider: &Ident,
        keys: &[String],
        required: Capability,
    ) -> syn::Result<()> {
        let name = symbol_name(provider);
        let key = (name.clone(), keys.to_vec());
        if required != Capability::Membership && self.analysis.structures.membership.contains(&key)
        {
            return fail(
                provider,
                "BE-Bind",
                "enumeration requires Exact/Range/Ordered, not Membership capability",
            );
        }
        let ty = self
            .analysis
            .structures
            .built_on(&name, keys)
            .map(|(ty, _)| ty.clone())
            .or_else(|| {
                self.analysis
                    .structures
                    .fallbacks
                    .iter()
                    .find(|((result, _, columns), _)| *result == name && columns == keys)
                    .map(|(_, ty)| ty.clone())
            })
            .or_else(|| {
                self.analysis
                    .structures
                    .keysets
                    .get(&key)
                    .map(|keyset| keyset.ty.clone())
            });
        let Some(ty) = ty else {
            return fail(provider, "BE-Bind", "lookup has no provider for these keys");
        };
        let capability = match (self.analysis.capabilities.get(&key), &required) {
            (Some(Capability::Ordered), _) | (_, Capability::Ordered) => Capability::Ordered,
            (Some(Capability::Range), _) | (_, Capability::Range) => Capability::Range,
            (Some(Capability::Exact), _) | (_, Capability::Exact) => Capability::Exact,
            _ => Capability::Membership,
        };
        let ordered = capability == Capability::Ordered;
        self.analysis
            .capabilities
            .insert(key.clone(), capability.clone());
        self.analysis.obligations.retain(|obligation| !matches!(obligation,
            Obligation::Provider { provider, keys: columns, .. } if *provider == name && columns == keys));
        self.analysis.obligations.push(Obligation::Provider {
            provider: name,
            keys: keys.to_vec(),
            capability,
            ty,
        });
        // Codec injectivity and monotonicity are deliberately not claimed statically.
        self.analysis.obligations.push(Obligation::Codec {
            columns: keys.to_vec(),
            ordered,
        });
        Ok(())
    }

    fn guard_len(&self, len: &Len) -> syn::Result<()> {
        match len {
            Len::Input(name)
                if self.facts.contains_key(name)
                    || self
                        .analysis
                        .structures
                        .physical
                        .headings
                        .contains_key(name)
                    || self.analysis.structures.host_layouts.contains_key(name) =>
            {
                Ok(())
            }
            Len::Lit(_) => Ok(()),
            Len::Add(left, right) => {
                self.guard_len(left)?;
                self.guard_len(right)
            }
            _ => Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "BE-Choose: guard must use available input lengths, literals, and addition",
            )),
        }
    }

    fn exhaustive(&self, alternatives: &[(Guard, Term)]) -> syn::Result<bool> {
        let mut covered = false;
        for (index, (guard, _)) in alternatives.iter().enumerate() {
            match guard {
                Guard::Else => {
                    if index + 1 != alternatives.len() {
                        return fail(guard, "BE-Choose", "else must be the last guard");
                    }
                    covered = true;
                }
                Guard::Lt(len, bound) | Guard::Ge(len, bound) => {
                    self.guard_len(len)?;
                    if let Some(value) = constant_len(len) {
                        covered |= match guard {
                            Guard::Lt(_, _) => value < *bound,
                            _ => value >= *bound,
                        };
                    }
                    if matches!(guard, Guard::Ge(_, 0)) {
                        covered = true;
                    }
                    if let Guard::Lt(_, upper) = guard {
                        covered |= alternatives.iter().any(|(other, _)| matches!(other, Guard::Ge(other_len, lower) if len == other_len && lower <= upper));
                    }
                }
            }
        }
        Ok(covered)
    }
}

fn constant_len(len: &Len) -> Option<usize> {
    match len {
        Len::Lit(value) => Some(*value),
        Len::Add(left, right) => constant_len(left)?.checked_add(constant_len(right)?),
        _ => None,
    }
}

/// Rewrites preserve Structures exactly and the final support by construction.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    let source_facts = analyze(source)?;
    let target_facts = analyze(target)?;
    if source.structures != target.structures
        || source.structures.to_token_stream().to_string()
            != target.structures.to_token_stream().to_string()
    {
        return fail(
            target,
            "BE-Alpha",
            "Sequence rewrite must preserve Structures exactly",
        );
    }
    let final_meaning = |program: &Program, facts: &Checker<'_>| {
        program
            .sequence
            .definitions
            .last()
            .map(|definition| facts.facts[&symbol_name(&definition.name)].meaning.clone())
    };
    if final_meaning(source, &source_facts) != final_meaning(target, &target_facts) {
        return fail(
            target,
            "BE-Choose",
            "rewrite changes final support rather than a construction view",
        );
    }
    Ok(())
}
