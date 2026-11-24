//! BE-Blocks execution facts reference Structures decisions, never reselect them.
use std::collections::{BTreeMap, BTreeSet};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Ident, LitInt, LitStr, Token, braced, bracketed, parenthesized};
use syn::parse::{Parse, ParseStream};
use crate::{Program, Term, structures};

mod kw { syn::custom_keyword!(blocks); syn::custom_keyword!(scan); syn::custom_keyword!(predicate); syn::custom_keyword!(read); syn::custom_keyword!(run); syn::custom_keyword!(route); }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    pub source: structures::encoded::SourceId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Prune, Exact, Positions }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Execution {
    pub scans: BTreeMap<String, Scan>,
    pub predicates: BTreeMap<String, Vec<Phase>>,
    /// One mask per run. Fusion crosses only total single-use positional
    /// predicates/projections, never an opaque read, store or branching use.
    pub runs: BTreeMap<String, Vec<String>>,
    /// Physical consumers are materialization execution points. Their exact
    /// demand and ownership actions remain authoritative Structures records.
    pub reads: BTreeSet<String>,
    /// PartitionK's global-code key reads, by Physical carrier and column.
    /// The constructor records keys/parts; these references authorize its reads.
    pub routing: BTreeMap<String, BTreeMap<String, structures::encoded::RepresentationId>>,
}
impl Execution {
    pub fn is_empty(&self) -> bool { self.scans.is_empty() && self.predicates.is_empty() && self.reads.is_empty() && self.runs.is_empty() && self.routing.is_empty() }
    pub fn record(physical: &structures::physical::Program, source: &structures::Program) -> Self {
        let blocks = &source.encoded.blocks;
        if blocks.scans.is_empty() { return Self::default(); }
        let definitions: BTreeMap<_, _> = physical.program.definitions.iter().map(|d| (structures::symbol_name(&d.name), d)).collect();
        let mut uses = BTreeMap::<String, usize>::new();
        for definition in &physical.program.definitions {
            for operand in definition.operator.operands() { *uses.entry(structures::symbol_name(operand)).or_default() += 1; }
        }
        *uses.entry(structures::symbol_name(&physical.program.returned)).or_default() += 1;
        let mut roots = BTreeMap::<String, String>::new();
        let mut runs = BTreeMap::<String, Vec<String>>::new();
        for definition in &physical.program.definitions {
            let name = structures::symbol_name(&definition.name);
            if blocks.scans.contains_key(&name) { roots.insert(name.clone(), name); continue; }
            let inherited = definition.operator.input().and_then(|input| {
                let input = structures::symbol_name(input);
                let previous = definitions[&input];
                (uses.get(&input) == Some(&1) && previous.canon.is_none() && structures::physical::ModeKind::from(&previous.mode) == structures::physical::ModeKind::Pipe).then(|| roots.get(&input).cloned()).flatten()
            });
            if blocks.predicates.contains_key(&name) {
                let root = inherited.unwrap_or_else(|| name.clone());
                runs.entry(root.clone()).or_default().push(name.clone());
                roots.insert(name, root);
            } else if matches!(definition.operator, structures::physical::Operator::Project(_)) {
                if let Some(root) = inherited { roots.insert(name, root); }
            }
        }
        Self {
            scans: blocks.scans.iter().map(|(name, scan)| (name.clone(), Scan { source: scan.source })).collect(),
            predicates: blocks.predicates.keys().map(|name| (name.clone(), vec![Phase::Prune, Phase::Exact, Phase::Positions])).collect(),
            runs,
            reads: blocks.reads.keys().cloned().collect(),
            routing: BTreeMap::new(),
        }
    }
}
impl Parse for Execution {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::blocks>()?;
        let body; braced!(body in input);
        let mut result = Self::default();
        while !body.is_empty() {
            if body.peek(kw::scan) {
                body.parse::<kw::scan>()?;
                let name: LitStr = body.parse()?;
                let args; parenthesized!(args in body);
                let source: LitInt = args.parse()?;
                if !args.is_empty() { return Err(args.error("unexpected scan execution field")); }
                if result.scans.insert(name.value(), Scan { source: structures::encoded::SourceId(source.base10_parse()?) }).is_some() { return Err(syn::Error::new_spanned(name, "duplicate block scan")); }
            } else if body.peek(kw::predicate) {
                body.parse::<kw::predicate>()?;
                let name: LitStr = body.parse()?;
                let phases; bracketed!(phases in body);
                let mut values = Vec::new();
                while !phases.is_empty() {
                    let phase: Ident = phases.parse()?;
                    values.push(match phase.to_string().as_str() { "prune" => Phase::Prune, "exact" => Phase::Exact, "positions" => Phase::Positions, _ => return Err(syn::Error::new_spanned(phase, "unknown block phase")) });
                    if !phases.is_empty() { phases.parse::<Token![,]>()?; }
                }
                if result.predicates.insert(name.value(), values).is_some() { return Err(syn::Error::new_spanned(name, "duplicate block predicate")); }
            } else if body.peek(kw::run) {
                body.parse::<kw::run>()?;
                let root: LitStr = body.parse()?;
                let names; bracketed!(names in body);
                let names = syn::punctuated::Punctuated::<LitStr, Token![,]>::parse_terminated(&names)?.into_iter().map(|name| name.value()).collect();
                if result.runs.insert(root.value(), names).is_some() { return Err(syn::Error::new_spanned(root, "duplicate block run")); }
            } else if body.peek(kw::route) {
                body.parse::<kw::route>()?;
                let result_name: LitStr = body.parse()?;
                let fields; bracketed!(fields in body);
                let mut columns = BTreeMap::new();
                while !fields.is_empty() {
                    let name: LitStr = fields.parse()?;
                    fields.parse::<Token![=]>()?;
                    let id: LitInt = fields.parse()?;
                    if columns.insert(name.value(), structures::encoded::RepresentationId(id.base10_parse()?)).is_some() {
                        return Err(syn::Error::new_spanned(name, "duplicate blocked routing field"));
                    }
                    if !fields.is_empty() { fields.parse::<Token![,]>()?; }
                }
                if result.routing.insert(result_name.value(), columns).is_some() {
                    return Err(syn::Error::new_spanned(result_name, "duplicate blocked routing carrier"));
                }
            } else {
                body.parse::<kw::read>()?; let name: LitStr = body.parse()?;
                if !result.reads.insert(name.value()) { return Err(syn::Error::new_spanned(name, "duplicate block read")); }
            }
            body.parse::<Token![;]>()?;
        }
        Ok(result)
    }
}
impl ToTokens for Execution {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        if self.is_empty() { return; }
        let scans = self.scans.iter().map(|(name, scan)| { let source = scan.source.0; quote!(scan #name (#source);) });
        let predicates = self.predicates.iter().map(|(name, phases)| { let phases = phases.iter().map(|phase| match phase { Phase::Prune => quote!(prune), Phase::Exact => quote!(exact), Phase::Positions => quote!(positions) }); quote!(predicate #name [#(#phases),*];) });
        let reads = self.reads.iter().map(|name| quote!(read #name;));
        let runs = self.runs.iter().map(|(root, names)| quote!(run #root [#(#names),*];));
        let routing = self.routing.iter().map(|(result, fields)| {
            let fields = fields.iter().map(|(column, id)| { let id = id.0; quote!(#column = #id) });
            quote!(route #result [#(#fields),*];)
        });
        tokens.extend(quote!(blocks { #(#scans)* #(#predicates)* #(#runs)* #(#reads)* #(#routing)* }));
    }
}

/// Formal rule BE-Blocks: independently check exact phase/consumer coverage,
/// native occurrence positions, bounded scratch and aligned task source covers.
/// Neither this checker nor correspondence invokes the execution producer.
pub fn check(physical: &structures::physical::Program, structures: &structures::Program, program: &Program) -> syn::Result<()> {
    let execution = &program.representation.blocks;
    let blocks = &structures.encoded.blocks;
    let invalid = || syn::Error::new_spanned(program, "BE-Blocks: invalid source cover, phase order or materialization point");
    if !execution.scans.keys().eq(blocks.scans.keys()) || !execution.predicates.keys().eq(blocks.predicates.keys()) || !execution.reads.iter().eq(blocks.reads.keys()) { return Err(invalid()); }
    for (name, scan) in &execution.scans {
        let source = &blocks.scans[name];
        if scan.source != source.source { return Err(invalid()); }
    }
    if execution.predicates.values().any(|phases| phases != &[Phase::Prune, Phase::Exact, Phase::Positions]) { return Err(invalid()); }
    let definitions: BTreeMap<_, _> = physical.program.definitions.iter().map(|d| (structures::symbol_name(&d.name), d)).collect();
    let mut uses = BTreeMap::<String, usize>::new();
    for definition in &physical.program.definitions {
        for operand in definition.operator.operands() { *uses.entry(structures::symbol_name(operand)).or_default() += 1; }
    }
    *uses.entry(structures::symbol_name(&physical.program.returned)).or_default() += 1;
    let mut covered = BTreeSet::new();
    for (root, names) in &execution.runs {
        if names.is_empty() || (!execution.scans.contains_key(root) && names.first() != Some(root)) { return Err(invalid()); }
        let mut previous = root.clone();
        for (index, name) in names.iter().enumerate() {
            let Some(predicate) = blocks.predicates.get(name) else { return Err(invalid()); };
            if !covered.insert(name.clone()) { return Err(invalid()); }
            if index == 0 && name == root { continue; }
            let mut current = predicate.input.clone();
            loop {
                let Some(definition) = definitions.get(&current) else { return Err(invalid()); };
                if uses.get(&current) != Some(&1) || definition.canon.is_some() || structures::physical::ModeKind::from(&definition.mode) != structures::physical::ModeKind::Pipe { return Err(invalid()); }
                if current == previous { break; }
                let structures::physical::Operator::Project(project) = &definition.operator else { return Err(invalid()); };
                current = structures::symbol_name(&project.input);
            }
            previous = name.clone();
        }
    }
    if !covered.iter().eq(blocks.predicates.keys()) { return Err(invalid()); }
    let mut scans = BTreeSet::new();
    let mut predicates = BTreeSet::new();
    let mut consumers = BTreeSet::new();
    fn visit(term: &Term, execution: &Execution, scans: &mut BTreeSet<String>, predicates: &mut BTreeSet<String>, consumers: &mut BTreeSet<String>) {
        match term {
            Term::Rows(name) => { let name = structures::symbol_name(name); if execution.scans.contains_key(&name) { scans.insert(name); } }
            Term::Map(name, inner) | Term::Compact(name, inner) | Term::Group(name, inner) => { let name = structures::symbol_name(name); if execution.predicates.contains_key(&name) { predicates.insert(name.clone()); } if execution.reads.contains(&name) { consumers.insert(name); } visit(inner, execution, scans, predicates, consumers); }
            Term::Flatten(inner) | Term::Fold(_, inner) | Term::Canon(inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } | Term::Lookup { inner, .. } => visit(inner, execution, scans, predicates, consumers),
            Term::Union(arms) => for arm in arms { visit(arm, execution, scans, predicates, consumers); },
            Term::Choose(arms) => for (_, arm) in arms { visit(arm, execution, scans, predicates, consumers); },
            Term::Ref(_) => {},
        }
    }
    fn has_canon(term: &Term) -> bool {
        match term {
            Term::Canon(_) => true,
            Term::Map(_, inner) | Term::Compact(_, inner) | Term::Group(_, inner) | Term::Flatten(inner) | Term::Fold(_, inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } | Term::Lookup { inner, .. } => has_canon(inner),
            Term::Union(arms) => arms.iter().any(has_canon),
            Term::Choose(arms) => arms.iter().any(|(_, arm)| has_canon(arm)),
            Term::Rows(_) | Term::Ref(_) => false,
        }
    }
    for definition in &program.sequence.definitions {
        visit(&definition.term, execution, &mut scans, &mut predicates, &mut consumers);
        let canonical = format!("canon:{}", structures::symbol_name(&definition.name));
        if execution.reads.contains(&canonical) && has_canon(&definition.term) { consumers.insert(canonical); }
    }
    let answer = structures::symbol_name(&physical.program.head.relation);
    let returned = structures::symbol_name(&physical.program.returned);
    for (name, read) in &blocks.reads {
        use structures::encoded::{Consumer, ReadPurpose};
        if read.input != returned { continue; }
        let at_output = match (&structures.consumer, &read.consumer, read.purpose) {
            (Some(demand), Consumer::Relational(consumer), ReadPurpose::PreserveCarrier)
            | (Some(demand), Consumer::OpaqueRust(consumer), ReadPurpose::OpaqueRustRead) =>
                *consumer == demand.original_consumer,
            (None, Consumer::FinalAnswer(consumer), ReadPurpose::FinalHostAnswer) =>
                *consumer == answer,
            _ => false,
        };
        if at_output { consumers.insert(name.clone()); }
    }
    let returned_canon = format!("canon:{}", structures::symbol_name(&physical.program.returned));
    if physical.program.return_canon.is_some() && execution.reads.contains(&returned_canon)
        && program.sequence.definitions.last().is_some_and(|definition| has_canon(&definition.term))
    {
        consumers.insert(returned_canon);
    }
    if !scans.iter().eq(execution.scans.keys()) || !predicates.iter().eq(execution.predicates.keys()) || consumers != execution.reads { return Err(invalid()); }
    Ok(())
}

/// A task partitions the borrowed source itself. Its contiguous row ranges
/// are disjoint, aligned, and clipped only at the source's live final extent.
pub fn check_task_covers(physical: &structures::physical::Program, structures: &structures::Program, program: &Program, structure_analysis: &structures::Analysis, analysis: &crate::Analysis) -> syn::Result<()> {
    if program.representation.blocks.is_empty() { return Ok(()); }
    check_routing(physical, structures, program, structure_analysis)?;
    let terms = analysis.task_execution.terms(program);
    fn unstored_source(term: &Term, program: &Program) -> bool {
        match term {
            Term::Rows(name) => program.representation.blocks.scans.contains_key(&structures::symbol_name(name)),
            Term::Ref(name) => program.sequence.definitions.iter().find(|definition| definition.name == *name).is_some_and(|definition| unstored_source(&definition.term, program)),
            Term::Store { .. } | Term::Canon(_) | Term::Group(_, _) | Term::Fold(_, _) => false,
            Term::Map(_, inner) | Term::Compact(_, inner) | Term::Flatten(inner) | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } | Term::Lookup { inner, .. } => unstored_source(inner, program),
            Term::Union(arms) => arms.iter().any(|arm| unstored_source(arm, program)),
            Term::Choose(arms) => arms.iter().any(|(_, arm)| unstored_source(arm, program)),
        }
    }
    fn visit(term: &Term, structures: &structures::Program, program: &Program, terms: &BTreeMap<*const Term, &crate::task_execution::Node>) -> syn::Result<()> {
        if let Term::Tile { n, inner, .. } = term {
            let producer = terms[&(term as *const Term)];
            if matches!(producer.staging, Some(crate::task_execution::Staging::TileInput)) && unstored_source(inner, program) {
                return Err(syn::Error::new_spanned(program, "BE-Blocks: tasks must borrow source ranges rather than collect an unstored blocked scan"));
            }
            if let Some(source) = producer.borrowed_host.as_ref() {
                if let Some(scan) = program.representation.blocks.scans.get(source) {
                    let vector_rows = structures.encoded.blocks.sources[&scan.source].vector_rows;
                    if *n == 0 || *n % vector_rows != 0 { return Err(syn::Error::new_spanned(program, "BE-Blocks: task range must align to source vectors")); }
                }
            }
        }
        match term {
            Term::Map(_, inner) | Term::Compact(_, inner) | Term::Group(_, inner) | Term::Flatten(inner) | Term::Fold(_, inner) | Term::Canon(inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } | Term::Lookup { inner, .. } => visit(inner, structures, program, terms)?,
            Term::Union(arms) => for arm in arms { visit(arm, structures, program, terms)?; },
            Term::Choose(arms) => for (_, arm) in arms { visit(arm, structures, program, terms)?; },
            Term::Rows(_) | Term::Ref(_) => {},
        }
        Ok(())
    }
    for definition in &program.sequence.definitions { visit(&definition.term, structures, program, &terms)?; }
    Ok(())
}

/// Original PartitionK key demands and their Physical carrier provenance.
/// This traversal interprets constructors, not emitted tokens or Rust types.
fn routing_sites(program: &Program) -> Vec<(String, Vec<String>)> {
    fn result(program: &Program, term: &Term, remaining: usize) -> Option<String> {
        match term {
            Term::Rows(name) | Term::Map(name, _) | Term::Compact(name, _) | Term::Group(name, _) => Some(structures::symbol_name(name)),
            Term::Ref(name) if remaining > 0 => program.sequence.definitions.iter()
                .find(|definition| definition.name == *name)
                .and_then(|definition| result(program, &definition.term, remaining - 1)),
            Term::Flatten(inner) | Term::Canon(inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } => result(program, inner, remaining),
            Term::Choose(arms) => {
                let first = result(program, &arms.first()?.1, remaining)?;
                arms.iter().all(|(_, arm)| result(program, arm, remaining).as_ref() == Some(&first)).then_some(first)
            }
            _ => None,
        }
    }
    fn visit(program: &Program, term: &Term, sites: &mut Vec<(String, Vec<String>)>) {
        if let Term::PartitionK { keys, inner, .. } = term
            && let Some(carrier) = result(program, inner, program.sequence.definitions.len())
        {
            sites.push((carrier, keys.iter().map(structures::symbol_name).collect()));
        }
        match term {
            Term::Map(_, inner) | Term::Compact(_, inner) | Term::Group(_, inner) | Term::Flatten(inner) | Term::Fold(_, inner) | Term::Canon(inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } | Term::Lookup { inner, .. } => visit(program, inner, sites),
            Term::Union(arms) => for arm in arms { visit(program, arm, sites); },
            Term::Choose(arms) => for (_, arm) in arms { visit(program, arm, sites); },
            Term::Rows(_) | Term::Ref(_) => {},
        }
    }
    let mut sites = Vec::new();
    for definition in &program.sequence.definitions { visit(program, &definition.term, &mut sites); }
    sites
}

/// Producer-side refinement after a Sequence rewrite introduces or moves a
/// routing constructor. Existing source/predicate/materialization facts remain
/// unchanged; the independent checker below never calls this producer.
pub fn record_routing(physical: &structures::physical::Program, structures: &structures::Program, program: &mut Program) -> syn::Result<()> {
    if program.representation.blocks.scans.is_empty() { return Ok(()); }
    let facts = structures::contract::check(physical, structures)?;
    let origins = structures::representation_bases(physical, structures, &facts);
    let mut routing = BTreeMap::<String, BTreeMap<String, structures::encoded::RepresentationId>>::new();
    for (result, keys) in routing_sites(program) {
        for column in keys {
            let representation = origins.get(&result).into_iter().flatten().find_map(|origin| {
                let scan = structures.encoded.blocks.scans.get(&origin.occurrence)?;
                scan.columns.get(origin.columns.get(&column)?).copied()
            });
            if let Some(representation) = representation {
                routing.entry(result.clone()).or_default().insert(column, representation);
            }
        }
    }
    program.representation.blocks.routing = routing;
    Ok(())
}

fn check_routing(physical: &structures::physical::Program, structures: &structures::Program, program: &Program, facts: &structures::Analysis) -> syn::Result<()> {
    let origins = structures::representation_bases(physical, structures, facts);
    let encoded = &structures.encoded;
    let mut used = BTreeSet::new();
    let invalid = || syn::Error::new_spanned(program, "BE-Blocks: routing key lacks its exact retained ordinal representation");
    for (result, keys) in routing_sites(program) {
        for column in keys {
            for origin in origins.get(&result).into_iter().flatten() {
                let Some(scan) = encoded.blocks.scans.get(&origin.occurrence) else { continue; };
                let Some(base) = origin.columns.get(&column) else { continue; };
                let recorded = program.representation.blocks.routing.get(&result)
                    .and_then(|fields| fields.get(&column)).ok_or_else(invalid)?;
                if scan.columns.get(base) != Some(recorded) { return Err(invalid()); }
                let representation = &encoded.records.representations[recorded];
                let declaration = &encoded.obligations.declarations[&representation.declaration];
                if !declaration.preserves.contains(&structures::encoded::Preservation::Equality) { return Err(invalid()); }
                used.insert((result.clone(), column.clone()));
            }
        }
    }
    for (result, fields) in &program.representation.blocks.routing {
        if fields.is_empty() { return Err(invalid()); }
        for column in fields.keys() {
            if !used.remove(&(result.clone(), column.clone())) { return Err(invalid()); }
        }
    }
    if !used.is_empty() { return Err(invalid()); }
    Ok(())
}
