//! Authored S1 scan bindings and checked decisions. Family and domain ownership
//! remains in `encoded`; these records only reference its declaration identities.
use std::collections::{BTreeMap, BTreeSet};
use crate::{encoded::*, physical::Operator, symbol_name, Analysis, Program, Repr};
use cq_syntax::{LogicalTypeId, constraint::{Constraint, Term}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summary { None, BlockMinMax, VectorMinMax }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prune { None, BlockMinMax, VectorMinMax }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Test { Encoded, DecodeCodes }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy { Window, Point, Formula }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Translation { S1Ordinal }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadAction { Owned, BorrowedScoped, Ordinal }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub slot: u64,
    pub logical: LogicalTypeId,
    pub declaration: DeclarationId,
    pub widths: BTreeSet<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub block_rows: u64,
    /// Fixed-vector protocol advertised by the S1 provider and consumed by execution.
    pub vector_rows: usize,
    pub summary: Summary,
    pub columns: BTreeMap<String, Column>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    pub source: SourceId,
    pub columns: BTreeMap<String, RepresentationId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Predicate {
    pub input: String,
    pub source: SourceId,
    pub columns: BTreeMap<String, RepresentationId>,
    pub prune: Prune,
    pub test: Test,
    pub strategy: Strategy,
    pub translation: Translation,
    pub preserves: BTreeSet<Preservation>,
}
/// Forward global ordinal bridge; rustc discharges `OrderKey<Ordinal = u64>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bridge { OrderKey64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    Blocked { representation: RepresentationId, position: usize },
    Host { occurrence: String, column: String, position: usize, slot: usize, logical: LogicalTypeId, domain: Domain, bridge: Bridge },
    Owned { column: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side { Left, Right }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexKind { Host, Built }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexBinding {
    pub kind: IndexKind,
    pub provider: String,
    pub levels: Vec<Vec<String>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinKey { pub column: String, pub left: Operand, pub right: Operand }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinPosition { pub side: Side, pub occurrence: String, pub input: usize, pub output: usize }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload { pub positions: Vec<String>, pub owned: Vec<String> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub left: String,
    pub right: String,
    pub binding: IndexBinding,
    pub keys: Vec<JoinKey>,
    pub positions: Vec<JoinPosition>,
    pub owned: Vec<String>,
    pub payload: Payload,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarrierPredicate {
    pub input: String,
    pub operands: BTreeMap<String, Operand>,
    pub preserves: BTreeSet<Preservation>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRead {
    pub representation: RepresentationId,
    pub action: ReadAction,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub id: u64,
    pub input: String,
    pub consumer: Consumer,
    pub purpose: ReadPurpose,
    pub fields: BTreeMap<String, FieldRead>,
    /// Ordinary positional and already-owned operands complete the original demand.
    pub ordinary: BTreeMap<String, Operand>,
}
/// Relational value-key recipe; execution sites belong to Sequence, not Structures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalKeys {
    pub input: String,
    pub fields: BTreeMap<String, FieldRead>,
    pub ordinary: BTreeMap<String, Operand>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contract {
    /// Original explicit provider/binder obligations, not a second family table.
    pub sources: BTreeMap<SourceId, Source>,
    pub scans: BTreeMap<String, Scan>,
    pub predicates: BTreeMap<String, Predicate>,
    /// Explicit row-host positional capability premises, never inferred from types.
    pub positional_hosts: BTreeSet<String>,
    pub joins: BTreeMap<String, Join>,
    pub carrier_predicates: BTreeMap<String, CarrierPredicate>,
    pub canonical_keys: BTreeMap<String, CanonicalKeys>,
    pub reads: BTreeMap<String, Read>,
}
fn require(ok: bool, message: &str) -> syn::Result<()> {
    if ok { Ok(()) } else { Err(syn::Error::new(proc_macro2::Span::call_site(), format!("blocked contract: {message}"))) }
}
/// Resolve native carrier slots by source occurrence, including self joins.
pub fn position_slot(definitions: &[crate::physical::Definition], facts: &Analysis, input: &str, occurrence: &str) -> Option<usize> {
    position_slot_in(&crate::repr::representation_bases_of(definitions, facts), facts, input, occurrence)
}
pub fn position_occurrence(definitions: &[crate::physical::Definition], facts: &Analysis, input: &str, slot: usize) -> Option<String> {
    let origins = crate::repr::representation_bases_of(definitions, facts);
    origins.get(input)?.iter().find(|o| position_slot_in(&origins, facts, input, &o.occurrence) == Some(slot)).map(|o| o.occurrence.clone())
}
/// A position handoff must retain the declared logical fields of that occurrence.
pub fn position_covers(facts: &Analysis, input: &str, input_slot: usize, output: &str, output_slot: usize) -> bool {
    let Some(Repr::Positions { bases: inputs, .. }) = facts.repr.get(input) else { return false; };
    let Some(Repr::Positions { bases: outputs, .. }) = facts.repr.get(output) else { return false; };
    let Some(input) = inputs.iter().nth(input_slot) else { return false; };
    let Some(output) = outputs.iter().nth(output_slot) else { return false; };
    output.columns.iter().all(|column| input.columns.iter().any(|c| c == column))
}
fn position_slot_in(origins: &BTreeMap<String, Vec<crate::Lineage>>, facts: &Analysis, input: &str, occurrence: &str) -> Option<usize> {
    let Repr::Positions { bases, .. } = facts.repr.get(input)? else { return None; };
    let mut seen = BTreeSet::new();
    for (slot, base) in bases.iter().enumerate() {
        let origin = origins.get(input)?.iter().find(|o| o.base == symbol_name(&base.base)
            && !seen.contains(&o.occurrence)
            && base.columns.iter().all(|c| o.columns.get(&symbol_name(&c.logical)) == Some(&symbol_name(&c.column))))?;
        if origin.occurrence == occurrence { return Some(slot); }
        seen.insert(origin.occurrence.clone());
    }
    None
}

/// Resolve declared field identities; Rust type spelling is deliberately unused.
pub fn operand(program: &Program, facts: &Analysis, origins: &BTreeMap<String, Vec<crate::Lineage>>, input: &str, column: &str) -> syn::Result<Operand> {
    if facts.physical.rows[input].computed.contains(column) { return Ok(Operand::Owned { column: column.into() }); }
    for origin in origins.get(input).into_iter().flatten() {
        let Some(base) = origin.columns.get(column) else { continue; };
        let Some(position) = position_slot_in(origins, facts, input, &origin.occurrence) else { continue; };
        let Some(Repr::Positions { bases, .. }) = facts.repr.get(input) else { continue; };
        if !bases[position].columns.iter().any(|c| symbol_name(&c.logical) == column && symbol_name(&c.column) == *base) { continue; }
        if let Some(scan) = program.encoded.blocks.scans.get(&origin.occurrence) {
            return Ok(Operand::Blocked { representation: scan.columns[base], position });
        }
        require(program.encoded.blocks.positional_hosts.contains(&origin.base), "mixed host requires explicit positional capability")?;
        let relation = program.physical.inputs().find(|r| symbol_name(&r.name) == origin.base).unwrap();
        let (slot, declaration) = relation.columns.iter().enumerate().find(|(_, c)| symbol_name(&c.name) == *base).unwrap();
        let logical = declaration.logical_type_id.ok_or_else(|| syn::Error::new_spanned(&declaration.name, "mixed host requires declared logical identity"))?;
        let domain = Domain { code: declaration.domain.as_ref().and_then(|d| d.id), dictionary: None };
        require(declaration.domain.is_none() || domain.code.is_some(), "mixed host requires declared domain identity")?;
        return Ok(Operand::Host { occurrence: origin.occurrence.clone(), column: base.clone(), position, slot, logical, domain, bridge: Bridge::OrderKey64 });
    }
    Err(syn::Error::new(proc_macro2::Span::call_site(), "operand has no source lineage"))
}

pub fn index_binding(facts: &Analysis, build: &str, reader: &str, columns: &[String]) -> syn::Result<IndexBinding> {
    let mut key = columns.to_vec(); key.sort();
    if facts.host_read(build) {
        let provider = facts.fallbacks.get(&(build.into(), crate::ReaderRef::Result(reader.into()), key.clone()))
            .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "blocked join requires an exact host fallback"))?;
        Ok(IndexBinding { kind: IndexKind::Host, provider: symbol_name(&provider.token), levels: vec![key] })
    } else {
        let (provider, depth) = facts.built_on(build, &key).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "blocked join requires an exact built index"))?;
        let (levels, _) = facts.built[build].iter().find(|(levels, p)| p == provider && levels.iter().take(depth).flatten().cloned().collect::<BTreeSet<_>>() == key.iter().cloned().collect()).unwrap();
        require(depth == levels.len(), "blocked join requires full exact index keys")?;
        Ok(IndexBinding { kind: IndexKind::Built, provider: symbol_name(&provider.token), levels: levels.clone() })
    }
}

pub fn payload(facts: &Analysis, origins: &BTreeMap<String, Vec<crate::Lineage>>, input: &str) -> syn::Result<Payload> {
    let Some(Repr::Positions { bases, owned, .. }) = facts.repr.get(input) else {
        return Err(syn::Error::new(proc_macro2::Span::call_site(), "retained payload must be positional"));
    };
    let mut positions: Vec<_> = origins[input].iter().filter_map(|o| {
        position_slot_in(origins, facts, input, &o.occurrence).map(|slot| (slot, o.occurrence.clone()))
    }).collect();
    require(positions.len() == bases.len(), "retained payload has an unbound occurrence")?;
    positions.sort_by_key(|(slot, _)| *slot);
    Ok(Payload { positions: positions.into_iter().map(|(_, o)| o).collect(), owned: owned.columns.iter().map(symbol_name).collect() })
}

/// Retarget references after an explicit provider/retention rewrite. This does
/// not select an algorithm or alter key, predicate, read, or canonical demand.
/// Callers still discharge the complete independent rewrite contract afterward.
pub fn refresh_join_bindings(program: &mut Program) -> syn::Result<()> {
    if program.encoded.blocks.joins.is_empty() { return Ok(()); }
    let facts = crate::analyze(program)?;
    let origins = crate::representation_bases(program, &facts);
    let updates = program.encoded.blocks.joins.iter().map(|(name, join)| {
        let keys: Vec<_> = join.keys.iter().map(|key| key.column.clone()).collect();
        Ok((name.clone(), index_binding(&facts, &join.right, name, &keys)?, payload(&facts, &origins, &join.right)?))
    }).collect::<syn::Result<Vec<_>>>()?;
    for (name, binding, payload) in updates {
        let join = program.encoded.blocks.joins.get_mut(&name).unwrap();
        join.binding = binding;
        join.payload = payload;
    }
    Ok(())
}

fn identity(program: &Program, operand: &Operand) -> syn::Result<(Domain, Option<LogicalTypeId>)> {
    match operand {
        Operand::Blocked { representation, .. } => {
            let r = program.encoded.records.representations.get(representation).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "unknown operand representation"))?;
            let d = &program.encoded.obligations.declarations[&r.declaration];
            Ok((d.domain.clone(), d.domain.code.is_none().then_some(program.encoded.blocks.sources[&r.source].columns[&r.column].logical)))
        }
        Operand::Host { logical, domain, .. } => Ok((domain.clone(), domain.code.is_none().then_some(*logical))),
        Operand::Owned { .. } => Err(syn::Error::new(proc_macro2::Span::call_site(), "ordinal comparison requires declared source identity")),
    }
}

fn check_formula(program: &Program, condition: &cq_syntax::algebra::Theta, operands: &BTreeMap<String, Operand>) -> syn::Result<()> {
    fn pair(program: &Program, operands: &BTreeMap<String, Operand>, left: &Term, right: &Term) -> syn::Result<()> {
        if let (Term::Variable(left), Term::Variable(right)) = (left, right) {
            let left = &operands[&symbol_name(left)];
            let right = &operands[&symbol_name(right)];
            if !matches!((left, right), (Operand::Owned { .. }, Operand::Owned { .. })) {
                require(identity(program, left)? == identity(program, right)?, "cross-domain ordinal comparison")?;
            }
        }
        Ok(())
    }
    fn visit(program: &Program, operands: &BTreeMap<String, Operand>, c: &Constraint) -> syn::Result<()> {
        match c {
            Constraint::Compare { left, right, .. } => pair(program, operands, left, right)?,
            Constraint::Between { term, low, high } => { pair(program, operands, term, low)?; pair(program, operands, term, high)?; }
            Constraint::Or(arms) => for c in arms.iter().flatten() { visit(program, operands, c)?; },
            Constraint::In { .. } => {}
        }
        Ok(())
    }
    check_predicate_operands(condition)?;
    for c in &condition.constraints { visit(program, operands, c)?; }
    for equality in &condition.equalities {
        if let cq_syntax::EqualityValue::Variable(right) = &equality.right {
            pair(program, operands, &Term::Variable(equality.left.clone()), &Term::Variable(right.clone()))?;
        }
    }
    Ok(())
}
/// Connected positional operands of blocked consumers; all join inputs participate.
pub fn positional_results(program: &Program) -> BTreeSet<String> {
    let mut active: BTreeSet<_> = program.encoded.blocks.scans.keys().cloned().collect();
    loop {
        let before = active.len();
        for d in &program.physical.program.definitions {
            if matches!(d.operator, Operator::Group(_)) { continue; }
            let name = symbol_name(&d.name);
            let operands = d.operator.operands();
            if active.contains(&name) || operands.iter().any(|i| active.contains(&symbol_name(i))) {
                active.insert(name);
                active.extend(operands.iter().map(|i| symbol_name(i)));
            }
        }
        if active.len() == before { return active; }
    }
}

/// Unary tile fusion ends at any retained or binary boundary.
pub fn tile_source(program: &Program, input: &str) -> Option<SourceId> {
    let mut input = input.to_owned();
    loop {
        let d = program.physical.program.definitions.iter().find(|d| symbol_name(&d.name) == input)?;
        if !matches!(d.mode, crate::physical::Mode::Pipe(_)) || d.canon.is_some() { return None; }
        if let Some(scan) = program.encoded.blocks.scans.get(&input) { return Some(scan.source); }
        match &d.operator {
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => input = symbol_name(&s.input),
            Operator::Project(p) => input = symbol_name(&p.input),
            Operator::Bind(b) => input = symbol_name(&b.input),
            Operator::Guard(g) => input = symbol_name(&g.input),
            _ => return None,
        }
    }
}

/// Operations required by the original algebra, including every disjunction arm.
/// Formal rule STR-PreservedOp: require the original algebra's equality/order laws.
pub fn preserved_operations(condition: &cq_syntax::algebra::Theta) -> BTreeSet<Preservation> {
    fn visit(c: &Constraint, result: &mut BTreeSet<Preservation>) {
        match c {
            Constraint::Compare { comparison, .. } => { result.insert(if comparison.ordered() { Preservation::Order } else { Preservation::Equality }); }
            Constraint::Between { .. } => { result.insert(Preservation::Order); }
            Constraint::In { .. } => { result.insert(Preservation::Equality); }
            Constraint::Or(arms) => for c in arms.iter().flatten() { visit(c, result); },
        }
    }
    let mut result = BTreeSet::new();
    if !condition.equalities.is_empty() { result.insert(Preservation::Equality); }
    for c in &condition.constraints { visit(c, &mut result); }
    result
}

/// Every encoded comparison needs a declared column's translation domain.
/// Closed-only leaves require a separate host-predicate record, not printer inference.
pub fn check_predicate_operands(condition: &cq_syntax::algebra::Theta) -> syn::Result<()> {
    fn pair(left: &Term, right: &Term) -> bool {
        matches!(left, Term::Variable(_)) || matches!(right, Term::Variable(_))
    }
    fn supported(constraint: &Constraint) -> bool {
        match constraint {
            Constraint::Compare { left, right, .. } => pair(left, right),
            Constraint::Between { term, low, high } => pair(term, low) && pair(term, high),
            Constraint::In { term, literals } => literals.is_empty() || matches!(term, Term::Variable(_)),
            Constraint::Or(arms) => arms.iter().flatten().all(supported),
        }
    }
    require(condition.constraints.iter().all(supported), "blocked predicate requires a declared column for every comparison")
}

/// Formal rule STR-Blocks: independently discharge original source/schema obligations.
/// Formal rule STR-BlockPredicate: retain the exact Physical formula and demand.
pub(crate) fn check(program: &Program, facts: &Analysis) -> syn::Result<BTreeSet<RepresentationId>> {
    let contract = &program.encoded.blocks;
    let encoded = &program.encoded;
    let mut used = BTreeSet::new();
    if contract == &Contract::default() { return Ok(used); }
    require(contract.positional_hosts.iter().all(|host| facts.host_layouts.contains_key(host)), "unknown positional host capability")?;
    let origins = crate::representation_bases(program, facts);
    let mut source_coverage = BTreeSet::new();
    for (occurrence, scan) in &contract.scans {
        let binding = contract.sources.get(&scan.source).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing blocked source binding"))?;
        let source = encoded.obligations.sources.get(&scan.source).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing encoded source"))?;
        require(source.occurrence == *occurrence, "source occurrence changed")?;
        require(!program.physical.program.host_sets.iter().any(|fact| symbol_name(&fact.relation) == source.relation && fact.discharged.is_none()), "blocked source cannot use borrowed-row host_set certification")?;
        require(binding.vector_rows == 1024 && binding.block_rows > 0 && binding.block_rows <= u32::MAX as u64 && binding.block_rows % binding.vector_rows as u64 == 0, "invalid S1 provider geometry")?;
        let relation = program.physical.inputs().find(|r| symbol_name(&r.name) == source.relation).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "unknown blocked relation"))?;
        let definition = program.physical.program.definitions.iter().find(|d| symbol_name(&d.name) == *occurrence).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "unknown blocked occurrence"))?;
        require(!facts.physical.fused.values().any(|group| group.prefix == *occurrence || group.members.contains(occurrence)), "blocked source cannot enter a fused scope")?;
        require(matches!(&definition.operator, Operator::Rename(r) if symbol_name(&r.relation) == source.relation && r.constants.is_none()), "blocked source must be its actual unrestricted rename")?;
        let names: BTreeSet<_> = relation.columns.iter().map(|c| symbol_name(&c.name)).collect();
        require(binding.columns.keys().cloned().collect::<BTreeSet<_>>() == names && scan.columns.keys().cloned().collect::<BTreeSet<_>>() == names, "source column coverage")?;
        for (slot, column) in relation.columns.iter().enumerate() {
            let name = symbol_name(&column.name);
            let bound = &binding.columns[&name];
            let id = scan.columns[&name];
            let representation = encoded.records.representations.get(&id).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "unknown block representation"))?;
            let declaration = encoded.obligations.declarations.get(&bound.declaration).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "unknown block declaration"))?;
            require(bound.slot == slot as u64 && Some(bound.logical) == column.logical_type_id, "schema slot or logical identity changed")?;
            require(representation.declaration == bound.declaration && representation.source == scan.source && representation.column == name && representation.snapshot == source.snapshot, "representation binding changed")?;
            require(declaration.role == Role::ScanColumn && declaration.layout == Layout::S1 && matches!(declaration.family, Family::Plain | Family::Bytes), "unsupported blocked provider family")?;
            require(declaration.domain.code == column.domain.as_ref().and_then(|d| d.id) && (column.domain.is_none() || declaration.domain.code.is_some()), "source domain identity changed")?;
            require(facts.fallbacks.iter().any(|((result, reader, key), provider)| result == occurrence && matches!(reader, crate::ReaderRef::Scan) && key.is_empty() && symbol_name(&provider.token) == declaration.provider), "blocked provider is not serving source")?;
            let native = match declaration.carrier { Carrier::U8 => 1, Carrier::U16 => 2, Carrier::U32 => 4, Carrier::U64 => 8 };
            let widths = if declaration.family == Family::Plain { BTreeSet::from([native]) } else { [0, 1, 2, native].into_iter().filter(|w| *w <= native).collect() };
            require(bound.widths == widths, "invalid physical width set")?;
            require(match binding.summary {
                Summary::None => true,
                Summary::BlockMinMax => declaration.buffers.contains(&BufferKind::BlockSummaries),
                Summary::VectorMinMax => declaration.buffers.contains(&BufferKind::VectorSummaries),
            }, "selected summary enclosure capability missing")?;
            require(declaration.preserves == BTreeSet::from([Preservation::Equality, Preservation::Order]), "S1 ordinal preserved-operation law missing")?;
            used.insert(id);
        }
        source_coverage.insert(scan.source);
    }
    require(source_coverage.iter().eq(contract.sources.keys()), "source binding coverage")?;
    let mut expected_predicates = BTreeSet::new();
    let mut expected_carrier_predicates = BTreeSet::new();
    let mut expected_joins = BTreeSet::new();
    let mut expected_reads = BTreeSet::new();
    let mut expected_canonical_keys = BTreeSet::new();
    let mut read_ids = BTreeSet::new();
    let active = positional_results(program);
    for definition in &program.physical.program.definitions {
        let name = symbol_name(&definition.name);
        let is_scan = contract.scans.contains_key(&name);
        let input = definition.operator.input().map(symbol_name);
        let inherited = definition.operator.operands().iter().any(|i| active.contains(&symbol_name(i)));
        if !active.contains(&name) && !is_scan && !inherited { continue; }
        let carry = matches!(definition.operator, Operator::Rename(_) | Operator::Select(_) | Operator::Point(..) | Operator::Range(..) | Operator::Project(_) | Operator::Bind(_) | Operator::Guard(_) | Operator::Probe(..));
        require(carry || matches!(definition.operator, Operator::Group(_)), "unsupported blocked consumer binding")?;
        if let Operator::Group(group) = &definition.operator {
            let input = symbol_name(&group.input);
            let mut demand: BTreeSet<_> = group.keys.iter().map(symbol_name).collect();
            for aggregate in &group.aggregates {
                if let Some(expression) = &aggregate.expression {
                    demand.extend(crate::demand::columns_used_rust(expression, &facts.physical.headings[&input], None));
                }
            }
            require(demand.iter().all(|c| !facts.physical.headings[&input].contains(c) || facts.physical.rows[&input].computed.contains(c)), "blocked group requires owned computed operands")?;
        }
        if carry {
            require(facts.physical.modes[&name] == crate::physical::ModeKind::Pipe || (facts.physical.modes[&name] == crate::physical::ModeKind::Let && (definition.canon.is_some() || facts.retained.contains_key(&name) || facts.built.contains_key(&name))), "blocked retained spine lacks storage binding")?;
            require(matches!(facts.repr.get(&name), Some(Repr::Positions { .. })), "blocked positions must remain positions")?;
        }
        if let Operator::Probe(_, keyed) = &definition.operator {
            expected_joins.insert(name.clone());
            let join = contract.joins.get(&name).ok_or_else(|| syn::Error::new_spanned(&definition.name, "missing blocked join boundary"))?;
            let left = symbol_name(&keyed.left);
            let right = symbol_name(&keyed.right);
            require(join.left == left && join.right == right, "join input identity changed")?;
            let mut columns: Vec<_> = keyed.columns.iter().map(symbol_name).collect(); columns.sort();
            require(join.binding == index_binding(facts, &right, &name, &columns)?, "join provider binding changed")?;
            require(join.keys.iter().map(|k| &k.column).eq(columns.iter()), "join key correspondence changed")?;
            for key in &join.keys {
                require(key.left == operand(program, facts, &origins, &left, &key.column)? && key.right == operand(program, facts, &origins, &right, &key.column)?, "join key operand lineage changed")?;
                require(identity(program, &key.left)? == identity(program, &key.right)?, "join key domain mismatch")?;
            }
            require(join.payload == payload(facts, &origins, &right)?, "join retained payload layout changed")?;
            let output = payload(facts, &origins, &name)?;
            require(join.owned == output.owned && join.positions.len() == output.positions.len(), "join output carrier coverage")?;
            let mut covered = BTreeSet::new();
            for position in &join.positions {
                let input = match position.side { Side::Left => &left, Side::Right => &right };
                require(covered.insert(position.output) && output.positions.get(position.output) == Some(&position.occurrence)
                    && position_slot_in(&origins, facts, input, &position.occurrence) == Some(position.input)
                    && position_covers(facts, input, position.input, &name, position.output), "join occurrence position mapping changed")?;
            }
        }
        if let Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) = &definition.operator {
            check_predicate_operands(&s.condition)?;
            if tile_source(program, &symbol_name(&s.input)).is_none() {
                expected_carrier_predicates.insert(name.clone());
                let p = contract.carrier_predicates.get(&name).ok_or_else(|| syn::Error::new_spanned(&definition.name, "missing exact carrier predicate"))?;
                require(p.input == symbol_name(&s.input) && p.preserves == preserved_operations(&s.condition), "carrier predicate input or laws changed")?;
                require(p.operands.keys().cloned().collect::<BTreeSet<_>>() == s.condition.columns(), "carrier predicate formula coverage")?;
                for (column, recorded) in &p.operands {
                    require(*recorded == operand(program, facts, &origins, &p.input, column)?, "carrier predicate operand lineage changed")?;
                    if let Operand::Blocked { representation, .. } = recorded {
                        let d = &encoded.obligations.declarations[&encoded.records.representations[representation].declaration];
                        require(p.preserves.is_subset(&d.preserves), "carrier predicate operation not preserved")?;
                    }
                }
                check_formula(program, &s.condition, &p.operands)?;
            } else {
            expected_predicates.insert(name.clone());
            let p = contract.predicates.get(&name).ok_or_else(|| syn::Error::new_spanned(&definition.name, "missing exact blocked predicate"))?;
            require(p.input == symbol_name(&s.input), "predicate input changed")?;
            let columns = s.condition.columns();
            require(p.columns.keys().cloned().collect::<BTreeSet<_>>() == columns, "exact predicate column coverage")?;
            require(p.preserves == preserved_operations(&s.condition), "predicate operation requirements changed")?;
            let source = encoded.obligations.sources.get(&p.source).ok_or_else(|| syn::Error::new_spanned(&definition.name, "unknown predicate source"))?;
            let scan = contract.scans.get(&source.occurrence).ok_or_else(|| syn::Error::new_spanned(&definition.name, "predicate source is not blocked"))?;
            require(match p.strategy {
                Strategy::Formula => true,
                Strategy::Window => s.condition.range_bounds().is_some(),
                Strategy::Point => columns.len() == 1 && s.condition.equalities.len() + s.condition.constraints.len() == 1 && columns.iter().all(|c| crate::selection_constants(s).contains_key(c)),
            }, "predicate strategy does not execute the exact formula")?;
            require(match p.prune {
                Prune::None => true,
                Prune::BlockMinMax => contract.sources[&p.source].summary == Summary::BlockMinMax && columns.len() == 1 && p.strategy != Strategy::Formula,
                Prune::VectorMinMax => contract.sources[&p.source].summary == Summary::VectorMinMax && columns.len() == 1 && p.strategy != Strategy::Formula,
            }, "illegal summary strategy")?;
            require(p.strategy != Strategy::Formula || p.test == Test::DecodeCodes, "formula requires recorded physical decode to global codes")?;
            for (logical, id) in &p.columns {
                let base = origins[&name].iter().find(|o| o.occurrence == source.occurrence).and_then(|o| o.columns.get(logical)).ok_or_else(|| syn::Error::new_spanned(&definition.name, "predicate column has no retained lineage"))?;
                require(scan.columns.get(base) == Some(id), "predicate representation lineage changed")?;
                let d = &encoded.obligations.declarations[&encoded.records.representations[id].declaration];
                require(p.preserves.is_subset(&d.preserves), "predicate not preserved by representation")?;
            }
            fn compatible(c: &Constraint, p: &Predicate, e: &crate::encoded::Contract) -> syn::Result<()> {
                let terms: Vec<&Term> = match c {
                    Constraint::Compare { left, right, .. } => vec![left, right],
                    Constraint::Between { term, low, high } => vec![term, low, high],
                    Constraint::In { term, .. } => vec![term],
                    Constraint::Or(arms) => { for c in arms.iter().flatten() { compatible(c, p, e)?; } return Ok(()); }
                };
                let identities: Vec<_> = terms.into_iter().filter_map(|t| if let Term::Variable(v) = t { p.columns.get(&symbol_name(v)) } else { None }).map(|id| {
                    let r = &e.records.representations[id];
                    let d = &e.obligations.declarations[&r.declaration];
                    (&d.domain, d.domain.code.is_none().then_some(e.blocks.sources[&r.source].columns[&r.column].logical))
                }).collect();
                require(identities.windows(2).all(|d| d[0] == d[1]), "cross-domain ordinal comparison")
            }
            for c in &s.condition.constraints { compatible(c, p, encoded)?; }
            for equality in &s.condition.equalities {
                if let cq_syntax::EqualityValue::Variable(right) = &equality.right {
                    compatible(&Constraint::Compare { left: Term::Variable(equality.left.clone()), comparison: cq_syntax::constraint::Comparison::Eq(Default::default()), right: Term::Variable(right.clone()) }, p, encoded)?;
                }
            }
            }
        }
        if inherited {
            if let Some(fields) = facts.physical.rust_reads(&definition.operator) {
                let input = input.as_ref().unwrap();
                check_read(program, facts, &origins, &name, input, fields, Consumer::OpaqueRust(name.clone()), ReadPurpose::OpaqueRustRead, &mut read_ids)?;
                expected_reads.insert(name.clone());
            }
        }
        if carry && definition.canon.is_some() {
            require(matches!(facts.canonical.get(&name), Some(crate::CanonSource::Rows(provider)) if program.structures.providers.facts(provider).is_some_and(|facts| facts.reads(&crate::Capability::ComparatorRows))), "blocked canonical boundary requires recorded comparator rows")?;
            check_canonical_keys(program, facts, &origins, &name)?;
            expected_canonical_keys.insert(name.clone());
        }
    }
    let returned = symbol_name(&program.physical.program.returned);
    if active.contains(&returned) {
        if program.physical.program.return_canon.is_some() {
            require(facts.head_dedup.get(&returned).is_some_and(|provider| program.structures.providers.facts(provider).is_some_and(|facts| facts.reads(&crate::Capability::ComparatorRows))), "blocked return canonical boundary requires recorded comparator rows")?;
            if expected_canonical_keys.insert(returned.clone()) {
                check_canonical_keys(program, facts, &origins, &returned)?;
            }
        }
        let consumer = symbol_name(&program.physical.program.head.relation);
        if let Some(demand) = &program.consumer {
            // The original checked consumer fixes roles, independently of the
            // selected packets. Captures are host values, not row reads.
            require(demand.value_reads.iter().all(|field| {
                !facts.physical.headings[&returned].contains(field)
                    || facts.physical.rows[&returned].computed.contains(field)
            }), "blocked external consumer requires owned computed operands or captures; base-value borrowing is not supported")?;
            for (role, purpose) in [("carrier", ReadPurpose::PreserveCarrier), ("read", ReadPurpose::OpaqueRustRead)] {
                let fields = demand.head.iter().filter(|field| {
                    facts.physical.headings[&returned].contains(field)
                        && demand.value_reads.contains(field) == (purpose == ReadPurpose::OpaqueRustRead)
                }).cloned().collect();
                let key = format!("output:{consumer}:{role}");
                let expected_consumer = if purpose == ReadPurpose::PreserveCarrier {
                    Consumer::Relational(demand.original_consumer.clone())
                } else {
                    Consumer::OpaqueRust(demand.original_consumer.clone())
                };
                check_read(program, facts, &origins, &key, &returned, fields, expected_consumer, purpose, &mut read_ids)?;
                expected_reads.insert(key);
            }
        } else {
            let fields = program.physical.program.head.args.iter().flat_map(|e| crate::demand::columns_used_rust(e, &facts.physical.headings[&returned], None)).collect();
            check_read(program, facts, &origins, &consumer, &returned, fields, Consumer::FinalAnswer(consumer.clone()), ReadPurpose::FinalHostAnswer, &mut read_ids)?;
            expected_reads.insert(consumer);
        }
    }
    require(expected_predicates.iter().eq(contract.predicates.keys()), "predicate decision coverage")?;
    require(expected_carrier_predicates.iter().eq(contract.carrier_predicates.keys()), "carrier predicate decision coverage")?;
    require(expected_joins.iter().eq(contract.joins.keys()), "join boundary coverage")?;
    require(expected_reads.iter().eq(contract.reads.keys()), "materialization decision coverage")?;
    require(expected_canonical_keys.iter().eq(contract.canonical_keys.keys()), "canonical key recipe coverage")?;
    Ok(used)
}
fn check_read(program: &Program, facts: &Analysis, origins: &BTreeMap<String, Vec<crate::Lineage>>, name: &str, input: &str, fields: BTreeSet<String>, expected_consumer: Consumer, purpose: ReadPurpose, ids: &mut BTreeSet<u64>) -> syn::Result<()> {
    let contract = &program.encoded.blocks;
    let read = contract.reads.get(name).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing blocked materialization authorization"))?;
    require(read.id <= u32::MAX as u64 && ids.insert(read.id), "duplicate or invalid consumer identity")?;
    require(read.input == input && read.consumer == expected_consumer && read.purpose == purpose, "invalid materialization consumer")?;
    check_fields(program, facts, origins, name, input, fields, &read.fields, &read.ordinary, purpose)
}
fn check_canonical_keys(program: &Program, facts: &Analysis, origins: &BTreeMap<String, Vec<crate::Lineage>>, input: &str) -> syn::Result<()> {
    let keys = program.encoded.blocks.canonical_keys.get(input).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing blocked canonical key recipe"))?;
    require(keys.input == input, "canonical key input changed")?;
    check_fields(program, facts, origins, input, input, facts.physical.headings[input].iter().cloned().collect(), &keys.fields, &keys.ordinary, ReadPurpose::PreserveCarrier)
}
fn check_fields(program: &Program, facts: &Analysis, origins: &BTreeMap<String, Vec<crate::Lineage>>, name: &str, input: &str, fields: BTreeSet<String>, recorded: &BTreeMap<String, FieldRead>, recorded_ordinary: &BTreeMap<String, Operand>, purpose: ReadPurpose) -> syn::Result<()> {
    let mut demanded = BTreeSet::new();
    let mut ordinary = BTreeMap::new();
    for field in fields {
        match operand(program, facts, origins, input, &field)? {
            Operand::Blocked { .. } => { demanded.insert(field); }
            value => { ordinary.insert(field, value); }
        }
    }
    require(recorded.keys().cloned().collect::<BTreeSet<_>>() == demanded && *recorded_ordinary == ordinary, "exact materialization demand changed")?;
    for (logical, field) in recorded {
        require(matches!(operand(program, facts, origins, input, logical)?, Operand::Blocked { representation, .. } if representation == field.representation), "materialization lineage changed")?;
        require((field.action == ReadAction::Ordinal) == (purpose == ReadPurpose::PreserveCarrier), "physical code read is not semantic reconstruction")?;
        if field.action == ReadAction::Ordinal {
            let representation = &program.encoded.records.representations[&field.representation];
            let declaration = &program.encoded.obligations.declarations[&representation.declaration];
            // S1 global value codes preserve equality in both directions.
            // Source positions and block-relative residuals do not satisfy this
            // law, even when a source happens to have a primary key.
            require(declaration.layout == Layout::S1
                && matches!(declaration.family, Family::Plain | Family::Bytes)
                && declaration.preserves.contains(&Preservation::Equality),
                "carrier publication requires injective global value codes")?;
        }
        if field.action == ReadAction::BorrowedScoped {
            require(purpose == ReadPurpose::OpaqueRustRead && program.physical.program.definitions.iter().any(|d| symbol_name(&d.name) == name && matches!(d.operator, Operator::Guard(_))), "borrowed temporary cannot escape read scope")?;
        }
    }
    Ok(())
}
