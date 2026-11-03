//! BE-Blocks execution facts reference Structures decisions, never reselect them.
use std::collections::{BTreeMap, BTreeSet};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{LitInt, LitStr, Token, braced, bracketed, parenthesized};
use syn::parse::{Parse, ParseStream};
use crate::{Program, Term, structures};

mod kw { syn::custom_keyword!(blocks); syn::custom_keyword!(scan); syn::custom_keyword!(read); syn::custom_keyword!(run); syn::custom_keyword!(route); syn::custom_keyword!(join); syn::custom_keyword!(predicate); }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    pub source: structures::encoded::SourceId,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Execution {
    pub scans: BTreeMap<String, Scan>,
    /// One mask per run. Fusion crosses only total single-use positional
    /// predicates/projections, never an opaque read, store or branching use.
    pub runs: BTreeMap<String, Vec<String>>,
    /// Physical consumers are materialization execution points. Their exact
    /// demand and ownership actions remain authoritative Structures records.
    pub reads: BTreeSet<String>,
    /// Exact index/probe execution sites, referencing Structures boundary facts.
    pub joins: BTreeSet<String>,
    /// Carrier-local exact formulas; these never enter a single-source tile run.
    pub carrier_predicates: BTreeSet<String>,
    /// PartitionK's global-code key reads, by Physical carrier and column.
    /// The constructor records keys/parts; these references authorize its reads.
    pub routing: BTreeMap<String, BTreeMap<String, structures::encoded::RepresentationId>>,
}
impl Execution {
    pub fn is_empty(&self) -> bool { self.scans.is_empty() && self.reads.is_empty() && self.runs.is_empty() && self.routing.is_empty() && self.joins.is_empty() && self.carrier_predicates.is_empty() }
    pub fn record(source: &structures::Program) -> Self {
        let blocks = &source.encoded.blocks;
        if blocks.scans.is_empty() { return Self::default(); }
        let definitions: BTreeMap<_, _> = source.physical.program.definitions.iter().map(|d| (structures::symbol_name(&d.name), d)).collect();
        let mut uses = BTreeMap::<String, usize>::new();
        for definition in &source.physical.program.definitions {
            for operand in definition.operator.operands() { *uses.entry(structures::symbol_name(operand)).or_default() += 1; }
        }
        *uses.entry(structures::symbol_name(&source.physical.program.returned)).or_default() += 1;
        let mut roots = BTreeMap::<String, String>::new();
        let mut runs = BTreeMap::<String, Vec<String>>::new();
        for definition in &source.physical.program.definitions {
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
            runs,
            reads: blocks.reads.keys().cloned().chain(blocks.canonical_keys.keys().map(|name| format!("canon:{name}"))).collect(),
            joins: blocks.joins.keys().cloned().collect(),
            carrier_predicates: blocks.carrier_predicates.keys().cloned().collect(),
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
            } else if body.peek(kw::run) {
                body.parse::<kw::run>()?;
                let root: LitStr = body.parse()?;
                let names; bracketed!(names in body);
                let names = syn::punctuated::Punctuated::<LitStr, Token![,]>::parse_terminated(&names)?.into_iter().map(|name| name.value()).collect();
                if result.runs.insert(root.value(), names).is_some() { return Err(syn::Error::new_spanned(root, "duplicate block run")); }
            } else if body.peek(kw::join) {
                body.parse::<kw::join>()?;
                let name: LitStr = body.parse()?;
                if !result.joins.insert(name.value()) { return Err(syn::Error::new_spanned(name, "duplicate blocked join site")); }
            } else if body.peek(kw::predicate) {
                body.parse::<kw::predicate>()?;
                let name: LitStr = body.parse()?;
                if !result.carrier_predicates.insert(name.value()) { return Err(syn::Error::new_spanned(name, "duplicate carrier predicate site")); }
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
        let reads = self.reads.iter().map(|name| quote!(read #name;));
        let runs = self.runs.iter().map(|(root, names)| quote!(run #root [#(#names),*];));
        let joins = self.joins.iter().map(|name| quote!(join #name;));
        let carrier_predicates = self.carrier_predicates.iter().map(|name| quote!(predicate #name;));
        let routing = self.routing.iter().map(|(result, fields)| {
            let fields = fields.iter().map(|(column, id)| { let id = id.0; quote!(#column = #id) });
            quote!(route #result [#(#fields),*];)
        });
        tokens.extend(quote!(blocks { #(#scans)* #(#runs)* #(#reads)* #(#routing)* #(#joins)* #(#carrier_predicates)* }));
    }
}

/// Formal rule BE-Blocks: independently check source/consumer coverage,
/// predicate runs and aligned task source covers.
/// Neither this checker nor correspondence invokes the execution producer.
pub fn check(program: &Program) -> syn::Result<()> {
    let execution = &program.representation.blocks;
    let blocks = &program.structures.encoded.blocks;
    let invalid = || syn::Error::new_spanned(program, "BE-Blocks: invalid source cover, predicate run or materialization point");
    let ordinary_reads: BTreeSet<_> = blocks.reads.keys().cloned().collect();
    let canonical_reads: BTreeSet<_> = blocks.canonical_keys.keys().map(|name| format!("canon:{name}")).collect();
    if !execution.scans.keys().eq(blocks.scans.keys())
        || !ordinary_reads.is_subset(&execution.reads)
        || execution.reads.iter().any(|name| !ordinary_reads.contains(name) && !canonical_reads.contains(name)) { return Err(invalid()); }
    if !execution.joins.iter().eq(blocks.joins.keys()) || !execution.carrier_predicates.iter().eq(blocks.carrier_predicates.keys()) { return Err(invalid()); }
    for (name, scan) in &execution.scans {
        let source = &blocks.scans[name];
        if scan.source != source.source { return Err(invalid()); }
    }
    let physical = &program.structures.physical.program;
    let definitions: BTreeMap<_, _> = physical.definitions.iter().map(|d| (structures::symbol_name(&d.name), d)).collect();
    let mut uses = BTreeMap::<String, usize>::new();
    for definition in &physical.definitions {
        for operand in definition.operator.operands() { *uses.entry(structures::symbol_name(operand)).or_default() += 1; }
    }
    *uses.entry(structures::symbol_name(&physical.returned)).or_default() += 1;
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
    fn visit(term: &Term, execution: &Execution, blocks: &structures::blocks::Contract, scans: &mut BTreeSet<String>, predicates: &mut BTreeSet<String>, consumers: &mut BTreeSet<String>) {
        match term {
            Term::Rows(name) => { let name = structures::symbol_name(name); if execution.scans.contains_key(&name) { scans.insert(name); } }
            Term::Map(name, inner) | Term::Compact(name, inner) | Term::Group(name, inner) => { let name = structures::symbol_name(name); if blocks.predicates.contains_key(&name) || blocks.carrier_predicates.contains_key(&name) { predicates.insert(name.clone()); } if execution.reads.contains(&name) { consumers.insert(name); } visit(inner, execution, blocks, scans, predicates, consumers); }
            Term::Flatten(inner) | Term::Fold(_, inner) | Term::Canon(inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } | Term::Lookup { inner, .. } => visit(inner, execution, blocks, scans, predicates, consumers),
            Term::Union(arms) => for arm in arms { visit(arm, execution, blocks, scans, predicates, consumers); },
            Term::Choose(arms) => for (_, arm) in arms { visit(arm, execution, blocks, scans, predicates, consumers); },
            Term::Ref(_) => {},
        }
    }
    for definition in &program.sequence.definitions {
        visit(&definition.term, execution, blocks, &mut scans, &mut predicates, &mut consumers);
    }
    let answer = structures::symbol_name(&physical.head.relation);
    let returned = structures::symbol_name(&physical.returned);
    for (name, read) in &blocks.reads {
        use structures::encoded::{Consumer, ReadPurpose};
        if read.input != returned { continue; }
        let at_output = match (&program.structures.consumer, &read.consumer, read.purpose) {
            (Some(demand), Consumer::Relational(consumer), ReadPurpose::PreserveCarrier)
            | (Some(demand), Consumer::OpaqueRust(consumer), ReadPurpose::OpaqueRustRead) =>
                *consumer == demand.original_consumer,
            (None, Consumer::FinalAnswer(consumer), ReadPurpose::FinalHostAnswer) =>
                *consumer == answer,
            _ => false,
        };
        if at_output { consumers.insert(name.clone()); }
    }
    let all_predicates: BTreeSet<_> = blocks.predicates.keys().chain(blocks.carrier_predicates.keys()).cloned().collect();
    if !scans.iter().eq(execution.scans.keys()) || predicates != all_predicates || consumers != ordinary_reads { return Err(invalid()); }
    Ok(())
}

    fn result(term: &Term, program: &Program, analysis: &crate::Analysis) -> Option<String> {
        match term {
            Term::Rows(name) => Some(structures::symbol_name(name)),
            Term::Ref(name) => analysis.results.get(&structures::symbol_name(name)).cloned(),
            Term::Map(name, _) | Term::Compact(name, _) | Term::Group(name, _) => Some(structures::symbol_name(name)),
            Term::Store { inner, .. } | Term::Canon(inner) | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } => result(inner, program, analysis),
            Term::Flatten(inner) => {
                let Term::Lookup { provider, keys, inner } = inner.as_ref() else { return result(inner, program, analysis); };
                let left = result(inner, program, analysis)?;
                let right = structures::symbol_name(provider);
                let mut columns: Vec<_> = keys.iter().map(structures::symbol_name).collect();
                columns.sort();
                program.structures.physical.program.definitions.iter().find_map(|definition| {
                    let structures::physical::Operator::Probe(_, keyed) = &definition.operator else { return None; };
                    let mut expected: Vec<_> = keyed.columns.iter().map(structures::symbol_name).collect();
                    expected.sort();
                    (structures::symbol_name(&keyed.left) == left && structures::symbol_name(&keyed.right) == right && expected == columns).then(|| structures::symbol_name(&definition.name))
                })
            }
            Term::Fold(_, _) | Term::Lookup { .. } | Term::Union(_) | Term::Choose(_) => None,
        }
    }
/// Actual Canon constructors are the sole owner of canonical execution.
/// Structures supplies key recipes; this extracts references to recipes used
/// by the supplied term spine without planning or rewriting that spine.
pub fn canonical_read_sites(program: &Program, analysis: &crate::Analysis) -> BTreeSet<String> {
    fn visit(term: &Term, program: &Program, analysis: &crate::Analysis, sites: &mut BTreeSet<String>) {
        if let Term::Canon(inner) = term {
            if let Some(input) = result(inner, program, analysis) {
                if program.structures.encoded.blocks.canonical_keys.contains_key(&input) {
                    sites.insert(format!("canon:{input}"));
                }
            }
        }
        match term {
            Term::Map(_, inner) | Term::Compact(_, inner) | Term::Group(_, inner) | Term::Flatten(inner) | Term::Fold(_, inner) | Term::Canon(inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } | Term::Lookup { inner, .. } => visit(inner, program, analysis, sites),
            Term::Union(arms) => for arm in arms { visit(arm, program, analysis, sites); },
            Term::Choose(arms) => for (_, arm) in arms { visit(arm, program, analysis, sites); },
            Term::Rows(_) | Term::Ref(_) => {}
        }
    }
    let mut sites = BTreeSet::new();
    for definition in &program.sequence.definitions { visit(&definition.term, program, analysis, &mut sites); }
    sites
}

/// Exact join sites are checked against the supplied Sequence lookup spine,
/// not against a regenerated lowering. Existing BE-Lookup checks own provider
/// availability and build-before-read; these records authorize carrier access.
fn check_join_sites(program: &Program, analysis: &crate::Analysis) -> syn::Result<()> {
    fn visit(term: &Term, program: &Program, analysis: &crate::Analysis, covered: &mut BTreeSet<String>) {
        if let Term::Lookup { provider, keys, inner } = term {
            if let Some(left) = result(inner, program, analysis) {
                let right = structures::symbol_name(provider);
                let mut columns: Vec<_> = keys.iter().map(structures::symbol_name).collect();
                columns.sort();
                for (name, join) in &program.structures.encoded.blocks.joins {
                    if join.left == left && join.right == right && join.keys.iter().map(|key| &key.column).eq(columns.iter()) {
                        covered.insert(name.clone());
                    }
                }
            }
        }
        match term {
            Term::Map(_, inner) | Term::Compact(_, inner) | Term::Group(_, inner) | Term::Flatten(inner) | Term::Fold(_, inner) | Term::Canon(inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } | Term::Lookup { inner, .. } => visit(inner, program, analysis, covered),
            Term::Union(arms) => for arm in arms { visit(arm, program, analysis, covered); },
            Term::Choose(arms) => for (_, arm) in arms { visit(arm, program, analysis, covered); },
            Term::Rows(_) | Term::Ref(_) => {}
        }
    }
    let mut covered = BTreeSet::new();
    for definition in &program.sequence.definitions { visit(&definition.term, program, analysis, &mut covered); }
    if covered != program.representation.blocks.joins {
        return Err(syn::Error::new_spanned(program, "BE-Blocks: missing exact blocked join execution site"));
    }
    Ok(())
}

/// A task partitions the borrowed source itself. Its contiguous row ranges
/// are disjoint, aligned, and clipped only at the source's live final extent.
pub fn check_task_covers(program: &Program, analysis: &crate::Analysis) -> syn::Result<()> {
    if program.representation.blocks.is_empty() { return Ok(()); }
    check_routing(program, &analysis.structures)?;
    check_join_sites(program, analysis)?;
    let expected_reads: BTreeSet<_> = program.structures.encoded.blocks.reads.keys().cloned()
        .chain(canonical_read_sites(program, analysis)).collect();
    if expected_reads != program.representation.blocks.reads {
        return Err(syn::Error::new_spanned(program, "BE-Blocks: canonical read sites do not match actual Canon constructors"));
    }
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
    fn visit(term: &Term, program: &Program, terms: &BTreeMap<*const Term, &crate::task_execution::Node>) -> syn::Result<()> {
        if let Term::Tile { n, inner, .. } = term {
            let producer = terms[&(term as *const Term)];
            if matches!(producer.staging, Some(crate::task_execution::Staging::TileInput)) && unstored_source(inner, program) {
                return Err(syn::Error::new_spanned(program, "BE-Blocks: tasks must borrow source ranges rather than collect an unstored blocked scan"));
            }
            if let Some(source) = producer.borrowed_host.as_ref() {
                if let Some(scan) = program.representation.blocks.scans.get(source) {
                    let vector_rows = program.structures.encoded.blocks.sources[&scan.source].vector_rows;
                    if vector_rows == 0 || *n == 0 || *n % vector_rows != 0 { return Err(syn::Error::new_spanned(program, "BE-Blocks: task range must align to source vectors")); }
                }
            }
        }
        match term {
            Term::Map(_, inner) | Term::Compact(_, inner) | Term::Group(_, inner) | Term::Flatten(inner) | Term::Fold(_, inner) | Term::Canon(inner) | Term::Store { inner, .. } | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } | Term::Lookup { inner, .. } => visit(inner, program, terms)?,
            Term::Union(arms) => for arm in arms { visit(arm, program, terms)?; },
            Term::Choose(arms) => for (_, arm) in arms { visit(arm, program, terms)?; },
            Term::Rows(_) | Term::Ref(_) => {},
        }
        Ok(())
    }
    for definition in &program.sequence.definitions { visit(&definition.term, program, &terms)?; }
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
pub fn record_routing(program: &mut Program) -> syn::Result<()> {
    if program.representation.blocks.scans.is_empty() { return Ok(()); }
    let facts = structures::contract::check(&program.structures)?;
    let origins = structures::representation_bases(&program.structures, &facts);
    let mut routing = BTreeMap::<String, BTreeMap<String, structures::encoded::RepresentationId>>::new();
    for (result, keys) in routing_sites(program) {
        for column in keys {
            let representation = origins.get(&result).into_iter().flatten().find_map(|origin| {
                let scan = program.structures.encoded.blocks.scans.get(&origin.occurrence)?;
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

fn check_routing(program: &Program, facts: &structures::Analysis) -> syn::Result<()> {
    let origins = structures::representation_bases(&program.structures, facts);
    let encoded = &program.structures.encoded;
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
