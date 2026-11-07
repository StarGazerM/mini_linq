//! Structural Sequence laws and construction-based rewrite equality.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_structures::physical::{self, ModeKind, Operator};
use cq_syntax::symbol_name;
use quote::ToTokens;
use syn::Ident;

use crate::{Analysis, Capability, Guard, Len, Monoid, Obligation, OutputProtocol, Program, Term};

#[path = "lookup_semantics.rs"]
mod lookup_semantics;
#[path = "publication.rs"]
mod publication;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Meaning {
    Result(String),
    Host(String),
    Lookup(String, Vec<String>, Box<Meaning>),
    Fold(Monoid, Box<Meaning>),
}

#[derive(Clone)]
struct Fact {
    /// Formal rule BE-Length: Derive pre-write Len once; published unknown lengths become runtime inputs, and separated grading consumes checked store lengths.
    len: Len,
    shape: Vec<Len>,
    meaning: Meaning,
    depth: usize,
    max_depth: usize,
    materializes: bool,
    /// A key proved for these positions, not merely for their relational support.
    unique: Option<Vec<String>>,
    /// Positional key preserved if valuation canonicalizers on this stream are removed.
    canon_free_unique: Option<Vec<String>>,
    reads: BTreeSet<(String, Vec<String>)>,
    /// Routing columns whose equal values inhabit exactly one outer part.
    partition: Option<Vec<String>>,
    /// Outer positions are a checked Tile or key-disjoint cover.
    partitioned: bool,
}

struct Checked<'a> {
    program: &'a Program,
    analysis: Analysis,
    lookup_slots: BTreeMap<*const Term, usize>,
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
    physical_program: physical::Program,
    analysis: cq_lang_structures::Analysis,
    physical: BTreeMap<String, usize>,
    lookups: LookupIndex,
    queries: std::rc::Rc<lookup_semantics::Queries>,
    /// Declared input codec of each logical type (BE-Code); one per type.
    codecs: Vec<(
        cq_lang_structures::LogicalTypeId,
        cq_lang_structures::CodecId,
        syn::Type,
    )>,
}

struct CheckedProof {
    prefix: std::rc::Rc<StructuresProof>,
    sequence: crate::Sequence,
    evidence: crate::Evidence,
    representation: crate::Representation,
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
    pub fn new(physical: &physical::Program, structures: &cq_lang_structures::Program, program: &Program) -> syn::Result<Self> {
        let mut checker = Self::default();
        checker.check_shared(physical, structures, program)?;
        Ok(checker)
    }

    fn structures(
        &self,
        physical: &physical::Program,
        program: &cq_lang_structures::Program,
    ) -> syn::Result<std::rc::Rc<StructuresProof>> {
        if let Some(proof) = self
            .structures
            .borrow()
            .iter()
            .rev()
            .find(|proof| proof.program == *program && proof.physical_program == *physical)
        {
            return Ok(proof.clone());
        }
        let mut lookups = LookupIndex::new();
        for (index, definition) in physical.program.definitions.iter().enumerate() {
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
        let analysis = cq_lang_structures::contract::check(physical, program)?;
        let queries = self.support.queries(physical, &analysis.physical);
        // BE-Code obligations name a column's codec: the one its logical type's
        // input columns declare; differing declarations leave it ambiguous.
        let mut codecs: Vec<(
            cq_lang_structures::LogicalTypeId,
            cq_lang_structures::CodecId,
            syn::Type,
        )> = Vec::new();
        for entry in &program.structures.entries {
            let (cq_lang_structures::Structure::Host { keyed, .. }
            | cq_lang_structures::Structure::Built { keyed, .. }) = &entry.structure
            else {
                continue;
            };
            for column in keyed.provider.input.iter().flat_map(|input| &input.columns) {
                let Some(codec) = &column.codec else {
                    continue;
                };
                let id = column.codec_id.expect("Structures checks codec identities");
                let logical_id = column
                    .logical_type_id
                    .expect("Structures checks logical type identities");
                match codecs.iter().find(|(logical, _, _)| *logical == logical_id) {
                    Some((_, previous, _)) if *previous != id => {
                        return fail(
                            codec,
                            "BE-Code",
                            "conflicting codec identities for one logical type",
                        );
                    }
                    Some(_) => {}
                    None => codecs.push((logical_id, id, codec.clone())),
                }
            }
        }
        let proof = std::rc::Rc::new(StructuresProof {
            program: program.clone(),
            physical_program: physical.clone(),
            analysis,
            physical: physical
                .program
                .definitions
                .iter()
                .enumerate()
                .map(|(i, definition)| (symbol_name(&definition.name), i))
                .collect(),
            lookups,
            queries,
            codecs,
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
    pub fn check_shared(&mut self, physical: &physical::Program, structures: &cq_lang_structures::Program, program: &Program) -> syn::Result<std::rc::Rc<Analysis>> {
        analyze(physical, structures, program, self).map(|checked| checked.analysis.clone())
    }

    pub fn check(&mut self, physical: &physical::Program, structures: &cq_lang_structures::Program, program: &Program) -> syn::Result<Analysis> {
        self.check_shared(physical, structures, program)
            .map(|analysis| (*analysis).clone())
    }

    pub fn check_rewrite_in(&mut self, source_physical: &physical::Program, source_structures: &cq_lang_structures::Program, source: &Program, target_physical: &physical::Program, target_structures: &cq_lang_structures::Program, target: &Program) -> syn::Result<()> {
        check_rewrite_in(source_physical, source_structures, source, target_physical, target_structures, target, self)
    }

    pub fn check_grading(
        &mut self,
        physical: &physical::Program,
        structures: &cq_lang_structures::Program,
        program: &Program,
        machine: &crate::Machine,
    ) -> syn::Result<Analysis> {
        let proof = analyze(physical, structures, program, self)?;
        crate::grading::check_grading_in(physical, structures, program, machine, (*proof.analysis).clone(), &proof.prefix.analysis)
    }
}

fn fail<T>(term: &impl ToTokens, rule: &str, message: &str) -> syn::Result<T> {
    Err(syn::Error::new_spanned(term, format!("{rule}: {message}")))
}

/// Check a Sequence annotation against its separately supplied base programs.
pub fn check(physical: &physical::Program, structures: &cq_lang_structures::Program, program: &Program) -> syn::Result<Analysis> {
    Checker::default().check(physical, structures, program)
}

fn check_native_read(
    provider: &Ident,
    read: cq_syntax::host::HostRead,
    required: &Capability,
) -> syn::Result<()> {
    use cq_syntax::host::HostRead;
    // Formal rule BE-NativeBind: only the lookup's own recorded read can
    // discharge its capability, and Sequence must be able to print that read.
    let covers = match required {
        Capability::Membership => matches!(read, HostRead::Membership | HostRead::Exact),
        Capability::Exact => read == HostRead::Exact,
        Capability::Range => matches!(read, HostRead::Range | HostRead::Ordered),
        Capability::Ordered => read == HostRead::Ordered,
        Capability::Candidates => read == HostRead::Candidates,
        Capability::Resident | Capability::Distinct | Capability::Positions => false,
    };
    if !covers || !matches!(read, HostRead::Membership | HostRead::Exact) {
        return fail(
            provider,
            "BE-NativeBind",
            "recorded native host read cannot serve this Sequence lookup capability",
        );
    }
    Ok(())
}

pub(crate) fn check_native_grading(structures: &cq_lang_structures::Analysis, analysis: &Analysis) -> syn::Result<()> {
    for (key, required) in &analysis.lookup_capabilities {
        if let Some(native) = structures.natives.get(key) {
            let provider = Ident::new(&key.0, proc_macro2::Span::call_site());
            check_native_read(&provider, native.read, required)?;
        }
    }
    Ok(())
}

fn analyze(physical: &physical::Program, structures: &cq_lang_structures::Program, program: &Program, context: &Checker) -> syn::Result<std::rc::Rc<CheckedProof>> {
    let prefix = context.structures(physical, structures)?;
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
                && proof.representation == program.representation
        })
        .cloned();
    if let Some(proof) = previous {
        return Ok(proof);
    }
    crate::blocks::check(&prefix.physical_program, &prefix.program, program)?;
    let representation = &program.representation;
    if !representation.identity.is_disjoint(&representation.encoded) {
        return fail(
            program,
            "BE-RustRepresentation",
            "a column cannot be both identity and encoded",
        );
    }
    for column in representation
        .identity
        .iter()
        .chain(&representation.encoded)
    {
        if !prefix
            .analysis
            .physical
            .rows
            .values()
            .any(|row| row.types.contains_key(column))
        {
            return fail(
                program,
                "BE-RustRepresentation",
                "unknown representation column",
            );
        }
    }
    let mut checker = Checked {
        program,
        lookup_slots: crate::lookup_slots(&program.sequence),
        analysis: Analysis {
            evidence: program.evidence.clone(),
            representation: program.representation.clone(),
            results: BTreeMap::new(),
            lengths: BTreeMap::new(),
            shapes: BTreeMap::new(),
            tilings: BTreeMap::new(),
            partial_groups: BTreeMap::new(),
            prefix_bounds: crate::bounds::record(&prefix.physical_program, program),
            canon_free_groups: BTreeSet::new(),
            views: BTreeSet::new(),
            stores: BTreeMap::new(),
            publications: BTreeMap::new(),
            task_execution: Default::default(),
            gpu_bindings: Default::default(),
            gpu_publications: BTreeMap::new(),
            host_sources: BTreeMap::new(),
            host_tiles: BTreeMap::new(),
            routed_providers: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            lookup_capabilities: BTreeMap::new(),
            lookup_readers: BTreeMap::new(),
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
    crate::bounds::check(&prefix.physical_program, program, &checker.analysis.prefix_bounds)?;
    let mut publication_facts = BTreeMap::new();
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
        if matches!(definition.term, Term::Store { .. }) {
            publication_facts.insert(name.clone(), publication::Input::from(&fact));
        }
        if checker.prefix.analysis
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
    publication::derive(&prefix.physical_program, &prefix.analysis, program, &mut checker.analysis, &publication_facts)?;
    checker.analysis.task_execution = crate::task_execution::analyze(&prefix.physical_program, &prefix.analysis, program, &checker.analysis);
    crate::blocks::check_task_covers(&prefix.physical_program, &prefix.program, program, &prefix.analysis, &checker.analysis)?;
    program.evidence.check(&prefix.physical_program, &prefix.analysis, &checker.analysis)?;
    Ok(context.remember(CheckedProof {
        prefix: prefix.clone(),
        sequence: program.sequence.clone(),
        evidence: program.evidence.clone(),
        representation: program.representation.clone(),
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
        if !self.prefix.analysis
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
            .map(|&index| &self.prefix.physical_program.program.definitions[index])
            .ok_or_else(|| syn::Error::new_spanned(name, "BE-Alpha: unknown Physical result"))
    }

    // Result substitution requires the same heading and both directions of
    // the scoped CQ containment proof. Opaque Rust/selection atoms are never
    // identified merely because their columns or estimated sizes agree.
    fn same_meaning(&self, left: &Meaning, right: &Meaning) -> bool {
        left == right
            || matches!((left, right),
            (Meaning::Result(left), Meaning::Result(right))
            if self.prefix.analysis.physical.headings[left]
                == self.prefix.analysis.physical.headings[right]
                && self.support.equal_in(
                    &self.prefix.queries,
                    left,
                    right,
                ))
    }

    fn heading(&self, meaning: &Meaning) -> Vec<String> {
        match meaning {
            Meaning::Result(name) => self.prefix.analysis.physical.headings[name].clone(),
            Meaning::Host(name) => self.prefix.analysis
                .host_layouts
                .get(name)
                .cloned()
                .unwrap_or_default(),
            Meaning::Lookup(provider, _, inner) => {
                let mut heading = self.heading(inner);
                heading.extend(
                    self.prefix.analysis.physical.headings[provider]
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
    /// Record one BE-Code obligation per exact codec over these
    /// columns. A column's codec is the declared input codec of its recorded
    /// logical type (`None`: no declared codec, the identity representation).
    /// Codec injectivity and monotonicity are deliberately not claimed statically.
    fn codec_obligations(&mut self, columns: Vec<String>, ordered: bool) {
        let mut groups: Vec<(
            Option<cq_lang_structures::CodecId>,
            Option<syn::Type>,
            Vec<String>,
        )> = Vec::new();
        for column in columns {
            let declared = self.prefix.analysis
                .physical
                .rows
                .values()
                .find_map(|row| row.logical_type_ids.get(&column))
                .and_then(|logical| {
                    self.prefix
                        .codecs
                        .iter()
                        .find(|(declared, _, _)| declared == logical)
                });
            let codec_id = declared.map(|(_, id, _)| *id);
            match groups
                .iter_mut()
                .find(|(recorded, _, _)| *recorded == codec_id)
            {
                Some((_, _, columns)) => columns.push(column),
                None => groups.push((
                    codec_id,
                    declared.map(|(_, _, codec)| codec.clone()),
                    vec![column],
                )),
            }
        }
        for (codec_id, codec, columns) in groups {
            self.analysis.obligations.push(Obligation::Codec {
                codec_id,
                columns,
                ordered,
                codec,
            });
        }
    }
    fn inferred_key(
        &self,
        name: &Ident,
        input: Option<(&Meaning, &Option<Vec<String>>)>,
    ) -> Option<Vec<String>> {
        let definition = self.physical(name).ok()?;
        // Formal rule BE-GroupKey: Explicitly trusted keys survive renaming and inherited-key transfer.
        if let Operator::Rename(rename) = &definition.operator {
            for fact in &self.program.evidence.facts {
                if let crate::EvidenceFact::TrustedKey {
                    subject, columns, ..
                } = fact
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
            &self.prefix.analysis.physical.headings[&symbol_name(name)],
            &self.prefix.physical_program.program.host_sets,
            |operand| {
                let result = symbol_name(operand);
                if let Some((meaning, unique)) = input
                    && matches!(meaning, Meaning::Result(name) if *name == result)
                {
                    return unique.clone();
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
                let definition = self.prefix.physical_program
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
                result.canon_free_unique = None;
                Ok(result)
            }
            Term::Rows(name) => {
                let name_string = symbol_name(name);
                let physical = &self.prefix.analysis.physical;
                let meaning = if physical.headings.contains_key(&name_string) {
                    if !physical.hosts.contains_key(&name_string)
                        && physical.modes[&name_string] != ModeKind::Let
                        && !self.prefix.analysis.retained.contains_key(&name_string)
                        && !matches!(self.physical(name)?.operator, Operator::Unit(_))
                    {
                        return fail(name, "BE-Alpha", "rows requires a retained result or host");
                    }
                    if !matches!(self.physical(name)?.operator, Operator::Unit(_)) {
                        self.require_store(name)?;
                    }
                    Meaning::Result(name_string.clone())
                } else if self.prefix.analysis
                    .host_layouts
                    .contains_key(&name_string)
                {
                    Meaning::Host(name_string.clone())
                } else {
                    return fail(name, "BE-Alpha", "rows names no retained result or host");
                };
                let unique = match &meaning {
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
                };
                Ok(Fact {
                    canon_free_unique: unique.clone(),
                    unique,
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
                    if fact.materializes && matches!(fact.len, Len::Unknown(_)) {
                        fact.len = Len::Input(symbol_name(name));
                        *fact.shape.last_mut().expect("row level") = fact.len.clone();
                    }
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
                            if self.prefix.analysis.membership.contains(&(
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
                    self.provider(provider, &columns, Capability::Membership, Some(name))?;
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
                        .map(|&index| &self.prefix.physical_program.program.definitions[index])
                        .ok_or_else(|| {
                            syn::Error::new_spanned(
                                name,
                                "BE-Alpha: map does not extend its input result",
                            )
                        })?;
                    let operator = &definition.operator;
                    // Opaque total Rust is legal here; BE-RustRepresentation
                    // checks device identity representations during grading.
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
                            provider_reads.push((
                                keyed.right.clone(),
                                keys,
                                capability,
                                definition.name.clone(),
                            ));
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
                for (provider, keys, capability, reader) in provider_reads {
                    let enumerated = matches!(capability, Capability::Exact | Capability::Ordered);
                    self.provider(&provider, &keys, capability, Some(&reader))?;
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
                    fact.canon_free_unique =
                        self.inferred_key(segment, Some((&fact.meaning, &fact.canon_free_unique)));
                    fact.unique = self.inferred_key(segment, Some((&fact.meaning, &fact.unique)));
                    fact.meaning = Meaning::Result(symbol_name(segment));
                }
                fact.meaning = Meaning::Result(symbol_name(name));
                self.codec_obligations(self.heading(&fact.meaning), ordered);
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
                let heading = &self.prefix.analysis.physical.headings[&provider_name];
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
                let reader = self
                    .prefix
                    .lookups
                    .get(&provider_name)
                    .and_then(|lookups| lookups.get(&names))
                    .into_iter()
                    .flatten()
                    .filter_map(|(index, input)| {
                        let definition =
                            &self.prefix.physical_program.program.definitions[*index];
                        self.same_meaning(&fact.meaning, &Meaning::Result(input.clone()))
                            .then_some(definition)
                    })
                    .min_by_key(|definition| !matches!(definition.operator, Operator::Merge(..)));
                let ordered_left = reader.and_then(|definition| {
                    if let Operator::Merge(_, keyed) = &definition.operator {
                        Some(keyed.left.clone())
                    } else {
                        None
                    }
                });
                let reader = reader.map(|definition| definition.name.clone());
                if let Some(reader) = &reader {
                    self.analysis.lookup_readers.insert(
                        self.lookup_slots[&(term as *const Term)],
                        cq_lang_structures::ReaderRef::Result(symbol_name(reader)),
                    );
                }
                let capability = if self.prefix.analysis
                    .membership
                    .contains(&(provider_name.clone(), names.clone()))
                {
                    Capability::Membership
                } else if ordered_left.is_some() {
                    Capability::Ordered
                } else {
                    Capability::Exact
                };
                self.provider(provider, &names, capability, reader.as_ref())?;
                self.analysis
                    .provider_reads
                    .entry(owner.into())
                    .or_default()
                    .insert(symbol_name(provider));
                if let Some(left) = ordered_left {
                    // The ordered left stream is already carried by this term;
                    // only the independently probed right needs a prior store.
                    let left_name = symbol_name(&left);
                    let ordered_stream = self.prefix.analysis.physical.modes[&left_name]
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
                        self.provider_capability(
                            &left,
                            &names,
                            Capability::Ordered,
                            reader.as_ref(),
                        )?;
                    }
                }
                fact.reads.insert((provider_name.clone(), names.clone()));
                let other = self
                    .result_keys
                    .get(&provider_name)
                    .map(|(_, key)| key.clone())
                    .or_else(|| self.inferred_key(provider, None));
                let combine = |key: Option<Vec<String>>| {
                    key.and_then(|mut key| {
                        let other = other.clone()?;
                        key.extend(other);
                        key.sort();
                        key.dedup();
                        Some(key)
                    })
                };
                fact.unique = combine(fact.unique.take());
                fact.canon_free_unique = combine(fact.canon_free_unique.take());
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
                    self.provider(&name, keys, Capability::Exact, None)?;
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
                                &self.prefix.physical_program.program.definitions[*index];
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
                if group_slot == 0 && fact.canon_free_unique.is_some() {
                    self.analysis.canon_free_groups.insert(owner.into());
                }
                // Canon removal does not cross a Group boundary.
                fact.canon_free_unique = fact.unique.clone();
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
                // Canon removal does not cross a Fold boundary.
                fact.canon_free_unique = fact.unique.clone();
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
                let canonical = match &fact.meaning {
                    Meaning::Result(name) => self.prefix.analysis.canonical.get(name).cloned(),
                    _ => None,
                };
                // Formal rule BE-CanonOrdered: the proof carrier belongs to this
                // direct source, not to an arbitrary stream with the same schema.
                if matches!(canonical, Some(cq_lang_structures::CanonSource::Host))
                    && !matches!(inner.as_ref(), Term::Rows(_))
                {
                    return fail(
                        term,
                        "BE-CanonOrdered",
                        "a canonical host carrier requires direct source rows",
                    );
                }
                // The returned answer boundary reads its recorded identity index.
                let answer_index = match (&canonical, &fact.meaning) {
                    (None, Meaning::Result(name)) => {
                        self.prefix.analysis.head_dedup.get(name).cloned()
                    }
                    _ => None,
                };
                self.analysis.obligations.push(Obligation::Canon {
                    result: owner.into(),
                    canonical,
                    answer_index,
                    level: None,
                });
                self.codec_obligations(self.heading(&fact.meaning), false);
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
                let mut fact = self.term(inner, owner)?;
                if fact.partitioned
                    && let Meaning::Result(result) = &fact.meaning
                    && self.prefix.analysis.selections.contains_key(result)
                {
                    return fail(
                        term,
                        "BE-KeyPartProvider",
                        "a native position snapshot requires its selected carrier",
                    );
                }
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
                // Formal rule BE-Choose: Scheduling guards are exhaustive and alternatives have equal support.
                if alternatives.is_empty() || !self.exhaustive(alternatives)? {
                    return fail(term, "BE-Choose", "scheduling guards are not exhaustive");
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
                let mut result = result.expect("nonempty alternatives checked");
                // Canon removal does not cross a Choose boundary.
                result.canon_free_unique = result.unique.clone();
                Ok(result)
            }
        }
    }

    // Formal rule BE-Bind: Reads record exactly the provider capability their execution requires.
    fn provider(
        &mut self,
        provider: &Ident,
        keys: &[String],
        required: Capability,
        reader: Option<&Ident>,
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
        self.provider_capability(provider, keys, required, reader)
    }

    fn provider_capability(
        &mut self,
        provider: &Ident,
        keys: &[String],
        required: Capability,
        reader: Option<&Ident>,
    ) -> syn::Result<()> {
        let name = symbol_name(provider);
        let key = (name.clone(), keys.to_vec());
        // Whole-relation access was established by Structures; no keyed trait
        // is required. Retention was checked by provider().
        if self.prefix.analysis
            .read_accesses
            .get(&name)
            .and_then(|reads| reads.get(keys))
            == Some(&cq_lang_structures::ReadAccess::Whole)
        {
            self.analysis.capabilities.insert(key, Capability::Exact);
            return Ok(());
        }
        if let Some(reader) = reader {
            let requirement = self
                .analysis
                .lookup_capabilities
                .entry((
                    name.clone(),
                    cq_lang_structures::ReaderRef::Result(symbol_name(reader)),
                    keys.to_vec(),
                ))
                .or_insert(required.clone());
            if *requirement == Capability::Membership && required == Capability::Exact {
                *requirement = Capability::Exact;
            }
        }
        // Check the actual reader before using a capability cached for another
        // reader or a retained provider recorded for the same result and keys.
        let native = reader.and_then(|reader| {
            self.prefix.analysis.natives.get(&(
                name.clone(),
                cq_lang_structures::ReaderRef::Result(symbol_name(reader)),
                keys.to_vec(),
            ))
        });
        if let Some(native) = native {
            check_native_read(provider, native.read, &required)?;
            let capability = self
                .analysis
                .capabilities
                .entry(key)
                .or_insert(required.clone());
            if required == Capability::Exact && *capability == Capability::Membership {
                *capability = Capability::Exact;
            }
            return Ok(());
        }
        if let Some(reader) = reader
            && self.prefix.analysis.built_on(&name, keys).is_none()
            && !self.prefix.analysis.keysets.contains_key(&key)
            && !self.prefix.analysis.fallbacks.contains_key(&(
                name.clone(),
                cq_lang_structures::ReaderRef::Result(symbol_name(reader)),
                keys.to_vec(),
            ))
        {
            return fail(
                provider,
                "BE-NativeBind",
                "lookup has no native read or fallback for its own reader",
            );
        }
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
        if required != Capability::Membership && self.prefix.analysis.membership.contains(&key)
        {
            return fail(
                provider,
                "BE-Bind",
                "enumeration requires Exact/Range/Ordered, not Membership capability",
            );
        }
        let ty = self.prefix.analysis
            .built_on(&name, keys)
            .map(|(ty, _)| ty.clone())
            .or_else(|| {
                let operator = |consumer: &str| {
                    self.prefix.physical_program
                        .program
                        .definitions
                        .iter()
                        .find(|definition| symbol_name(&definition.name) == consumer)
                        .map(|definition| &definition.operator)
                };
                let fallbacks = self.prefix.analysis.fallbacks.iter().filter(
                    |((result, reader, columns), _)| {
                        *result == name
                            && columns == keys
                            && match reader {
                                cq_lang_structures::ReaderRef::Scan => false,
                                cq_lang_structures::ReaderRef::Candidates
                                | cq_lang_structures::ReaderRef::Membership => true,
                                cq_lang_structures::ReaderRef::Result(consumer) => {
                                    operator(consumer).is_some_and(|operator| {
                                        !matches!(
                                            operator,
                                            Operator::Point(..) | Operator::Range(..)
                                        )
                                    })
                                }
                            }
                    },
                );
                // Formal rule BE-OrderedFallback: an ordered read of a host
                // result is typed by the fallback recorded for its merge
                // reader, not by another reader's (e.g. membership) fallback.
                let ordered = fallbacks.clone().find(|((_, reader, _), _)| {
                    required == Capability::Ordered
                        && matches!(reader, cq_lang_structures::ReaderRef::Result(consumer)
                            if matches!(operator(consumer), Some(Operator::Merge(..))))
                });
                ordered
                    .or_else(|| fallbacks.clone().next())
                    .map(|(_, ty)| ty.clone())
            })
            .or_else(|| {
                self.prefix.analysis
                    .keysets
                    .get(&key)
                    .map(|keyset| keyset.provider.clone())
            });
        let Some(ty) = ty else {
            return fail(
                provider,
                "BE-Bind",
                "lookup has no provider for this reader and keys",
            );
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
            instance: ty,
        });
        self.codec_obligations(keys.to_vec(), ordered);
        Ok(())
    }

    // Formal rule BE-StatsRead: checked immutable identity and dominating publication.
    fn statistic(&self, read: &crate::stats::Guard) -> syn::Result<()> {
        use cq_lang_structures::stats::{Placement, Unit};
        let reference = &read.reference;
        let Some(publication) = self.prefix.analysis
            .publications
            .get(&symbol_name(&reference.publication))
        else {
            return fail(read, "BE-StatsRead", "unknown statistic publication");
        };
        if publication.reference() != *reference {
            return fail(
                read,
                "BE-StatsRead",
                "stale statistic identity, layout, domain, or unit",
            );
        }
        if publication.statistic.placement != Placement::Host
            || publication.statistic.unit == Unit::CodeBounds
        {
            return fail(
                read,
                "BE-StatsRead",
                "scalar guard requires a host numeric publication",
            );
        }
        let ready = symbol_name(&read.ready);
        let Some(fact) = self.facts.get(&ready) else {
            return fail(
                read,
                "BE-StatsRead",
                "publication must precede the statistic read",
            );
        };
        if !fact.materializes
            || !matches!(&fact.meaning, Meaning::Result(build) if *build == symbol_name(&publication.build))
            || !self.analysis.stores.contains_key(&ready)
            || fact.depth != 0
        {
            return fail(
                read,
                "BE-StatsRead",
                "readiness requires the completed build's Store",
            );
        }
        let Some(definition) = self
            .program
            .sequence
            .definitions
            .iter()
            .find(|definition| definition.name == read.ready)
        else {
            return fail(read, "BE-StatsRead", "missing publication definition");
        };
        if !matches!(&definition.term, Term::Store { .. }) {
            return fail(
                read,
                "BE-StatsRead",
                "readiness must name a publication, not an alias",
            );
        }
        Ok(())
    }

    fn guard_len(&self, len: &Len) -> syn::Result<()> {
        match len {
            Len::Input(name)
                if self.facts.contains_key(name)
                    || self.prefix.analysis
                        .physical
                        .headings
                        .contains_key(name)
                    || self.prefix.analysis.host_layouts.contains_key(name) =>
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
                Guard::WorkersLt(bound) => {
                    if *bound == 0 {
                        return fail(guard, "BE-Choose", "worker guard bound must be positive");
                    }
                }
                Guard::Statistic(read) => {
                    self.statistic(read)?;
                    if !matches!(alternatives.get(index + 1), Some((Guard::Else, _))) {
                        return fail(
                            guard,
                            "BE-StatsRead",
                            "statistic reads must be immediately followed by else",
                        );
                    }
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

/// Rewrites preserve final support by construction and preserve the Physical
/// plan, except for independently checked catalog retention.
pub fn check_rewrite(source_physical: &physical::Program, source_structures: &cq_lang_structures::Program, source: &Program, target_physical: &physical::Program, target_structures: &cq_lang_structures::Program, target: &Program) -> syn::Result<()> {
    Checker::default().check_rewrite_in(source_physical, source_structures, source, target_physical, target_structures, target)
}

fn check_rewrite_in(source_physical: &physical::Program, source_structures: &cq_lang_structures::Program, source: &Program, target_physical: &physical::Program, target_structures: &cq_lang_structures::Program, target: &Program, context: &Checker) -> syn::Result<()> {
    if !source
        .representation
        .identity
        .is_subset(&target.representation.identity)
        || !source
            .representation
            .encoded
            .is_subset(&target.representation.encoded)
    {
        return fail(
            target,
            "BE-RustRepresentation",
            "rewrites may add but must preserve representation facts",
        );
    }
    // Resolve complete Structures syntax once per distinct endpoint. All
    // subsequent proof comparisons use the interned prefix identity rather
    // than revisiting opaque Rust tokens for each candidate Sequence proof.
    let source_prefix = context.structures(source_physical, source_structures)?;
    let target_prefix = context.structures(target_physical, target_structures)?;
    let unchanged = std::rc::Rc::ptr_eq(&source_prefix, &target_prefix);
    let source_facts = analyze_in(source, context, source_prefix)?;
    if unchanged
        && source.sequence == target.sequence
        && source.evidence == target.evidence
        && source.representation == target.representation
    {
        return Ok(());
    }
    if (!unchanged || source.sequence != target.sequence)
        && target.evidence.facts.iter().any(|fact| {
            matches!(fact, crate::EvidenceFact::TrustedKey { .. })
                && !source.evidence.facts.contains(fact)
        })
    {
        return fail(
            target,
            "PL-Evidence",
            "a structural rewrite cannot invent trusted key premises; record authored evidence first",
        );
    }
    let target_facts = analyze_in(target, context, target_prefix)?;
    let support = &context.support;
    // Catalog selection may retain host renames and replace their keyed
    // fallbacks, while preserving opaque syntax and the original scans.
    let storage_retyped = !unchanged
        && cq_lang_structures::check_retention_rewrite(source_physical, source_structures, target_physical, target_structures)
            .is_ok();
    // Formal rule BE-PhysicalPreserved: Only checked catalog retention may change the prefix.
    if !unchanged && !storage_retyped {
        return fail(
            target,
            "BE-PhysicalPreserved",
            "Structures change is not checked catalog retention; Sequence rewrites must preserve Physical",
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

    struct Fixture {
        physical: physical::Program,
        structures: cq_lang_structures::Program,
        program: Program,
    }

    impl syn::parse::Parse for Fixture {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let physical = input.parse()?;
            let structures = cq_lang_structures::Program::parse(input, &physical)?;
            let program = input.parse()?;
            Ok(Self { physical, structures, program })
        }
    }

    #[test]
    fn native_lookup_capabilities_require_printable_recorded_reads() {
        use cq_syntax::host::HostRead;
        let provider: Ident = syn::parse_quote!(r);
        for read in [
            HostRead::Membership,
            HostRead::Exact,
            HostRead::Range,
            HostRead::Ordered,
            HostRead::Candidates,
        ] {
            for required in [
                Capability::Membership,
                Capability::Exact,
                Capability::Range,
                Capability::Ordered,
                Capability::Candidates,
            ] {
                let supported = matches!(
                    (read, &required),
                    (
                        HostRead::Membership | HostRead::Exact,
                        Capability::Membership
                    ) | (HostRead::Exact, Capability::Exact)
                );
                let result = check_native_read(&provider, read, &required);
                assert_eq!(result.is_ok(), supported, "{read:?} for {required:?}");
                if let Err(error) = result {
                    assert!(error.to_string().contains("BE-NativeBind"));
                }
            }
        }
    }

    #[test]
    fn indexed_publication_keys_preserve_lexical_fact_order() {
        let fixture: Fixture = syn::parse_quote! {
            physical {
                relation R(k:i32, value:i64);
                r = let canon rename R@0 {k -> k, value -> value};
                grouped = let group r by [k] { n: usize = count(); };
                return grouped as answer(k,n);
            }
            structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; rows = Rows { reads [exact, range, membership], build inferred }; group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed }; }
                r host scan = @reader as Reader [];
                r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows;
                grouped rows = @rows as Rows;
                grouped group = @group_map as ::std::collections::HashMap<_, (usize,)>;
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
        let proof = analyze(&fixture.physical, &fixture.structures, &fixture.program, &Checker::default()).unwrap();
        assert_eq!(proof.facts["before"].unique, Some(vec!["k".into()]));
        for name in ["after", "last"] {
            assert_eq!(proof.facts[name].unique, Some(vec!["k".into(), "n".into()]));
        }
    }

    #[test]
    fn indexed_publication_keys_do_not_promote_unstored_views() {
        let fixture: Fixture = syn::parse_quote! {
            physical {
                relation R(a:i32);
                r = pipe rename R@0 {a -> a};
                output = let canon project r keep {a};
                return output as answer(a);
            }
            structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; } r host scan = @reader as Reader []; output rows = @sort as ::cq_storage::dedup::Sort; output canon = rows; }
            sequence {
                a_view = canon rows r;
                before = rows r;
                z_store = store a_view;
                after = rows r;
                output = canon map output after;
            }
        };
        let proof = analyze(&fixture.physical, &fixture.structures, &fixture.program, &Checker::default()).unwrap();
        assert_eq!(proof.facts["before"].unique, None);
        assert_eq!(proof.facts["after"].unique, Some(vec!["a".into()]));
    }
}
