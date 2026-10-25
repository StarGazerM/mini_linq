//! Structural Sequence laws and construction-based rewrite equality.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_structures::physical::{self, ModeKind, Operator};
use cq_syntax::symbol_name;
use quote::ToTokens;
use syn::Ident;

use crate::{Analysis, Capability, Guard, Len, Monoid, Obligation, OutputProtocol, Program, Term};

#[path = "lookup_semantics.rs"]
mod lookup_semantics;

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
    /// A key proved for these positions, not merely for their relational support.
    unique: Option<Vec<String>>,
    reads: BTreeSet<(String, Vec<String>)>,
    /// Routing columns whose equal values inhabit exactly one outer part.
    partition: Option<Vec<String>>,
    /// Outer positions are a checked Tile or key-disjoint cover.
    partitioned: bool,
}

struct Checked<'a> {
    program: &'a Program,
    analysis: Analysis,
    facts: BTreeMap<String, Fact>,
    stored_results: BTreeSet<String>,
    result_keys: BTreeMap<String, (String, Vec<String>)>,
    stored_result_keys: BTreeMap<String, (String, Vec<String>)>,
    prefix: &'a StructuresProof,
    support: &'a lookup_semantics::Context,
}

type LookupIndex = BTreeMap<String, BTreeMap<Vec<String>, Vec<(usize, String)>>>;

struct StructuresProof {
    program: cq_lang_structures::Program,
    analysis: cq_lang_structures::Analysis,
    physical: BTreeMap<String, usize>,
    lookups: LookupIndex,
    queries: std::rc::Rc<lookup_semantics::Queries>,
}

struct CheckedProof {
    prefix: std::rc::Rc<StructuresProof>,
    sequence: crate::Sequence,
    evidence: crate::Evidence,
    analysis: std::rc::Rc<Analysis>,
    facts: std::rc::Rc<BTreeMap<String, Fact>>,
}

/// Proof context shared by every checked rewrite in a planning invocation.
#[derive(Default)]
pub struct Checker {
    support: lookup_semantics::Context,
    structures: std::cell::RefCell<Vec<std::rc::Rc<StructuresProof>>>,
    proofs: std::cell::RefCell<Vec<std::rc::Rc<CheckedProof>>>,
}

impl Checker {
    fn remember(&self, proof: CheckedProof) -> std::rc::Rc<CheckedProof> {
        let mut proofs = self.proofs.borrow_mut();
        // Keep the invocation's original source while exploring alternatives.
        if proofs.len() == 3 {
            proofs.remove(1);
        }
        let proof = std::rc::Rc::new(proof);
        proofs.push(proof.clone());
        proof
    }

    /// Seed an invocation with its checked source. Opaque syntax stays in the
    /// prefix proof; subsequent Sequence proofs refer to that interned identity.
    pub fn new(program: &Program) -> syn::Result<Self> {
        let mut checker = Self::default();
        checker.check_shared(program)?;
        Ok(checker)
    }

    fn structures(
        &self,
        program: &cq_lang_structures::Program,
    ) -> syn::Result<std::rc::Rc<StructuresProof>> {
        if let Some(proof) = self
            .structures
            .borrow()
            .iter()
            .rev()
            .find(|proof| proof.program == *program)
        {
            return Ok(proof.clone());
        }
        let mut lookups = LookupIndex::new();
        for (index, definition) in program.physical.program.definitions.iter().enumerate() {
            if let Operator::Probe(_, keyed) | Operator::Merge(_, keyed) = &definition.operator {
                let mut keys: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
                keys.sort();
                lookups
                    .entry(symbol_name(&keyed.right))
                    .or_default()
                    .entry(keys)
                    .or_default()
                    .push((index, symbol_name(&keyed.left)));
            }
        }
        let analysis = cq_lang_structures::contract::check(program)?;
        let queries = self.support.queries(&program.physical, &analysis.physical);
        let proof = std::rc::Rc::new(StructuresProof {
            program: program.clone(),
            analysis,
            physical: program
                .physical
                .program
                .definitions
                .iter()
                .enumerate()
                .map(|(i, definition)| (symbol_name(&definition.name), i))
                .collect(),
            lookups,
            queries,
        });
        let mut cache = self.structures.borrow_mut();
        // Preserve the invocation's source and the two most recent prefixes.
        if cache.len() == 3 {
            cache.remove(1);
        }
        cache.push(proof.clone());
        Ok(proof)
    }

    /// Borrow immutable checked facts without cloning their complete analysis.
    /// Only structural facts are shared; machine grading is always rechecked.
    pub fn check_shared(&mut self, program: &Program) -> syn::Result<std::rc::Rc<Analysis>> {
        analyze(program, self).map(|checked| checked.analysis.clone())
    }

    pub fn check(&mut self, program: &Program) -> syn::Result<Analysis> {
        self.check_shared(program)
            .map(|analysis| (*analysis).clone())
    }

    pub fn check_rewrite_in(&mut self, source: &Program, target: &Program) -> syn::Result<()> {
        check_rewrite_in(source, target, self)
    }

    pub fn check_grading(
        &mut self,
        program: &Program,
        machine: &crate::Machine,
    ) -> syn::Result<Analysis> {
        let analysis = self.check(program)?;
        crate::grading::check_grading_in(program, machine, analysis)
    }
}

fn fail<T>(term: &impl ToTokens, rule: &str, message: &str) -> syn::Result<T> {
    Err(syn::Error::new_spanned(term, format!("{rule}: {message}")))
}

/// Check the complete Structures prefix and derive the Sequence facts.
pub fn check(program: &Program) -> syn::Result<Analysis> {
    Checker::default().check(program)
}

fn analyze(program: &Program, context: &Checker) -> syn::Result<std::rc::Rc<CheckedProof>> {
    let prefix = context.structures(&program.structures)?;
    analyze_in(program, context, prefix)
}

fn analyze_in(
    program: &Program,
    context: &Checker,
    prefix: std::rc::Rc<StructuresProof>,
) -> syn::Result<std::rc::Rc<CheckedProof>> {
    let previous = context
        .proofs
        .borrow()
        .iter()
        .rev()
        .find(|proof| {
            std::rc::Rc::ptr_eq(&proof.prefix, &prefix)
                && proof.sequence == program.sequence
                && proof.evidence == program.evidence
        })
        .cloned();
    if let Some(proof) = previous {
        return Ok(proof);
    }
    let mut checker = Checked {
        program,
        analysis: Analysis {
            structures: prefix.analysis.clone(),
            evidence: program.evidence.clone(),
            results: BTreeMap::new(),
            lengths: BTreeMap::new(),
            shapes: BTreeMap::new(),
            tilings: BTreeMap::new(),
            partial_groups: BTreeMap::new(),
            views: BTreeSet::new(),
            stores: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            provider_reads: BTreeMap::new(),
            levels: BTreeMap::new(),
            dispatches: Vec::new(),
            resources: BTreeMap::new(),
            collectives: Vec::new(),
            output: None,
            obligations: Vec::new(),
        },
        facts: BTreeMap::new(),
        stored_results: BTreeSet::new(),
        result_keys: BTreeMap::new(),
        stored_result_keys: BTreeMap::new(),
        prefix: &prefix,
        support: &context.support,
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
        let mut fact = checker.term(&definition.term, &name)?;
        if checker
            .analysis
            .structures
            .physical
            .headings
            .contains_key(&name)
            && checker.same_meaning(&fact.meaning, &Meaning::Result(name.clone()))
        {
            // A support-equal branch can replace a named union endpoint, but
            // its carrier remains a distinct layout. Record the checked target
            // provenance so interpreters rehead rows rather than reevaluating
            // the Physical union. same_meaning also proves identical headings.
            fact.meaning = Meaning::Result(name.clone());
        }
        if let Meaning::Result(result) = &fact.meaning {
            if fact.materializes {
                checker.stored_results.insert(result.clone());
            }
            if let Some(key) = &fact.unique {
                // Match facts.values().find_map's lexical definition order,
                // independently for all facts and actual publications.
                let indices = std::iter::once(&mut checker.result_keys)
                    .chain(fact.materializes.then_some(&mut checker.stored_result_keys));
                for index in indices {
                    let entry = index
                        .entry(result.clone())
                        .or_insert_with(|| (name.clone(), key.clone()));
                    if name < entry.0 {
                        *entry = (name.clone(), key.clone());
                    }
                }
            }
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
    program.evidence.check(program, &checker.analysis)?;
    Ok(context.remember(CheckedProof {
        prefix: prefix.clone(),
        sequence: program.sequence.clone(),
        evidence: program.evidence.clone(),
        analysis: std::rc::Rc::new(checker.analysis),
        facts: std::rc::Rc::new(checker.facts),
    }))
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

impl Checked<'_> {
    fn require_store(&self, name: &Ident) -> syn::Result<()> {
        let result = symbol_name(name);
        if !self
            .analysis
            .structures
            .physical
            .hosts
            .contains_key(&result)
            && !self.stored_results.contains(&result)
        {
            return fail(
                name,
                "BE-Scope",
                "non-host read requires an earlier Sequence store",
            );
        }
        Ok(())
    }

    fn physical(&self, name: &Ident) -> syn::Result<&physical::Definition> {
        self.prefix
            .physical
            .get(&symbol_name(name))
            .map(|&index| &self.program.structures.physical.program.definitions[index])
            .ok_or_else(|| syn::Error::new_spanned(name, "BE-Alpha: unknown Physical result"))
    }

    // Result substitution requires the same heading and both directions of
    // the scoped CQ containment proof. Opaque Rust/selection atoms are never
    // identified merely because their columns or estimated sizes agree.
    fn same_meaning(&self, left: &Meaning, right: &Meaning) -> bool {
        left == right
            || matches!((left, right),
            (Meaning::Result(left), Meaning::Result(right))
            if self.analysis.structures.physical.headings[left]
                == self.analysis.structures.physical.headings[right]
                && self.support.equal_in(
                    &self.prefix.queries,
                    left,
                    right,
                ))
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
    fn inferred_key(&self, name: &Ident, input: Option<&Fact>) -> Option<Vec<String>> {
        let definition = self.physical(name).ok()?;
        // Formal rule BE-GroupKey: Asserted relation keys survive renaming and inherited-key transfer.
        if let Operator::Rename(rename) = &definition.operator {
            for fact in &self.program.evidence.facts {
                if let crate::EvidenceFact::Key { subject, columns } = fact
                    && *subject == rename.relation
                    && let Some(key) = columns
                        .iter()
                        .filter(|column| {
                            !rename.constants.as_ref().is_some_and(|constants| {
                                constants.equalities.iter().any(|equality| {
                                    equality.left == **column
                                        && matches!(
                                            equality.right,
                                            cq_syntax::EqualityValue::Literal(_)
                                        )
                                })
                            })
                        })
                        .map(|column| {
                            rename
                                .mapping
                                .iter()
                                .find(|mapping| mapping.source == *column)
                                .map(|mapping| symbol_name(&mapping.target))
                        })
                        .collect::<Option<Vec<_>>>()
                {
                    return Some(key);
                }
            }
        }
        physical::inherited_key(
            &definition.operator,
            &self.analysis.structures.physical.headings[&symbol_name(name)],
            &self.program.structures.physical.program.host_sets,
            |operand| {
                let result = symbol_name(operand);
                if let Some(input) = input
                    && matches!(&input.meaning, Meaning::Result(name) if *name == result)
                {
                    return input.unique.clone();
                }
                self.result_keys.get(&result).map(|(_, key)| key.clone())
            },
        )
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
                                self.same_meaning(&fact.meaning, &Meaning::Result(symbol_name(operand)))))
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
                result.unique = None;
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
                    unique: match &meaning {
                        Meaning::Result(result) => self
                            .stored_result_keys
                            .get(result)
                            .map(|(_, key)| key.clone())
                            .or_else(|| self.inferred_key(name, None)),
                        _ => self.facts.values().find_map(|fact| {
                            (fact.materializes && fact.meaning == meaning)
                                .then(|| fact.unique.clone())
                                .flatten()
                        }),
                    },
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
                    reads: BTreeSet::new(),
                    partition: None,
                    partitioned: false,
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
                    if !self.same_meaning(&fact.meaning, &Meaning::Result(left))
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
                    self.analysis
                        .provider_reads
                        .entry(owner.into())
                        .or_default()
                        .insert(symbol_name(provider));
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
                let mut segments = Vec::new();
                loop {
                    if self.same_meaning(&source, &Meaning::Result(current.clone())) {
                        break;
                    }
                    let definition = self
                        .prefix
                        .physical
                        .get(&current)
                        .map(|&index| &self.program.structures.physical.program.definitions[index])
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
                    if matches!(operator, Operator::Group(_)) {
                        return fail(
                            name,
                            "BE-Group",
                            "group requires an explicit Sequence group",
                        );
                    }
                    segments.push(definition.name.clone());
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
                    self.analysis
                        .provider_reads
                        .entry(owner.into())
                        .or_default()
                        .insert(symbol_name(&provider));
                    if enumerated {
                        fact.reads.insert((symbol_name(&provider), keys));
                    }
                }
                for segment in segments.iter().rev() {
                    fact.unique = self.inferred_key(segment, Some(&fact));
                    fact.meaning = Meaning::Result(symbol_name(segment));
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
            Term::PartitionK { keys, parts, inner } => {
                if *parts == 0 {
                    return fail(term, "BE-KeyPart", "partition count must be positive");
                }
                let Term::Ref(source) = inner.as_ref() else {
                    return fail(
                        term,
                        "BE-KeyPart",
                        "partition input must name an ancestor store",
                    );
                };
                if !self.analysis.stores.contains_key(&symbol_name(source)) {
                    return fail(
                        term,
                        "BE-KeyPart",
                        "partition input must name an ancestor store",
                    );
                }
                let mut fact = self.term(inner, owner)?;
                let heading = self.heading(&fact.meaning);
                let columns: Vec<_> = keys.iter().map(symbol_name).collect();
                if fact.max_depth != 0
                    || columns.iter().any(|key| !heading.contains(key))
                    || columns.iter().collect::<BTreeSet<_>>().len() != columns.len()
                {
                    return fail(
                        term,
                        "BE-KeyPart",
                        "routing key must be distinct columns determined by the flat valuation",
                    );
                }
                fact.depth = 1;
                fact.max_depth = 1;
                fact.shape = vec![Len::Lit(*parts), fact.len.clone()];
                fact.partition = Some(columns);
                fact.partitioned = true;
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
                fact.partitioned = true;
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
                    .prefix
                    .lookups
                    .get(&provider_name)
                    .and_then(|lookups| lookups.get(&names))
                    .into_iter()
                    .flatten()
                    .find_map(|(index, input)| {
                        let definition =
                            &self.program.structures.physical.program.definitions[*index];
                        if let Operator::Merge(_, keyed) = &definition.operator
                            && self.same_meaning(&fact.meaning, &Meaning::Result(input.clone()))
                        {
                            Some(keyed.left.clone())
                        } else {
                            None
                        }
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
                self.analysis
                    .provider_reads
                    .entry(owner.into())
                    .or_default()
                    .insert(symbol_name(provider));
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
                fact.reads.insert((provider_name.clone(), names.clone()));
                fact.unique = fact.unique.take().and_then(|mut key| {
                    let other = self
                        .result_keys
                        .get(&provider_name)
                        .map(|(_, key)| key.clone())
                        .or_else(|| self.inferred_key(provider, None))?;
                    key.extend(other);
                    key.sort();
                    key.dedup();
                    Some(key)
                });
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
                // Nesting alone does not require random bucket offsets:
                // a leaf task enumerates each bucket sequentially. Grading
                // adds Range only for collective scans or distributing a
                // flattened bucket stream across executors.
                for (provider, keys) in &fact.reads {
                    let name = Ident::new(provider, proc_macro2::Span::call_site());
                    self.provider(&name, keys, Capability::Exact)?;
                }
                fact.depth -= 1;
                fact.max_depth -= 1;
                fact.len = Len::Scan(owner.into());
                fact.shape.pop();
                *fact.shape.last_mut().expect("flatten leaves a row level") = fact.len.clone();
                if fact.depth == 0 {
                    fact.partition = None;
                    fact.partitioned = false;
                }
                if let Meaning::Lookup(provider, keys, inner) = &fact.meaning {
                    if let Meaning::Result(_) = inner.as_ref() {
                        if let Some((index, _)) = self
                            .prefix
                            .lookups
                            .get(provider)
                            .and_then(|lookups| lookups.get(keys))
                            .into_iter()
                            .flatten()
                            .find(|(_, input)| {
                                self.same_meaning(inner.as_ref(), &Meaning::Result(input.clone()))
                            })
                        {
                            let definition =
                                &self.program.structures.physical.program.definitions[*index];
                            fact.meaning = Meaning::Result(symbol_name(&definition.name));
                        }
                    }
                }
                Ok(fact)
            }
            Term::Group(name, inner) => {
                // Formal rule BE-Group: Fold each key's complete distinct valuation set at host.
                let group_slot = {
                    let modes = self
                        .analysis
                        .partial_groups
                        .entry(owner.into())
                        .or_default();
                    let slot = modes.len();
                    modes.push(false);
                    slot
                };
                let mut fact = self.term(inner, owner)?;
                let Operator::Group(group) = &self.physical(name)?.operator else {
                    return fail(name, "BE-Group", "group must name a Physical group");
                };
                if fact.meaning != Meaning::Result(symbol_name(&group.input)) {
                    return fail(
                        term,
                        "BE-Group",
                        "group must consume its complete Physical valuation heading",
                    );
                }
                // Formal rule BE-KeyPartGroup: Equal group keys route to one part.
                if let Some(keys) = &fact.partition
                    && !group.keys.is_empty()
                {
                    let group_keys: Vec<_> = group.keys.iter().map(symbol_name).collect();
                    if keys.iter().any(|key| !group_keys.contains(key)) {
                        return fail(
                            term,
                            "BE-KeyPartGroup",
                            "group key must determine the routing key",
                        );
                    }
                }
                // Formal rule BE-GroupPartial: Globally distinct parts fold locally and merge once at host.
                let partial = fact.partitioned
                    && fact.max_depth == 1
                    && (group.keys.is_empty() || fact.partition.is_none());
                if partial && fact.unique.is_none() {
                    return fail(
                        term,
                        "BE-GroupPartial",
                        "partial folds require valuations distinct across parts: a global key or key-routed canon",
                    );
                }
                if !partial && fact.max_depth != usize::from(fact.partition.is_some()) {
                    return fail(
                        term,
                        "BE-Group",
                        "group requires a flat valuation set; flatten partitions first",
                    );
                }
                if fact.unique.is_none() {
                    return fail(
                        term,
                        "BE-Group",
                        "group input must canonicalize complete valuations, not aggregate arguments",
                    );
                }
                fact.len = if group.keys.is_empty() {
                    Len::Lit(1)
                } else {
                    Len::Scan(owner.into())
                };
                if partial {
                    fact.depth = 0;
                    fact.max_depth = 0;
                    fact.partition = None;
                    fact.partitioned = false;
                }
                if fact.partition.is_some() {
                    *fact.shape.last_mut().expect("partition row level") = fact.len.clone();
                } else {
                    fact.shape = vec![fact.len.clone()];
                }
                fact.unique = Some(group.keys.iter().map(symbol_name).collect());
                fact.meaning = Meaning::Result(symbol_name(name));
                fact.materializes = true;
                self.analysis
                    .partial_groups
                    .get_mut(owner)
                    .expect("reserved Group constructor slot")[group_slot] = partial;
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
                fact.unique = (fact.depth == 0).then(Vec::new);
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
                // Formal rule BE-KeyPartCanon: Equal complete valuations route to one part.
                let heading = self.heading(&fact.meaning);
                if fact
                    .partition
                    .as_ref()
                    .is_some_and(|keys| keys.iter().any(|key| !heading.contains(key)))
                {
                    return fail(
                        term,
                        "BE-KeyPartCanon",
                        "complete valuation must determine the routing key",
                    );
                }
                fact.unique = (fact.depth == 0
                    || (fact.depth == 1 && fact.max_depth == 1 && fact.partition.is_some()))
                .then_some(heading);
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
                        if !self.same_meaning(&previous.meaning, &next.meaning) {
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
                        if previous.unique != next.unique {
                            previous.unique = None;
                        }
                        previous.reads.extend(next.reads);
                        previous.partitioned &= next.partitioned;
                        if previous.partition != next.partition {
                            previous.partition = None;
                            previous.unique = None;
                        }
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
        // Formal rule BE-KeyPartProvider: A lookup key determines the provider's routing key.
        for fact in self.facts.values().filter(|fact| {
            fact.materializes && fact.meaning == Meaning::Result(symbol_name(provider))
        }) {
            if fact
                .partition
                .as_ref()
                .is_some_and(|routing| routing.iter().any(|key| !keys.contains(key)))
            {
                return fail(
                    provider,
                    "BE-KeyPartProvider",
                    "lookup key must determine the stored provider routing key",
                );
            }
        }
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
        if self.analysis.capabilities.get(&key).is_some_and(|current| {
            *current == required
                || *current == Capability::Ordered
                || (*current == Capability::Range && required != Capability::Ordered)
                || (*current == Capability::Exact && required == Capability::Membership)
        }) {
            // The complete provider/type/key obligation and its codec premise
            // already hold. Repeated inherited reads add no stronger premise.
            return Ok(());
        }
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

/// Rewrites preserve final support by construction. A changed Structures prefix
/// additionally needs the narrow, evidence-independent join reexpression proof.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    Checker::default().check_rewrite_in(source, target)
}

fn check_rewrite_in(source: &Program, target: &Program, context: &Checker) -> syn::Result<()> {
    // Resolve complete Structures syntax once per distinct endpoint. All
    // subsequent proof comparisons use the interned prefix identity rather
    // than revisiting opaque Rust tokens for each candidate Sequence proof.
    let source_prefix = context.structures(&source.structures)?;
    let target_prefix = if std::ptr::eq(source, target) {
        source_prefix.clone()
    } else {
        context.structures(&target.structures)?
    };
    let unchanged = std::rc::Rc::ptr_eq(&source_prefix, &target_prefix);
    let source_facts = analyze_in(source, context, source_prefix)?;
    if unchanged && source.sequence == target.sequence && source.evidence == target.evidence {
        return Ok(());
    }
    if (!unchanged || source.sequence != target.sequence)
        && target.evidence.facts.iter().any(|fact| {
            matches!(fact, crate::EvidenceFact::Key { .. }) && !source.evidence.facts.contains(fact)
        })
    {
        return fail(
            target,
            "PL-Evidence",
            "a structural rewrite cannot invent key premises; record author evidence first",
        );
    }
    let target_facts = analyze_in(target, context, target_prefix)?;
    let support = &context.support;
    let preserves_adapters = unchanged
        || source.structures.structures.entries.iter().all(|entry| {
            use cq_lang_structures::Structure;
            if matches!(entry.structure, Structure::Built { .. }) {
                return true;
            }
            let definitions = &target.structures.physical.program.definitions;
            if !definitions
                .iter()
                .any(|definition| definition.name == entry.name)
            {
                return true;
            }
            if let Structure::Host { reader, .. } = &entry.structure
                && reader != "scan"
                && (target_facts.analysis.structures.physical.modes[&symbol_name(&entry.name)]
                    != ModeKind::Pipe
                    || !definitions
                        .iter()
                        .any(|definition| definition.name == *reader))
            {
                return true;
            }
            // Catalog retyping and relational reexpression may occur in the
            // same composed pass. Compare the surviving layout, not the
            // selected storage implementation, while keeping readers exact.
            target
                .structures
                .structures
                .entries
                .iter()
                .any(|target| cq_lang_structures::storage_layout_eq(entry, target))
        });
    // Catalog selection may retain host renames and replace their keyed
    // fallbacks, while preserving opaque syntax and the original scans.
    let storage_retyped = !unchanged
        && cq_lang_structures::check_retention_rewrite(&source.structures, &target.structures)
            .is_ok();
    if !unchanged
        && !storage_retyped
        && (source.structures.physical == target.structures.physical
            || !preserves_adapters
            || !lookup_semantics::reexpression(
                &source.structures.physical,
                &source_facts.analysis.structures.physical,
                &target.structures.physical,
                &target_facts.analysis.structures.physical,
                &source_facts.prefix.queries,
                &target_facts.prefix.queries,
                support,
            ))
    {
        return fail(
            target,
            "BE-Alpha",
            "Structures change lacks a checked join reexpression",
        );
    }
    let final_meaning = |program: &Program, facts: &CheckedProof| {
        program
            .sequence
            .definitions
            .last()
            .map(|definition| facts.facts[&symbol_name(&definition.name)].meaning.clone())
    };
    let left = final_meaning(source, &source_facts);
    let right = final_meaning(target, &target_facts);
    let equivalent = if unchanged && left == right {
        true
    } else {
        matches!((&left, &right), (Some(Meaning::Result(left)), Some(Meaning::Result(right)))
        if support.same_result(
            &source_facts.prefix.queries,
            left,
            &target_facts.prefix.queries,
            right,
        ))
    };
    if !equivalent {
        return fail(
            target,
            "BE-Choose",
            "rewrite changes final support rather than a construction view",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_publication_keys_preserve_lexical_fact_order() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(k:i32, value:i64);
                r = pipe canon rename R@0 {k -> k, value -> value};
                grouped = let group r by [k] { n: usize = count(); };
                return grouped as answer(k,n);
            }
            structures {
                r host scan = Reader [];
                r canon = Canon;
                grouped rows = Rows;
            }
            sequence {
                source = canon rows r;
                z_group = group grouped source;
                before = rows grouped;
                a_store = store canon z_group;
                after = rows grouped;
                zero_later = store z_group;
                last = rows grouped;
            }
        };
        let proof = analyze(&program, &Checker::default()).unwrap();
        assert_eq!(proof.facts["before"].unique, Some(vec!["k".into()]));
        for name in ["after", "last"] {
            assert_eq!(proof.facts[name].unique, Some(vec!["k".into(), "n".into()]));
        }
    }

    #[test]
    fn indexed_publication_keys_do_not_promote_unstored_views() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i32);
                r = pipe rename R@0 {a -> a};
                output = pipe canon project r keep {a};
                return output as answer(a);
            }
            structures { r host scan = Reader []; output canon = Canon; }
            sequence {
                a_view = canon rows r;
                before = rows r;
                z_store = store a_view;
                after = rows r;
                output = canon map output after;
            }
        };
        let proof = analyze(&program, &Checker::default()).unwrap();
        assert_eq!(proof.facts["before"].unique, None);
        assert_eq!(proof.facts["after"].unique, Some(vec!["a".into()]));
    }
}
