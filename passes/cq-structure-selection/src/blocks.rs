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

pub(super) fn choose(physical: &structures::physical::Program, target: &mut structures::Program, advice: &Advice) -> syn::Result<()> {
    let facts = structures::contract::check(physical, target)?;
    let origins = structures::representation_bases(physical, target, &facts);
    let mut matched = BTreeSet::new();
    let mut next = 1u64;
    for definition in &physical.program.definitions {
        let Operator::Rename(rename) = &definition.operator else { continue; };
        let base = symbol_name(&rename.relation);
        let Some(binding) = advice.0.get(&base) else { continue; };
        let occurrence = symbol_name(&definition.name);
        let relation = physical.inputs().find(|r| symbol_name(&r.name) == base).unwrap();
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
    if matched != advice.0.keys().cloned().collect() { return Err(syn::Error::new_spanned(&physical, "blocks names an unused or unknown source")); }
    let mut active = BTreeSet::new();
    let mut entries = Vec::new();
    for definition in &physical.program.definitions {
        let name = symbol_name(&definition.name);
        let input = definition.operator.input().map(symbol_name);
        let inherited = input.as_ref().is_some_and(|i| active.contains(i));
        if !target.encoded.blocks.scans.contains_key(&name) && !inherited { continue; }
        if matches!(definition.operator, Operator::Rename(_) | Operator::Select(_) | Operator::Point(..) | Operator::Range(..) | Operator::Project(_) | Operator::Bind(_) | Operator::Guard(_)) {
            if facts.physical.modes[&name] != ModeKind::Pipe && !(facts.physical.modes[&name] == ModeKind::Let && definition.canon.is_some()) { return Err(syn::Error::new_spanned(&definition.name, "blocked spine requires a pipe or canonical retained binding")); }
            let bases = origins.get(&name).ok_or_else(|| syn::Error::new_spanned(&definition.name, "blocked binding cannot enter a fused scope"))?.iter().map(|origin| {
                let base = syn::Ident::new(&origin.base, proc_macro2::Span::call_site());
                let columns = origin.columns.iter().map(|(logical, host)| {
                    let logical = &facts.physical.rows[&name].bindings[logical];
                    let host = syn::Ident::new(host, proc_macro2::Span::call_site());
                    quote!(#logical -> #host)
                });
                quote!(#base lineage { #(#columns),* })
            });
            let owned = facts.physical.rows[&name].computed.iter().filter(|c| facts.physical.headings[&name].contains(c)).map(|c| &facts.physical.rows[&name].bindings[c]);
            let ident = &definition.name;
            entries.push(syn::parse2(quote!(#ident repr = positions { #(#bases),* } owned [#(#owned),*];))?);
            active.insert(name.clone());
        }
        if let Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) = &definition.operator {
            b::check_predicate_operands(&s.condition)?;
            let origin = origins[&name].first().ok_or_else(|| syn::Error::new_spanned(&definition.name, "blocked predicate lacks source lineage"))?;
            let scan = &target.encoded.blocks.scans[&origin.occurrence];
            let mut columns = BTreeMap::new();
            for logical in s.condition.columns() {
                let base = origin.columns.get(&logical).ok_or_else(|| syn::Error::new_spanned(&definition.name, "blocked predicate requires retained source columns"))?;
                columns.insert(logical, scan.columns[base]);
            }
            let strategy = if s.condition.range_bounds().is_some() { b::Strategy::Window } else if columns.len() == 1 && s.condition.equalities.len() + s.condition.constraints.len() == 1 && columns.keys().all(|c| structures::selection_constants(s).contains_key(c)) { b::Strategy::Point } else { b::Strategy::Formula };
            let prune = match (target.encoded.blocks.sources[&scan.source].summary, strategy) {
                (_, b::Strategy::Formula) | (b::Summary::None, _) => b::Prune::None,
                (b::Summary::BlockMinMax, _) => b::Prune::BlockMinMax,
                (b::Summary::VectorMinMax, _) => b::Prune::VectorMinMax,
            };
            let test = if strategy == b::Strategy::Formula { b::Test::DecodeCodes } else { b::Test::Encoded };
            target.encoded.blocks.predicates.insert(name.clone(), b::Predicate { input: symbol_name(&s.input), source: scan.source, columns, prune, test, strategy, translation: b::Translation::S1Ordinal, preserves: b::preserved_operations(&s.condition) });
        }
        if inherited {
            if let Some(fields) = facts.physical.rust_reads(&definition.operator) {
                let read = make_read(target, &facts, &origins, &name, input.as_ref().unwrap(), fields, false, next)?;
                next += 1;
                target.encoded.blocks.reads.insert(name.clone(), read);
            }
        }
        if active.contains(&name) && definition.canon.is_some() {
            let mut read = make_read(target, &facts, &origins, &name, &name, facts.physical.headings[&name].iter().cloned().collect(), false, next)?;
            next += 1;
            read.consumer = e::Consumer::Relational(name.clone());
            read.purpose = e::ReadPurpose::PreserveCarrier;
            for field in read.fields.values_mut() { field.action = b::ReadAction::Ordinal; }
            target.encoded.blocks.reads.insert(format!("canon:{name}"), read);
        }
    }
    let returned = symbol_name(&physical.program.returned);
    if active.contains(&returned) {
        let canonical_key = format!("canon:{returned}");
        if physical.program.return_canon.is_some() && !target.encoded.blocks.reads.contains_key(&canonical_key) {
            let mut read = make_read(target, &facts, &origins, &returned, &returned, facts.physical.headings[&returned].iter().cloned().collect(), false, next)?;
            next += 1;
            read.consumer = e::Consumer::Relational(returned.clone());
            read.purpose = e::ReadPurpose::PreserveCarrier;
            for field in read.fields.values_mut() { field.action = b::ReadAction::Ordinal; }
            target.encoded.blocks.reads.insert(canonical_key, read);
        }
        let consumer = symbol_name(&physical.program.head.relation);
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
            let fields = physical.program.head.args.iter().flat_map(|e| structures::demand::columns_used_rust(e, &facts.physical.headings[&returned], None)).collect();
            let read = make_read(target, &facts, &origins, &consumer, &returned, fields, true, next)?;
            target.encoded.blocks.reads.insert(consumer, read);
        }
    }
    target.structures.entries.extend(entries);
    structures::contract::check(physical, target)?;
    Ok(())
}
fn make_read(target: &structures::Program, facts: &structures::Analysis, origins: &BTreeMap<String, Vec<structures::Lineage>>, name: &str, input: &str, fields: BTreeSet<String>, final_answer: bool, id: u64) -> syn::Result<b::Read> {
    let mut reads = BTreeMap::new();
    for logical in fields {
        if facts.physical.rows[input].computed.contains(&logical) { continue; }
        let representation = origins[input].iter().find_map(|o| target.encoded.blocks.scans.get(&o.occurrence).and_then(|s| o.columns.get(&logical).and_then(|c| s.columns.get(c)))).copied().ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "blocked materialization field has no source binding"))?;
        reads.insert(logical, b::FieldRead { representation, action: b::ReadAction::Owned });
    }
    Ok(b::Read { id, input: input.into(), consumer: if final_answer { e::Consumer::FinalAnswer(name.into()) } else { e::Consumer::OpaqueRust(name.into()) }, purpose: if final_answer { e::ReadPurpose::FinalHostAnswer } else { e::ReadPurpose::OpaqueRustRead }, fields: reads })
}
