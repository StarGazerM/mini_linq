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
    shape: Vec<Len>,
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
            results: BTreeMap::new(),
            lengths: BTreeMap::new(),
            shapes: BTreeMap::new(),
            tilings: BTreeMap::new(),
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
        if let Meaning::Result(result) = &fact.meaning {
            checker
                .analysis
                .results
                .insert(name.clone(), result.clone());
        }
        checker
            .analysis
            .lengths
            .insert(name.clone(), fact.len.clone());
        checker
            .analysis
            .shapes
            .insert(name.clone(), fact.shape.clone());
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
    fn require_store(&self, name: &Ident) -> syn::Result<()> {
        let result = symbol_name(name);
        if !self
            .analysis
            .structures
            .physical
            .hosts
            .contains_key(&result)
            && !self.facts.values().any(|fact| {
                fact.materializes
                    && matches!(&fact.meaning, Meaning::Result(name) if name == &result)
            })
        {
            return fail(
                name,
                "BE-Scope",
                "non-host read requires an earlier Sequence store",
            );
        }
        Ok(())
    }

    fn range_reads(&mut self, fact: &Fact) -> syn::Result<()> {
        for (provider, keys) in &fact.reads {
            self.provider(
                &Ident::new(provider, proc_macro2::Span::call_site()),
                keys,
                Capability::Range,
            )?;
        }
        Ok(())
    }
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
            Term::Union(arms) => {
                if arms.is_empty() {
                    return fail(term, "BE-Alpha", "union requires at least one arm");
                }
                let mut facts = arms
                    .iter()
                    .map(|arm| self.term(arm, owner))
                    .collect::<syn::Result<Vec<_>>>()?;
                if facts.iter().any(|fact| fact.max_depth != 0) {
                    return fail(term, "BE-Alpha", "union requires flat arms");
                }
                let definition = self
                    .program
                    .structures
                    .physical
                    .program
                    .definitions
                    .iter()
                    .find(|definition| {
                        matches!(&definition.operator, Operator::Union(union)
                        if union.operands.len() == facts.len()
                            && union.operands.iter().zip(&facts).all(|(operand, fact)|
                                fact.meaning == Meaning::Result(symbol_name(operand))))
                    })
                    .ok_or_else(|| {
                        syn::Error::new_spanned(term, "BE-Alpha: union arms differ from Physical")
                    })?;
                let meaning = Meaning::Result(symbol_name(&definition.name));
                let mut result = facts.remove(0);
                for fact in facts {
                    result.len = if matches!(result.len, Len::Unknown(_))
                        || matches!(fact.len, Len::Unknown(_))
                    {
                        Len::Unknown(None)
                    } else {
                        Len::Add(Box::new(result.len), Box::new(fact.len))
                    };
                    result.reads.extend(fact.reads);
                }
                result.meaning = meaning;
                result.materializes = false;
                Ok(result)
            }
            Term::Rows(name) => {
                let name_string = symbol_name(name);
                let physical = &self.analysis.structures.physical;
                let meaning = if physical.headings.contains_key(&name_string) {
                    if !physical.hosts.contains_key(&name_string)
                        && physical.modes[&name_string] != ModeKind::Let
                        && !self.analysis.structures.retained.contains_key(&name_string)
                        && !matches!(self.physical(name)?.operator, Operator::Unit(_))
                    {
                        return fail(name, "BE-Alpha", "rows requires a retained result or host");
                    }
                    if !matches!(self.physical(name)?.operator, Operator::Unit(_)) {
                        self.require_store(name)?;
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
                    shape: vec![if matches!(meaning, Meaning::Result(_))
                        && matches!(self.physical(name)?.operator, Operator::Unit(_))
                    {
                        Len::Lit(1)
                    } else {
                        Len::Input(name_string.clone())
                    }],
                    len: if matches!(meaning, Meaning::Result(_))
                        && matches!(self.physical(name)?.operator, Operator::Unit(_))
                    {
                        Len::Lit(1)
                    } else {
                        Len::Input(name_string)
                    },
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
                // Membership consumes a bucket's existence, never its payload
                // or offsets. Its named Physical operator determines polarity.
                if matches!(term, Term::Compact(_, _))
                    && let Term::Lookup {
                        provider,
                        keys,
                        inner,
                    } = inner.as_ref()
                {
                    let definition = self.physical(name)?;
                    let keyed = match &definition.operator {
                        Operator::Semi(_, keyed) | Operator::Anti(_, keyed) => keyed,
                        Operator::Probe(_, keyed)
                            if self.analysis.structures.membership.contains(&(
                                symbol_name(provider),
                                {
                                    let mut keys: Vec<_> = keys.iter().map(symbol_name).collect();
                                    keys.sort();
                                    keys
                                },
                            )) =>
                        {
                            keyed
                        }
                        _ => {
                            return fail(
                                name,
                                "BE-Alpha",
                                "membership compact needs a membership operator",
                            );
                        }
                    };
                    let left = symbol_name(&keyed.left);
                    let right = symbol_name(&keyed.right);
                    let mut expected: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
                    expected.sort();
                    let mut columns: Vec<_> = keys.iter().map(symbol_name).collect();
                    columns.sort();
                    let mut fact = self.term(inner, owner)?;
                    if fact.meaning != Meaning::Result(left)
                        || symbol_name(provider) != right
                        || columns != expected
                    {
                        return fail(
                            term,
                            "BE-Alpha",
                            "membership lookup differs from its Physical read",
                        );
                    }
                    self.provider(provider, &columns, Capability::Membership)?;
                    fact.meaning = Meaning::Result(symbol_name(name));
                    fact.len = Len::Scan(owner.into());
                    *fact.shape.last_mut().expect("row level") = fact.len.clone();
                    return Ok(fact);
                }
                let mut fact = self.term(inner, owner)?;
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
                    // Opaque Rust is legal here; BE-Partial confines its
                    // evaluation to host when a target grading is checked.
                    if matches!(operator, Operator::Union(_)) {
                        return fail(name, "BE-Alpha", "union requires explicit Sequence arms");
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
                    fact.depth += 1;
                    fact.max_depth += 1;
                    fact.shape.push(Len::Unknown(None));
                    fact.len = Len::Unknown(None);
                } else if changes_length || matches!(term, Term::Compact(_, _)) {
                    fact.len = Len::Scan(owner.into());
                    *fact.shape.last_mut().expect("row level") = fact.len.clone();
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
                let rows = fact.shape.last_mut().expect("row level");
                *rows = Len::CeilDiv(Box::new(rows.clone()), *n);
                fact.shape.push(Len::Unknown(Some(*n)));
                Ok(fact)
            }
            Term::Lookup {
                provider,
                keys,
                inner,
            } => {
                let mut fact = self.term(inner, owner)?;
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
                let ordered_left = self
                    .program
                    .structures
                    .physical
                    .program
                    .definitions
                    .iter()
                    .find_map(|definition| {
                        if let Operator::Merge(_, keyed) = &definition.operator {
                            let mut columns: Vec<_> =
                                keyed.columns.iter().map(symbol_name).collect();
                            columns.sort();
                            if fact.meaning == Meaning::Result(symbol_name(&keyed.left))
                                && symbol_name(&keyed.right) == provider_name
                                && columns == names
                            {
                                return Some(keyed.left.clone());
                            }
                        }
                        None
                    });
                let capability = if self
                    .analysis
                    .structures
                    .membership
                    .contains(&(provider_name.clone(), names.clone()))
                {
                    Capability::Membership
                } else if ordered_left.is_some() {
                    Capability::Ordered
                } else {
                    Capability::Exact
                };
                self.provider(provider, &names, capability)?;
                if let Some(left) = ordered_left {
                    // The ordered left stream is already carried by this term;
                    // only the independently probed right needs a prior store.
                    let left_name = symbol_name(&left);
                    let ordered_stream = self.analysis.structures.physical.modes[&left_name]
                        == ModeKind::Pipe
                        && matches!(&self.physical(&left)?.operator, Operator::Merge(_, keyed)
                        if {
                            let mut columns: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
                            columns.sort();
                            columns == names
                        });
                    if ordered_stream {
                        // Structures has checked this producing merge's order.
                        // A carried stream needs no new indexed-provider trait.
                        self.analysis
                            .capabilities
                            .insert((left_name, names.clone()), Capability::Ordered);
                    } else {
                        self.provider_capability(&left, &names, Capability::Ordered)?;
                    }
                }
                fact.reads.push((provider_name.clone(), names.clone()));
                fact.meaning = Meaning::Lookup(provider_name, names, Box::new(fact.meaning));
                fact.depth += 1;
                fact.max_depth += 1;
                fact.shape.push(Len::Unknown(None));
                fact.len = Len::Unknown(None);
                Ok(fact)
            }
            Term::Flatten(inner) => {
                // Formal rule BE-Offsets: Flatten requires nested positions and records their exclusive scan.
                let mut fact = self.term(inner, owner)?;
                if fact.depth == 0 {
                    return fail(term, "BE-Offsets", "flatten requires nested positions");
                }
                // Only segmented flattening needs indexed bucket offsets.
                // A single row stream enumerates exact buckets sequentially.
                if fact.depth > 1 {
                    self.range_reads(&fact)?;
                } else {
                    for (provider, keys) in &fact.reads {
                        let name = Ident::new(provider, proc_macro2::Span::call_site());
                        self.provider(&name, keys, Capability::Exact)?;
                    }
                }
                fact.depth -= 1;
                fact.max_depth -= 1;
                fact.len = Len::Scan(owner.into());
                fact.shape.pop();
                *fact.shape.last_mut().expect("flatten leaves a row level") = fact.len.clone();
                if let Meaning::Lookup(provider, keys, inner) = &fact.meaning {
                    if let Meaning::Result(left) = inner.as_ref() {
                        if let Some(definition) = self.program.structures.physical.program.definitions.iter().find(|definition| {
                            matches!(&definition.operator, Operator::Probe(_, keyed) | Operator::Merge(_, keyed)
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
                fact.len = match fact.shape.len() {
                    1 => Len::Lit(1),
                    2 => fact.shape[0].clone(),
                    _ => Len::Scan(owner.into()),
                };
                *fact.shape.last_mut().expect("row level") = Len::Lit(1);
                Ok(fact)
            }
            Term::Canon(inner) => {
                // Formal rule BE-Canon: Canonicalize innermost rows; providers remain trait obligations.
                let mut fact = self.term(inner, owner)?;
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
                *fact.shape.last_mut().expect("row level") = fact.len.clone();
                Ok(fact)
            }
            Term::Store { capacity, inner } => {
                // Formal rule BE-Length: Host stores may grow; separated unknown stores require capacity.
                let mut fact = self.term(inner, owner)?;
                if matches!(fact.len, Len::Unknown(_)) {
                    let carried = match fact.len {
                        Len::Unknown(value) => value,
                        _ => None,
                    };
                    let capacity = capacity.or(carried);
                    fact.len = Len::Unknown(capacity);
                    *fact.shape.last_mut().expect("row level") = fact.len.clone();
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
                        previous.shape.truncate(next.shape.len());
                        for (left, right) in previous.shape.iter_mut().zip(&next.shape) {
                            if left != right {
                                *left = Len::Unknown(None);
                            }
                        }
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

    // Formal rule BE-Bind: Reads record exactly the provider capability their execution requires.
    fn provider(
        &mut self,
        provider: &Ident,
        keys: &[String],
        required: Capability,
    ) -> syn::Result<()> {
        self.require_store(provider)?;
        self.provider_capability(provider, keys, required)
    }

    fn provider_capability(
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
