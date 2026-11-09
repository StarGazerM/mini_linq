//! Authored S1 scan bindings and checked decisions. Family and domain ownership
//! remains in `encoded`; these records only reference its declaration identities.
use std::collections::{BTreeMap, BTreeSet};
use crate::{encoded::*, physical::Operator, symbol_name, Analysis, Program, Repr};
use cq_syntax::{LogicalTypeId, constraint::{Constraint, Term}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summary { None, MinMax }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prune { None, MinMax }
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
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contract {
    /// Original explicit provider/binder obligations, not a second family table.
    pub sources: BTreeMap<SourceId, Source>,
    pub scans: BTreeMap<String, Scan>,
    pub predicates: BTreeMap<String, Predicate>,
    pub reads: BTreeMap<String, Read>,
}
fn require(ok: bool, message: &str) -> syn::Result<()> {
    if ok { Ok(()) } else { Err(syn::Error::new(proc_macro2::Span::call_site(), format!("blocked contract: {message}"))) }
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

/// Formal rule STR-Blocks: independently discharge original source/schema obligations.
/// Formal rule STR-BlockPredicate: retain the exact Physical formula and demand.
pub(crate) fn check(physical: &crate::physical::Program, program: &Program, facts: &Analysis) -> syn::Result<BTreeSet<RepresentationId>> {
    let contract = &program.encoded.blocks;
    let encoded = &program.encoded;
    let mut used = BTreeSet::new();
    if contract == &Contract::default() { return Ok(used); }
    let origins = crate::representation_bases(physical, program, facts);
    let mut source_coverage = BTreeSet::new();
    for (occurrence, scan) in &contract.scans {
        let binding = contract.sources.get(&scan.source).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing blocked source binding"))?;
        let source = encoded.obligations.sources.get(&scan.source).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing encoded source"))?;
        require(source.occurrence == *occurrence, "source occurrence changed")?;
        require(binding.block_rows > 0 && binding.block_rows <= u32::MAX as u64 && binding.block_rows % 1024 == 0, "invalid block geometry")?;
        let relation = physical.inputs().find(|r| symbol_name(&r.name) == source.relation).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "unknown blocked relation"))?;
        let definition = physical.program.definitions.iter().find(|d| symbol_name(&d.name) == *occurrence).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "unknown blocked occurrence"))?;
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
            require(binding.summary != Summary::MinMax || declaration.buffers.contains(&BufferKind::BlockSummaries), "summary enclosure capability missing")?;
            require(declaration.preserves == BTreeSet::from([Preservation::Equality, Preservation::Order]), "S1 ordinal preserved-operation law missing")?;
            used.insert(id);
        }
        source_coverage.insert(scan.source);
    }
    require(source_coverage.iter().eq(contract.sources.keys()), "source binding coverage")?;
    let mut expected_predicates = BTreeSet::new();
    let mut expected_reads = BTreeSet::new();
    let mut read_ids = BTreeSet::new();
    let mut active = BTreeSet::new();
    for definition in &physical.program.definitions {
        let name = symbol_name(&definition.name);
        let is_scan = contract.scans.contains_key(&name);
        let input = definition.operator.input().map(symbol_name);
        let inherited = definition.operator.operands().iter().any(|i| active.contains(&symbol_name(i)));
        if !is_scan && !inherited { continue; }
        let carry = matches!(definition.operator, Operator::Rename(_) | Operator::Select(_) | Operator::Point(..) | Operator::Range(..) | Operator::Project(_) | Operator::Bind(_) | Operator::Guard(_));
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
            require(facts.physical.modes[&name] == crate::physical::ModeKind::Pipe || (facts.physical.modes[&name] == crate::physical::ModeKind::Let && definition.canon.is_some()), "blocked spine cannot cross noncanonical retained storage")?;
            require(matches!(facts.repr.get(&name), Some(Repr::Positions { .. })), "blocked positions must remain positions")?;
            active.insert(name.clone());
        }
        if let Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) = &definition.operator {
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
            require(p.prune != Prune::MinMax || (contract.sources[&p.source].summary == Summary::MinMax && columns.len() == 1 && p.strategy != Strategy::Formula), "illegal summary strategy")?;
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
        if inherited {
            if let Some(fields) = facts.physical.rust_reads(&definition.operator) {
                let input = input.as_ref().unwrap();
                check_read(physical, program, facts, &origins, &name, input, fields, Consumer::OpaqueRust(name.clone()), ReadPurpose::OpaqueRustRead, &mut read_ids)?;
                expected_reads.insert(name.clone());
            }
        }
        if carry && definition.canon.is_some() {
            require(matches!(facts.canonical.get(&name), Some(crate::CanonSource::Rows(provider)) if symbol_name(&provider.token) == "sort"), "blocked canonical boundary requires recorded comparator rows")?;
            let key = format!("canon:{name}");
            check_read(physical, program, facts, &origins, &key, &name, facts.physical.headings[&name].iter().cloned().collect(), Consumer::Relational(name.clone()), ReadPurpose::PreserveCarrier, &mut read_ids)?;
            expected_reads.insert(key);
        }
    }
    let returned = symbol_name(&physical.program.returned);
    if active.contains(&returned) {
        if physical.program.return_canon.is_some() {
            require(facts.head_dedup.get(&returned).is_some_and(|provider| symbol_name(&provider.token) == "sort"), "blocked return canonical boundary requires recorded comparator rows")?;
            let key = format!("canon:{returned}");
            if expected_reads.insert(key.clone()) {
                check_read(physical, program, facts, &origins, &key, &returned, facts.physical.headings[&returned].iter().cloned().collect(), Consumer::Relational(returned.clone()), ReadPurpose::PreserveCarrier, &mut read_ids)?;
            }
        }
        let consumer = symbol_name(&physical.program.head.relation);
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
                check_read(physical, program, facts, &origins, &key, &returned, fields, expected_consumer, purpose, &mut read_ids)?;
                expected_reads.insert(key);
            }
        } else {
            let fields = physical.program.head.args.iter().flat_map(|e| crate::demand::columns_used_rust(e, &facts.physical.headings[&returned], None)).collect();
            check_read(physical, program, facts, &origins, &consumer, &returned, fields, Consumer::FinalAnswer(consumer.clone()), ReadPurpose::FinalHostAnswer, &mut read_ids)?;
            expected_reads.insert(consumer);
        }
    }
    require(expected_predicates.iter().eq(contract.predicates.keys()), "predicate decision coverage")?;
    require(expected_reads.iter().eq(contract.reads.keys()), "materialization decision coverage")?;
    Ok(used)
}
fn check_read(physical: &crate::physical::Program, program: &Program, facts: &Analysis, origins: &BTreeMap<String, Vec<crate::Lineage>>, name: &str, input: &str, fields: BTreeSet<String>, expected_consumer: Consumer, purpose: ReadPurpose, ids: &mut BTreeSet<u64>) -> syn::Result<()> {
    let contract = &program.encoded.blocks;
    let read = contract.reads.get(name).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "missing blocked materialization authorization"))?;
    require(read.id <= u32::MAX as u64 && ids.insert(read.id), "duplicate or invalid consumer identity")?;
    require(read.input == input && read.consumer == expected_consumer && read.purpose == purpose, "invalid materialization consumer")?;
    let demanded: BTreeSet<_> = fields.into_iter().filter(|f| !facts.physical.rows[input].computed.contains(f)).collect();
    require(read.fields.keys().cloned().collect::<BTreeSet<_>>() == demanded, "exact materialization demand changed")?;
    for (logical, field) in &read.fields {
        let original = origins[input].iter().find_map(|o| contract.scans.get(&o.occurrence).and_then(|scan| o.columns.get(logical).and_then(|base| scan.columns.get(base))));
        require(original == Some(&field.representation), "materialization lineage changed")?;
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
            require(purpose == ReadPurpose::OpaqueRustRead && physical.program.definitions.iter().any(|d| symbol_name(&d.name) == name && matches!(d.operator, Operator::Guard(_))), "borrowed temporary cannot escape read scope")?;
        }
    }
    Ok(())
}
