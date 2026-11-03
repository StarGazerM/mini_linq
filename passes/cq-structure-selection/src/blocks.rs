//! Explicit S1 provider premises: no Rust type, expression, or spelling inference.
use super::*;
use structures::{blocks as b, encoded as e};
use syn::parse::{Parse, ParseStream};

#[derive(Default)]
pub(super) struct Advice(pub BTreeMap<String, Binding>);
pub(super) struct Binding {
    block_rows: u64,
    summary: b::Summary,
    columns: BTreeMap<String, (e::Family, e::Carrier)>,
}
fn word(input: ParseStream<'_>, expected: &str) -> syn::Result<()> {
    let word: syn::Ident = input.parse()?;
    if word == expected { Ok(()) } else { Err(syn::Error::new_spanned(word, format!("expected {expected}"))) }
}
impl Parse for Advice {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut result = Self::default();
        while !input.is_empty() {
            let relation: syn::Ident = input.parse()?;
            let body; syn::braced!(body in input);
            word(&body, "block_rows")?;
            let block_rows = body.parse::<syn::LitInt>()?.base10_parse()?;
            body.parse::<syn::Token![;]>()?;
            word(&body, "summary")?;
            let summary: syn::Ident = body.parse()?;
            let summary = match summary.to_string().as_str() { "none" => b::Summary::None, "minmax" => b::Summary::BlockMinMax, "vector_minmax" => b::Summary::VectorMinMax, _ => return Err(syn::Error::new_spanned(summary, "expected none, minmax or vector_minmax")) };
            body.parse::<syn::Token![;]>()?;
            word(&body, "columns")?;
            let columns_body; syn::bracketed!(columns_body in body);
            let mut columns = BTreeMap::new();
            while !columns_body.is_empty() {
                let column: syn::Ident = columns_body.parse()?;
                columns_body.parse::<syn::Token![:]>()?;
                let family: syn::Ident = columns_body.parse()?;
                let family = match family.to_string().as_str() { "plain" => e::Family::Plain, "bytes" => e::Family::Bytes, _ => return Err(syn::Error::new_spanned(family, "expected plain or bytes")) };
                let carrier: syn::Ident = columns_body.parse()?;
                let carrier = match carrier.to_string().as_str() { "u8" => e::Carrier::U8, "u16" => e::Carrier::U16, "u32" => e::Carrier::U32, "u64" => e::Carrier::U64, _ => return Err(syn::Error::new_spanned(carrier, "expected explicit u8/u16/u32/u64 carrier")) };
                if columns.insert(symbol_name(&column), (family, carrier)).is_some() { return Err(syn::Error::new_spanned(column, "duplicate block column")); }
                if !columns_body.is_empty() { columns_body.parse::<syn::Token![,]>()?; }
            }
            body.parse::<syn::Token![;]>()?;
            if !body.is_empty() { return Err(body.error("extra block binding fields")); }
            if result.0.insert(symbol_name(&relation), Binding { block_rows, summary, columns }).is_some() { return Err(syn::Error::new_spanned(relation, "duplicate block relation")); }
            if !input.is_empty() { input.parse::<syn::Token![,]>()?; }
        }
        Ok(result)
    }
}

pub(super) fn choose(target: &mut structures::Program, advice: &Advice, hosts: Option<&BTreeSet<String>>) -> syn::Result<()> {
    let mut facts = structures::contract::check(target)?;
    let origins = structures::representation_bases(target, &facts);
    let mut matched = BTreeSet::new();
    target.encoded.blocks.positional_hosts = hosts.cloned().unwrap_or_default();
    let mut next = 1u64;
    for definition in &target.physical.program.definitions {
        let Operator::Rename(rename) = &definition.operator else { continue; };
        let base = symbol_name(&rename.relation);
        let Some(binding) = advice.0.get(&base) else { continue; };
        let occurrence = symbol_name(&definition.name);
        let relation = target.physical.inputs().find(|r| symbol_name(&r.name) == base).unwrap();
        let names: BTreeSet<_> = relation.columns.iter().map(|c| symbol_name(&c.name)).collect();
        if binding.columns.keys().cloned().collect::<BTreeSet<_>>() != names { return Err(syn::Error::new_spanned(&definition.name, "blocks must declare every source column exactly once")); }
        let provider = facts.fallbacks.iter().find(|((name, reader, key), _)| name == &occurrence && matches!(reader, structures::ReaderRef::Scan) && key.is_empty()).map(|(_, p)| symbol_name(&p.token)).ok_or_else(|| syn::Error::new_spanned(&definition.name, "blocks require an explicit scan provider"))?;
        let source = e::SourceId(next); next += 1;
        let snapshot = e::SnapshotId(source.0);
        let mut columns = BTreeMap::new();
        let mut selected = BTreeMap::new();
        for (slot, column) in relation.columns.iter().enumerate() {
            let name = symbol_name(&column.name);
            let (family, carrier) = binding.columns[&name];
            let declaration = e::DeclarationId(next);
            let representation = e::RepresentationId(next); next += 1;
            let logical = column.logical_type_id.ok_or_else(|| syn::Error::new_spanned(&column.name, "blocks require declaration-table logical identity"))?;
            let native = match carrier { e::Carrier::U8 => 1, e::Carrier::U16 => 2, e::Carrier::U32 => 4, e::Carrier::U64 => 8 };
            let widths = if family == e::Family::Plain { BTreeSet::from([native]) } else { [0, 1, 2, native].into_iter().filter(|w| *w <= native).collect() };
            let mut buffers = BTreeSet::from([e::BufferKind::Payload, e::BufferKind::Headers]);
            match binding.summary {
                b::Summary::None => {}
                b::Summary::BlockMinMax => { buffers.insert(e::BufferKind::BlockSummaries); }
                b::Summary::VectorMinMax => { buffers.insert(e::BufferKind::VectorSummaries); }
            }
            target.encoded.obligations.declarations.insert(declaration, e::Declaration {
                provider: provider.clone(), domain: e::Domain { code: column.domain.as_ref().and_then(|d| d.id), dictionary: None },
                role: e::Role::ScanColumn, family, layout: e::Layout::S1, carrier, identity: false,
                preserves: BTreeSet::from([e::Preservation::Equality, e::Preservation::Order]), buffers,
            });
            target.encoded.records.representations.insert(representation, e::Representation { declaration, source, column: name.clone(), snapshot });
            columns.insert(name.clone(), b::Column { slot: slot as u64, logical, declaration, widths });
            selected.insert(name, representation);
        }
        target.encoded.obligations.sources.insert(source, e::Source { relation: base.clone(), occurrence: occurrence.clone(), snapshot });
        target.encoded.blocks.sources.insert(source, b::Source { block_rows: binding.block_rows, vector_rows: 1024, summary: binding.summary, columns });
        target.encoded.blocks.scans.insert(occurrence, b::Scan { source, columns: selected });
        matched.insert(base);
    }
    if matched != advice.0.keys().cloned().collect() { return Err(syn::Error::new_spanned(&target.physical, "blocks names an unused or unknown source")); }
    let active = b::positional_results(target);
    let mut entries = Vec::<structures::Entry>::new();
    for definition in &target.physical.program.definitions {
        let name = symbol_name(&definition.name);
        if !active.contains(&name) { continue; }
        if matches!(definition.operator, Operator::Rename(_) | Operator::Select(_) | Operator::Point(..) | Operator::Range(..) | Operator::Project(_) | Operator::Bind(_) | Operator::Guard(_) | Operator::Probe(..)) {
            if facts.physical.modes[&name] != ModeKind::Pipe && !(facts.physical.modes[&name] == ModeKind::Let && (definition.canon.is_some() || facts.retained.contains_key(&name) || facts.built.contains_key(&name))) { return Err(syn::Error::new_spanned(&definition.name, "blocked retained spine requires a storage binding")); }
            let mut covered = BTreeSet::new();
            let bases = origins.get(&name).ok_or_else(|| syn::Error::new_spanned(&definition.name, "blocked binding cannot enter a fused scope"))?.iter().filter_map(|origin| {
                let columns: Vec<_> = origin.columns.iter().filter_map(|(logical, host)| {
                    if !covered.insert(logical.clone()) { return None; }
                    let logical = &facts.physical.rows[&name].bindings[logical];
                    let host = syn::Ident::new(host, proc_macro2::Span::call_site());
                    Some(quote!(#logical -> #host))
                }).collect();
                if columns.is_empty() { return None; }
                let base = syn::Ident::new(&origin.base, proc_macro2::Span::call_site());
                Some(quote!(#base lineage { #(#columns),* }))
            });
            let owned = facts.physical.rows[&name].computed.iter().filter(|c| facts.physical.headings[&name].contains(c)).map(|c| &facts.physical.rows[&name].bindings[c]);
            let ident = &definition.name;
            entries.push(syn::parse2(quote!(#ident repr = positions { #(#bases),* } owned [#(#owned),*];))?);
        }
    }
    target.structures.entries.retain(|entry| !active.contains(&symbol_name(&entry.name)) || !matches!(entry.structure, structures::Structure::Repr { .. }));
    for entry in &entries {
        if let structures::Structure::Repr { repr, .. } = &entry.structure { facts.repr.insert(symbol_name(&entry.name), repr.clone()); }
    }
    target.structures.entries.extend(entries);
    for definition in &target.physical.program.definitions {
        let name = symbol_name(&definition.name);
        let input = definition.operator.input().map(symbol_name);
        let inherited = definition.operator.operands().iter().any(|i| active.contains(&symbol_name(i)));
        if !active.contains(&name) && !inherited { continue; }
        if let Operator::Probe(_, keyed) = &definition.operator {
            let left = symbol_name(&keyed.left);
            let right = symbol_name(&keyed.right);
            let mut columns: Vec<_> = keyed.columns.iter().map(symbol_name).collect(); columns.sort();
            let binding = b::index_binding(&facts, &right, &name, &columns)?;
            let keys = columns.into_iter().map(|column| Ok(b::JoinKey {
                left: b::operand(target, &facts, &origins, &left, &column)?,
                right: b::operand(target, &facts, &origins, &right, &column)?, column,
            })).collect::<syn::Result<_>>()?;
            let payload = b::payload(&facts, &origins, &right)?;
            let output = b::payload(&facts, &origins, &name)?;
            let positions = output.positions.into_iter().enumerate().map(|(output, occurrence)| {
                let (side, input) = [(b::Side::Left, &left), (b::Side::Right, &right)].into_iter().find_map(|(side, input)| {
                    b::position_slot(&target.physical.program.definitions, &facts, input, &occurrence)
                        .filter(|slot| b::position_covers(&facts, input, *slot, &name, output)).map(|slot| (side, slot))
                }).ok_or_else(|| syn::Error::new_spanned(&definition.name, "join occurrence fields require a retained input position"))?;
                Ok(b::JoinPosition { side, occurrence, input, output })
            }).collect::<syn::Result<_>>()?;
            target.encoded.blocks.joins.insert(name.clone(), b::Join { left, right, binding, keys, positions, owned: output.owned, payload });
        }
        if let Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) = &definition.operator {
            b::check_predicate_operands(&s.condition)?;
            if b::tile_source(target, &symbol_name(&s.input)).is_none() {
                let operands = s.condition.columns().into_iter().map(|column| {
                    Ok((column.clone(), b::operand(target, &facts, &origins, &symbol_name(&s.input), &column)?))
                }).collect::<syn::Result<_>>()?;
                target.encoded.blocks.carrier_predicates.insert(name.clone(), b::CarrierPredicate { input: symbol_name(&s.input), operands, preserves: b::preserved_operations(&s.condition) });
            } else {
            let source = b::tile_source(target, &symbol_name(&s.input)).unwrap();
            let occurrence = &target.encoded.obligations.sources[&source].occurrence;
            let origin = origins[&name].iter().find(|o| &o.occurrence == occurrence).unwrap();
            let scan = &target.encoded.blocks.scans[occurrence];
            let mut columns = BTreeMap::new();
            for logical in s.condition.columns() {
                let base = origin.columns.get(&logical).ok_or_else(|| syn::Error::new_spanned(&definition.name, "blocked predicate requires retained source columns"))?;
                columns.insert(logical, scan.columns[base]);
            }
            let strategy = if s.condition.range_bounds().is_some() { b::Strategy::Window } else if columns.len() == 1 && s.condition.equalities.len() + s.condition.constraints.len() == 1 && columns.keys().all(|c| structures::selection_constants(s).contains_key(c)) { b::Strategy::Point } else { b::Strategy::Formula };
            let prune = if strategy == b::Strategy::Formula { b::Prune::None } else {
                match target.encoded.blocks.sources[&scan.source].summary {
                    b::Summary::None => b::Prune::None,
                    b::Summary::BlockMinMax => b::Prune::BlockMinMax,
                    b::Summary::VectorMinMax => b::Prune::VectorMinMax,
                }
            };
            let test = if strategy == b::Strategy::Formula { b::Test::DecodeCodes } else { b::Test::Encoded };
            target.encoded.blocks.predicates.insert(name.clone(), b::Predicate { input: symbol_name(&s.input), source: scan.source, columns, prune, test, strategy, translation: b::Translation::S1Ordinal, preserves: b::preserved_operations(&s.condition) });
            }
        }
        if inherited {
            if let Some(fields) = facts.physical.rust_reads(&definition.operator) {
                let read = make_read(target, &facts, &origins, &name, input.as_ref().unwrap(), fields, false, next)?;
                next += 1;
                target.encoded.blocks.reads.insert(name.clone(), read);
            }
        }
        if active.contains(&name) && definition.canon.is_some() {
            let keys = make_canonical_keys(target, &facts, &origins, &name)?;
            target.encoded.blocks.canonical_keys.insert(name.clone(), keys);
        }
    }
    let returned = symbol_name(&target.physical.program.returned);
    if active.contains(&returned) {
        if target.physical.program.return_canon.is_some() && !target.encoded.blocks.canonical_keys.contains_key(&returned) {
            let keys = make_canonical_keys(target, &facts, &origins, &returned)?;
            target.encoded.blocks.canonical_keys.insert(returned.clone(), keys);
        }
        let consumer = symbol_name(&target.physical.program.head.relation);
        if let Some(demand) = &target.consumer {
            for (role, purpose) in [("carrier", e::ReadPurpose::PreserveCarrier), ("read", e::ReadPurpose::OpaqueRustRead)] {
                let fields = demand.head.iter().filter(|field| {
                    facts.physical.headings[&returned].contains(field)
                        && demand.value_reads.contains(field) == (purpose == e::ReadPurpose::OpaqueRustRead)
                }).cloned().collect();
                let mut read = make_read(target, &facts, &origins, &demand.original_consumer, &returned, fields, false, next)?;
                next += 1;
                read.purpose = purpose;
                if purpose == e::ReadPurpose::PreserveCarrier {
                    read.consumer = e::Consumer::Relational(demand.original_consumer.clone());
                    for field in read.fields.values_mut() { field.action = b::ReadAction::Ordinal; }
                }
                target.encoded.blocks.reads.insert(format!("output:{consumer}:{role}"), read);
            }
        } else {
            let fields = target.physical.program.head.args.iter().flat_map(|e| structures::demand::columns_used_rust(e, &facts.physical.headings[&returned], None)).collect();
            let read = make_read(target, &facts, &origins, &consumer, &returned, fields, true, next)?;
            target.encoded.blocks.reads.insert(consumer, read);
        }
    }
    structures::contract::check(target)?;
    Ok(())
}
fn make_read(target: &structures::Program, facts: &structures::Analysis, origins: &BTreeMap<String, Vec<structures::Lineage>>, name: &str, input: &str, fields: BTreeSet<String>, final_answer: bool, id: u64) -> syn::Result<b::Read> {
    let (reads, ordinary) = make_fields(target, facts, origins, input, fields)?;
    Ok(b::Read { id, input: input.into(), consumer: if final_answer { e::Consumer::FinalAnswer(name.into()) } else { e::Consumer::OpaqueRust(name.into()) }, purpose: if final_answer { e::ReadPurpose::FinalHostAnswer } else { e::ReadPurpose::OpaqueRustRead }, fields: reads, ordinary })
}
fn make_canonical_keys(target: &structures::Program, facts: &structures::Analysis, origins: &BTreeMap<String, Vec<structures::Lineage>>, input: &str) -> syn::Result<b::CanonicalKeys> {
    let (mut fields, ordinary) = make_fields(target, facts, origins, input, facts.physical.headings[input].iter().cloned().collect())?;
    for field in fields.values_mut() { field.action = b::ReadAction::Ordinal; }
    Ok(b::CanonicalKeys { input: input.into(), fields, ordinary })
}
fn make_fields(target: &structures::Program, facts: &structures::Analysis, origins: &BTreeMap<String, Vec<structures::Lineage>>, input: &str, fields: BTreeSet<String>) -> syn::Result<(BTreeMap<String, b::FieldRead>, BTreeMap<String, b::Operand>)> {
    let mut reads = BTreeMap::new();
    let mut ordinary = BTreeMap::new();
    for logical in fields {
        match b::operand(target, facts, origins, input, &logical)? {
            b::Operand::Blocked { representation, .. } => { reads.insert(logical, b::FieldRead { representation, action: b::ReadAction::Owned }); }
            operand => { ordinary.insert(logical, operand); }
        }
    }
    Ok((reads, ordinary))
}
