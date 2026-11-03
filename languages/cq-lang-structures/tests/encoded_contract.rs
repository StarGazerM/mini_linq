//! Independent authored obligations/records, with no selector or runtime codec.
use cq_lang_structures::{self as structures, encoded::*};
use cq_syntax::DomainId;
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};

fn fields(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn fixture() -> structures::Program {
    let mut program: structures::Program = syn::parse_quote! {
        physical {
            relation R(a: u32 in [7] (symbols), b: u32 in [7] (symbols));
            r = pipe rename R@0 {a -> x, b -> y};
            s = pipe guard r if (*x != 0);
            return canon s as answer(x,y);
        }
        structures {
            providers {
                input = ArbitraryName { reads [], build typed };
                sort = Dedup { reads [distinct], build inferred, shared };
            }
            r host scan = @input as ArbitraryName [];
            s head = @sort as Dedup;
        }
    };
    let domain = Domain {
        code: Some(DomainId(7)),
        dictionary: Some(Dictionary {
            identity: "symbols".into(),
            version: 3,
        }),
    };
    let declaration = Declaration {
        provider: "input".into(),
        domain: domain.clone(),
        role: Role::ScanColumn,
        family: Family::Bytes,
        layout: Layout::S1,
        carrier: Carrier::U32,
        identity: false,
        preserves: [Preservation::Equality].into(),
        buffers: [
            BufferKind::Payload,
            BufferKind::Headers,
            BufferKind::Dictionary,
        ]
        .into(),
    };
    let retained = RetainedSource {
        source: SourceId(1),
        snapshot: SnapshotId(9),
    };
    let mut boundary = BoundaryFacts {
        location: Location::Host,
        live_count: "n".into(),
        capacity: "capacity".into(),
        readiness: "producer_ready".into(),
        retained_sources: [retained].into(),
        buffers: BTreeMap::new(),
    };
    let mut required = BTreeMap::new();
    let mut published = BTreeMap::new();
    let mut representations = BTreeMap::new();
    for (index, (logical, column)) in [("x", "a"), ("y", "b")].into_iter().enumerate() {
        let id = RepresentationId(index as u64);
        representations.insert(
            id,
            Representation {
                declaration: DeclarationId(1),
                source: SourceId(1),
                column: column.into(),
                snapshot: SnapshotId(9),
            },
        );
        required.insert(
            logical.into(),
            FieldRequirement {
                source: SourceId(1),
                column: column.into(),
                domain: domain.clone(),
                role: Role::ScanColumn,
                preserves: [Preservation::Equality].into(),
            },
        );
        let mut buffers = BTreeMap::new();
        for (offset, kind) in declaration.buffers.iter().copied().enumerate() {
            let id = BufferId((index * 10 + offset) as u64);
            buffers.insert(kind, id);
            boundary.buffers.insert(
                id,
                BufferFacts {
                    source: retained,
                    column: column.into(),
                    kind,
                    initialized_bytes: format!("{column}_{kind:?}_bytes"),
                    owner: format!("owner_{}", id.0),
                },
            );
        }
        published.insert(
            logical.into(),
            PublishedField {
                representation: id,
                buffers,
            },
        );
    }
    let read = ReadPacket {
        publication: PublicationId(1),
        boundary: BoundaryId(1),
        consumer: Consumer::OpaqueRust("s".into()),
        purpose: ReadPurpose::OpaqueRustRead,
        fields: fields(&["x"]),
    };
    program.encoded = Contract {
        obligations: Obligations {
            declarations: [(DeclarationId(1), declaration)].into(),
            sources: [(
                SourceId(1),
                Source {
                    relation: "R".into(),
                    occurrence: "r".into(),
                    snapshot: SnapshotId(9),
                },
            )]
            .into(),
            boundaries: [(BoundaryId(1), boundary)].into(),
            publications: [(
                PublicationId(1),
                PublicationObligation {
                    producer: "r".into(),
                    boundary: BoundaryId(1),
                    fields: required,
                },
            )]
            .into(),
            reads: [(ReadId(1), read.clone())].into(),
            copies: BTreeMap::new(),
        },
        records: Records {
            representations,
            publications: [(
                PublicationId(1),
                Publication {
                    boundary: BoundaryId(1),
                    fields: published,
                },
            )]
            .into(),
            reads: [(ReadId(1), read)].into(),
            copies: BTreeMap::new(),
        },
        blocks: Default::default(),
    };
    program
}

fn check(program: &structures::Program) -> syn::Result<structures::Analysis> {
    structures::contract::check(program)
}

#[test]
fn host_opaque_read_preserves_other_carriers_and_round_trips() {
    let program = fixture();
    check(&program).unwrap();
    assert_eq!(
        program.encoded.records.reads[&ReadId(1)].fields,
        fields(&["x"])
    );
    assert_eq!(
        program.encoded.records.publications[&PublicationId(1)]
            .fields
            .len(),
        2
    );
    let reparsed: structures::Program = syn::parse2(program.to_token_stream()).unwrap();
    assert_eq!(reparsed, program);
    check(&reparsed).unwrap();
}

#[test]
fn unrelated_declared_provider_cannot_supply_representation_guarantees() {
    let mut program = fixture();
    program
        .encoded
        .obligations
        .declarations
        .get_mut(&DeclarationId(1))
        .unwrap()
        .provider = "sort".into();
    assert!(check(&program).is_err());
}

#[test]
fn ordinary_program_has_no_encoded_tokens_or_default_choices() {
    let mut program = fixture();
    program.encoded = Contract::default();
    check(&program).unwrap();
    assert!(program.encoded.to_token_stream().is_empty());
    let reparsed: structures::Program = syn::parse2(program.to_token_stream()).unwrap();
    assert_eq!(reparsed, program);
    assert_eq!(reparsed.encoded, Contract::default());
}

#[test]
fn changed_read_identity_purpose_or_fields_are_rejected() {
    let mutations: &[fn(&mut ReadPacket)] = &[
        |read| read.consumer = Consumer::OpaqueRust("r".into()),
        |read| read.fields.clear(),
        |read| {
            read.fields.insert("y".into());
        },
        |read| read.boundary = BoundaryId(99),
        |read| read.purpose = ReadPurpose::FinalHostAnswer,
    ];
    for mutate in mutations {
        let mut program = fixture();
        mutate(program.encoded.records.reads.get_mut(&ReadId(1)).unwrap());
        assert!(check(&program).is_err());
    }
    let mut missing = fixture();
    missing.encoded.records.reads.clear();
    assert!(check(&missing).is_err());
}

#[test]
fn changed_domain_dictionary_role_preservation_and_snapshot_are_rejected() {
    let mutations: &[fn(&mut Declaration)] = &[
        |declaration| declaration.domain.code = Some(DomainId(8)),
        |declaration| declaration.domain.dictionary.as_mut().unwrap().version += 1,
        |declaration| declaration.domain.dictionary.as_mut().unwrap().identity = "other".into(),
        |declaration| declaration.role = Role::IndexRun,
        |declaration| declaration.preserves.clear(),
        |declaration| declaration.provider = "undeclared".into(),
    ];
    for mutate in mutations {
        let mut program = fixture();
        let mut different = program.encoded.obligations.declarations[&DeclarationId(1)].clone();
        mutate(&mut different);
        program
            .encoded
            .obligations
            .declarations
            .insert(DeclarationId(2), different);
        program
            .encoded
            .records
            .representations
            .get_mut(&RepresentationId(0))
            .unwrap()
            .declaration = DeclarationId(2);
        assert!(check(&program).is_err());
    }
    let mut stale = fixture();
    stale
        .encoded
        .records
        .representations
        .get_mut(&RepresentationId(0))
        .unwrap()
        .snapshot = SnapshotId(8);
    assert!(check(&stale).is_err());
    let mut lineage = fixture();
    lineage
        .encoded
        .obligations
        .sources
        .get_mut(&SourceId(1))
        .unwrap()
        .occurrence = "s".into();
    assert!(check(&lineage).is_err());
}

#[test]
fn missing_or_cross_column_buffers_and_unretained_sources_are_rejected() {
    let mut missing = fixture();
    missing
        .encoded
        .records
        .publications
        .get_mut(&PublicationId(1))
        .unwrap()
        .fields
        .remove("y");
    assert!(check(&missing).is_err());
    let mut missing_buffer = fixture();
    missing_buffer
        .encoded
        .records
        .publications
        .get_mut(&PublicationId(1))
        .unwrap()
        .fields
        .get_mut("x")
        .unwrap()
        .buffers
        .remove(&BufferKind::Headers);
    assert!(check(&missing_buffer).is_err());
    let mut wrong_buffer = fixture();
    let buffer = wrong_buffer.encoded.records.publications[&PublicationId(1)].fields["y"].buffers
        [&BufferKind::Payload];
    wrong_buffer
        .encoded
        .records
        .publications
        .get_mut(&PublicationId(1))
        .unwrap()
        .fields
        .get_mut("x")
        .unwrap()
        .buffers
        .insert(BufferKind::Payload, buffer);
    assert!(check(&wrong_buffer).is_err());
    let mut unretained = fixture();
    unretained
        .encoded
        .obligations
        .boundaries
        .get_mut(&BoundaryId(1))
        .unwrap()
        .retained_sources
        .clear();
    assert!(check(&unretained).is_err());
    let mut stale = fixture();
    stale
        .encoded
        .obligations
        .sources
        .get_mut(&SourceId(1))
        .unwrap()
        .snapshot = SnapshotId(10);
    assert!(check(&stale).is_err());
}

#[test]
fn different_lawful_family_passes_without_selector_equality() {
    let mut program = fixture();
    let mut alternative = program.encoded.obligations.declarations[&DeclarationId(1)].clone();
    alternative.family = Family::Plain;
    alternative.carrier = Carrier::U64;
    program
        .encoded
        .obligations
        .declarations
        .insert(DeclarationId(2), alternative);
    check(&program).unwrap();
    for representation in program.encoded.records.representations.values_mut() {
        representation.declaration = DeclarationId(2);
    }
    check(&program).unwrap();
}

#[test]
fn device_opaque_read_requires_explicit_semantic_identity() {
    let mut program = fixture();
    program
        .encoded
        .obligations
        .boundaries
        .get_mut(&BoundaryId(1))
        .unwrap()
        .location = Location::Device;
    assert!(check(&program).is_err());
    let declaration = program
        .encoded
        .obligations
        .declarations
        .get_mut(&DeclarationId(1))
        .unwrap();
    declaration.family = Family::Plain;
    // Plain dictionary codes are still not logical Rust values.
    assert!(check(&program).is_err());
    program
        .encoded
        .obligations
        .declarations
        .get_mut(&DeclarationId(1))
        .unwrap()
        .identity = true;
    check(&program).unwrap();
}

fn transport() -> structures::Program {
    let mut program = fixture();
    let mut destination = program.encoded.obligations.boundaries[&BoundaryId(1)].clone();
    destination.location = Location::Device;
    destination.readiness = "upload_done".into();
    let demanded = program.encoded.records.publications[&PublicationId(1)].fields["x"]
        .buffers
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut buffers = BTreeMap::new();
    destination.buffers = destination
        .buffers
        .into_iter()
        .filter_map(|(id, mut buffer)| {
            if !demanded.contains(&id) {
                return None;
            }
            let target = BufferId(id.0 + 100);
            buffers.insert(id, target);
            buffer.owner = format!("device_{}", target.0);
            Some((target, buffer))
        })
        .collect();
    program
        .encoded
        .obligations
        .boundaries
        .insert(BoundaryId(2), destination);
    let copy = Copy {
        publication: PublicationId(1),
        from: BoundaryId(1),
        to: BoundaryId(2),
        reason: CopyReason::CrossDevice,
        buffers,
    };
    program
        .encoded
        .obligations
        .copies
        .insert(CopyId(1), copy.clone());
    program.encoded.records.copies.insert(CopyId(1), copy);
    let read = ReadPacket {
        publication: PublicationId(1),
        boundary: BoundaryId(1),
        consumer: Consumer::Transport(CopyId(1)),
        purpose: ReadPurpose::PreserveCarrier,
        fields: fields(&["x"]),
    };
    program
        .encoded
        .obligations
        .reads
        .insert(ReadId(2), read.clone());
    program.encoded.records.reads.insert(ReadId(2), read);
    program
}

#[test]
fn cross_device_transport_copies_only_exact_demanded_initialized_buffers() {
    let program = transport();
    check(&program).unwrap();
    assert_eq!(program.encoded.records.copies[&CopyId(1)].buffers.len(), 3);
    let mutations: &[fn(&mut structures::Program)] = &[
        |program| {
            program
                .encoded
                .records
                .copies
                .get_mut(&CopyId(1))
                .unwrap()
                .reason = CopyReason::Reorganize;
        },
        |program| {
            program
                .encoded
                .records
                .copies
                .get_mut(&CopyId(1))
                .unwrap()
                .buffers
                .clear();
        },
        |program| {
            program
                .encoded
                .obligations
                .boundaries
                .get_mut(&BoundaryId(2))
                .unwrap()
                .buffers
                .values_mut()
                .next()
                .unwrap()
                .initialized_bytes = "capacity_bytes".into();
        },
        |program| {
            program
                .encoded
                .obligations
                .boundaries
                .get_mut(&BoundaryId(2))
                .unwrap()
                .live_count = "different_count".into();
        },
        |program| {
            program
                .encoded
                .obligations
                .boundaries
                .get_mut(&BoundaryId(2))
                .unwrap()
                .readiness
                .clear();
        },
        |program| {
            program
                .encoded
                .obligations
                .boundaries
                .get_mut(&BoundaryId(2))
                .unwrap()
                .buffers
                .values_mut()
                .next()
                .unwrap()
                .owner
                .clear();
        },
    ];
    for mutate in mutations {
        let mut changed = program.clone();
        mutate(&mut changed);
        assert!(check(&changed).is_err());
    }
    // Even matching forged copy records/obligations cannot broaden original
    // field demand into an all-column copy.
    let mut extra = program.clone();
    let buffer = extra.encoded.records.publications[&PublicationId(1)].fields["y"].buffers
        [&BufferKind::Payload];
    extra
        .encoded
        .obligations
        .copies
        .get_mut(&CopyId(1))
        .unwrap()
        .buffers
        .insert(buffer, BufferId(199));
    extra.encoded.records.copies = extra.encoded.obligations.copies.clone();
    assert!(check(&extra).is_err());
}

#[test]
fn transport_never_authorizes_final_answer_reconstruction() {
    let mut program = transport();
    program
        .encoded
        .obligations
        .reads
        .get_mut(&ReadId(2))
        .unwrap()
        .purpose = ReadPurpose::FinalHostAnswer;
    program.encoded.records.reads = program.encoded.obligations.reads.clone();
    assert!(check(&program).is_err());
}

#[test]
fn final_answer_is_host_only_with_exact_output_fields() {
    let mut program = fixture();
    program
        .encoded
        .obligations
        .publications
        .get_mut(&PublicationId(1))
        .unwrap()
        .producer = "s".into();
    let answer = ReadPacket {
        publication: PublicationId(1),
        boundary: BoundaryId(1),
        consumer: Consumer::FinalAnswer("answer".into()),
        purpose: ReadPurpose::FinalHostAnswer,
        fields: fields(&["x", "y"]),
    };
    program.encoded.obligations.reads = [(ReadId(1), answer.clone())].into();
    program.encoded.records.reads = [(ReadId(1), answer)].into();
    check(&program).unwrap();
    let mut wrong = program.clone();
    wrong
        .encoded
        .obligations
        .boundaries
        .get_mut(&BoundaryId(1))
        .unwrap()
        .location = Location::Device;
    assert!(check(&wrong).is_err());
    let mut missing = program.clone();
    missing
        .encoded
        .obligations
        .reads
        .get_mut(&ReadId(1))
        .unwrap()
        .fields
        .remove("y");
    missing.encoded.records.reads = missing.encoded.obligations.reads.clone();
    assert!(check(&missing).is_err());
}

#[test]
fn original_opaque_demand_is_rederived_even_if_both_packets_change() {
    let mut program = fixture();
    program
        .encoded
        .obligations
        .reads
        .get_mut(&ReadId(1))
        .unwrap()
        .fields
        .insert("y".into());
    program.encoded.records.reads = program.encoded.obligations.reads.clone();
    assert!(check(&program).is_err());
}

#[test]
fn preserved_carriers_chain_without_opaque_device_permission() {
    let mut program = fixture();
    let preserved = ReadPacket {
        publication: PublicationId(1),
        boundary: BoundaryId(1),
        consumer: Consumer::Relational("s".into()),
        purpose: ReadPurpose::PreserveCarrier,
        fields: fields(&["y"]),
    };
    program
        .encoded
        .obligations
        .reads
        .insert(ReadId(2), preserved.clone());
    program.encoded.records.reads.insert(ReadId(2), preserved);
    check(&program).unwrap();
    let duplicate = program.encoded.records.reads[&ReadId(2)].clone();
    program
        .encoded
        .obligations
        .reads
        .insert(ReadId(3), duplicate.clone());
    program.encoded.records.reads.insert(ReadId(3), duplicate);
    assert!(check(&program).is_err());
}

#[test]
fn index_role_requires_existing_byte_addressable_layout() {
    let mut program = fixture();
    for field in program
        .encoded
        .obligations
        .publications
        .get_mut(&PublicationId(1))
        .unwrap()
        .fields
        .values_mut()
    {
        field.role = Role::IndexRun;
    }
    let declaration = program
        .encoded
        .obligations
        .declarations
        .get_mut(&DeclarationId(1))
        .unwrap();
    declaration.role = Role::IndexRun;
    assert!(check(&program).is_err());
    let declaration = program
        .encoded
        .obligations
        .declarations
        .get_mut(&DeclarationId(1))
        .unwrap();
    declaration.family = Family::IndexByteAddressable;
    declaration.layout = Layout::ProviderIndex;
    assert!(check(&program).is_err());
    program.physical = syn::parse_quote! {
        physical {
            relation L(a: u32 in [7] (symbols));
            relation R(a: u32 in [7] (symbols), b: u32 in [7] (symbols));
            l = pipe rename L@1 {a -> x};
            r = let rename R@0 {a -> x, b -> y};
            s = pipe probe first [x] l,r;
            return canon s as answer(x,y);
        }
    };
    program.structures.entries.push(syn::parse_quote!(
        l host scan = @input as ArbitraryName [];
    ));
    program
        .structures
        .providers
        .declarations
        .push(syn::parse_quote!(
            index = Index { reads [exact, range, membership], build typed };
        ));
    program
        .structures
        .entries
        .push(syn::parse_quote!(r = @index as Index [x];));
    program
        .encoded
        .obligations
        .declarations
        .get_mut(&DeclarationId(1))
        .unwrap()
        .provider = "index".into();
    let read = program
        .encoded
        .obligations
        .reads
        .get_mut(&ReadId(1))
        .unwrap();
    read.consumer = Consumer::Relational("s".into());
    read.purpose = ReadPurpose::PreserveCarrier;
    program.encoded.records.reads = program.encoded.obligations.reads.clone();
    check(&program).unwrap();
}

#[test]
fn block_and_vector_summary_buffers_are_distinct_exact_demands() {
    let mut program = fixture();
    for (offset, kind) in [BufferKind::BlockSummaries, BufferKind::VectorSummaries]
        .into_iter()
        .enumerate()
    {
        program
            .encoded
            .obligations
            .declarations
            .get_mut(&DeclarationId(1))
            .unwrap()
            .buffers
            .insert(kind);
        for (index, (logical, column)) in [("x", "a"), ("y", "b")].into_iter().enumerate() {
            let buffer = BufferId(200 + (offset * 2 + index) as u64);
            program
                .encoded
                .records
                .publications
                .get_mut(&PublicationId(1))
                .unwrap()
                .fields
                .get_mut(logical)
                .unwrap()
                .buffers
                .insert(kind, buffer);
            program
                .encoded
                .obligations
                .boundaries
                .get_mut(&BoundaryId(1))
                .unwrap()
                .buffers
                .insert(
                    buffer,
                    BufferFacts {
                        source: RetainedSource {
                            source: SourceId(1),
                            snapshot: SnapshotId(9),
                        },
                        column: column.into(),
                        kind,
                        initialized_bytes: format!("{column}_{kind:?}"),
                        owner: format!("owner_{}", buffer.0),
                    },
                );
        }
    }
    check(&program).unwrap();
    program
        .encoded
        .records
        .publications
        .get_mut(&PublicationId(1))
        .unwrap()
        .fields
        .get_mut("x")
        .unwrap()
        .buffers
        .remove(&BufferKind::VectorSummaries);
    assert!(check(&program).is_err());
}

#[test]
fn optional_encoded_envelope_leaves_following_pipeline_syntax_untouched() {
    use syn::parse::Parser;
    for present in [false, true] {
        let mut program = fixture();
        if !present {
            program.encoded = Contract::default();
        }
        let parser = |input: syn::parse::ParseStream<'_>| -> syn::Result<structures::Program> {
            let program = input.parse()?;
            let next: syn::Ident = input.parse()?;
            assert_eq!(next, "sequence");
            let body;
            syn::braced!(body in input);
            assert!(body.is_empty());
            Ok(program)
        };
        let parsed = parser.parse2(quote::quote!(#program sequence {})).unwrap();
        assert_eq!(parsed, program);
    }
}

fn blocked_fixture() -> structures::Program {
    use structures::blocks as b;
    let mut program: structures::Program = syn::parse_quote! {
        physical {
            relation R(a: Alias [42], b: Alias [42]);
            r = pipe rename R@0 { a -> x, b -> y };
            p = pipe range select r if (x >= 3);
            g = pipe guard p if (opaque(x));
            return canon g as answer(x, y);
        }
        structures {
            providers {
                input = ScanProvider { reads [], build typed };
                by_value = Dedup { reads [distinct, comparator_rows], build inferred, shared };
            }
            r host scan = @input as ScanProvider [];
            r host result(p) = @input as ScanProvider [x];
            g head = @by_value as Dedup;
            r repr = positions { R lineage { x -> a, y -> b } } owned [];
            p repr = positions { R lineage { x -> a, y -> b } } owned [];
            g repr = positions { R lineage { x -> a, y -> b } } owned [];
        }
    };
    let source = SourceId(1);
    program.encoded.obligations.sources.insert(source, Source {
        relation: "R".into(), occurrence: "r".into(), snapshot: SnapshotId(9),
    });
    let mut columns = BTreeMap::new();
    let mut selected = BTreeMap::new();
    for (slot, name) in ["a", "b"].into_iter().enumerate() {
        let declaration = DeclarationId(slot as u64 + 1);
        let representation = RepresentationId(slot as u64);
        program.encoded.obligations.declarations.insert(declaration, Declaration {
            provider: "input".into(), domain: Domain { code: None, dictionary: None },
            role: Role::ScanColumn, family: Family::Bytes, layout: Layout::S1,
            carrier: Carrier::U32, identity: false,
            preserves: [Preservation::Equality, Preservation::Order].into(),
            buffers: [BufferKind::Payload, BufferKind::Headers, BufferKind::BlockSummaries].into(),
        });
        program.encoded.records.representations.insert(representation, Representation {
            declaration, source, column: name.into(), snapshot: SnapshotId(9),
        });
        columns.insert(name.into(), b::Column {
            slot: slot as u64, logical: cq_syntax::LogicalTypeId(42), declaration,
            widths: [0, 1, 2, 4].into(),
        });
        selected.insert(name.into(), representation);
    }
    program.encoded.blocks.sources.insert(source, b::Source {
        block_rows: 1024, vector_rows: 1024, summary: b::Summary::BlockMinMax, columns,
    });
    program.encoded.blocks.scans.insert("r".into(), b::Scan { source, columns: selected });
    program.encoded.blocks.predicates.insert("p".into(), b::Predicate {
        input: "r".into(), source, columns: [("x".into(), RepresentationId(0))].into(),
        prune: b::Prune::BlockMinMax, test: b::Test::Encoded, strategy: b::Strategy::Window,
        translation: b::Translation::S1Ordinal, preserves: [Preservation::Order].into(),
    });
    program.encoded.blocks.reads.insert("g".into(), b::Read {
        id: 1, input: "p".into(), consumer: Consumer::OpaqueRust("g".into()),
        purpose: ReadPurpose::OpaqueRustRead,
        fields: [("x".into(), b::FieldRead { representation: RepresentationId(0), action: b::ReadAction::Owned })].into(),
        ordinary: BTreeMap::new(),
    });
    program.encoded.blocks.reads.insert("answer".into(), b::Read {
        id: 2, input: "g".into(), consumer: Consumer::FinalAnswer("answer".into()),
        purpose: ReadPurpose::FinalHostAnswer,
        fields: [
            ("x".into(), b::FieldRead { representation: RepresentationId(0), action: b::ReadAction::Owned }),
            ("y".into(), b::FieldRead { representation: RepresentationId(1), action: b::ReadAction::Owned }),
        ].into(),
        ordinary: BTreeMap::new(),
    });
    program.encoded.blocks.canonical_keys.insert("g".into(), b::CanonicalKeys {
        input: "g".into(),
        fields: [
            ("x".into(), b::FieldRead { representation: RepresentationId(0), action: b::ReadAction::Ordinal }),
            ("y".into(), b::FieldRead { representation: RepresentationId(1), action: b::ReadAction::Ordinal }),
        ].into(),
        ordinary: BTreeMap::new(),
    });
    program
}

#[test]
fn blocked_canonicalization_requires_capability_not_provider_spelling() {
    let mut program = blocked_fixture();
    check(&program).unwrap();
    for declaration in &mut program.structures.providers.declarations {
        declaration.facts.reads.retain(|capability| capability != &structures::Capability::ComparatorRows);
    }
    assert!(check(&program).is_err());
}

#[test]
fn blocked_formulas_reject_column_free_leaves_before_printing() {
    use structures::blocks as b;
    let mut program = blocked_fixture();
    program.structures.entries.remove(1);
    program.physical.program.definitions[1].operator = syn::parse_quote!(select r if (x >= 3, x < 9));
    let predicate = program.encoded.blocks.predicates.get_mut("p").unwrap();
    predicate.strategy = b::Strategy::Formula;
    predicate.test = b::Test::DecodeCodes;
    predicate.prune = b::Prune::None;
    check(&program).unwrap();
    program.physical.program.definitions[1].operator = syn::parse_quote!(select r if (x >= 3, 1 < 2));
    assert!(check(&program).is_err());
}

#[test]
fn blocked_summary_granularity_and_provider_geometry_are_checked() {
    use structures::blocks as b;
    let mut program = blocked_fixture();
    program.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().summary = b::Summary::VectorMinMax;
    program.encoded.blocks.predicates.get_mut("p").unwrap().prune = b::Prune::VectorMinMax;
    assert!(check(&program).is_err());
    for declaration in program.encoded.obligations.declarations.values_mut() {
        declaration.buffers.insert(BufferKind::VectorSummaries);
    }
    check(&program).unwrap();
    program.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().vector_rows = 2048;
    assert!(check(&program).is_err());
}

#[test]
fn blocks_independent_authored_contract_roundtrips() {
    let program = blocked_fixture();
    check(&program).unwrap();
    let parsed: structures::Program = syn::parse2(program.to_token_stream()).unwrap();
    assert_eq!(parsed, program);
    check(&parsed).unwrap();
}

#[test]
fn blocks_mutations_cannot_change_source_predicate_or_read_obligations() {
    use structures::blocks as b;
    for mutation in 0..17 {
        let mut p = blocked_fixture();
        match mutation {
            0 => p.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().columns.get_mut("a").unwrap().slot = 1,
            1 => p.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().columns.get_mut("a").unwrap().logical = cq_syntax::LogicalTypeId(99),
            2 => p.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().block_rows = 0,
            3 => p.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().columns.get_mut("a").unwrap().widths.insert(8).then_some(()).unwrap(),
            4 => p.encoded.records.representations.get_mut(&RepresentationId(0)).unwrap().snapshot = SnapshotId(88),
            5 => p.encoded.obligations.declarations.get_mut(&DeclarationId(1)).unwrap().domain.code = Some(DomainId(77)),
            6 => p.encoded.obligations.declarations.get_mut(&DeclarationId(1)).unwrap().provider = "sort".into(),
            7 => { p.encoded.obligations.declarations.get_mut(&DeclarationId(1)).unwrap().preserves.remove(&Preservation::Order); }
            8 => { p.encoded.blocks.predicates.remove("p"); }
            9 => { p.encoded.blocks.predicates.get_mut("p").unwrap().columns.clear(); }
            10 => p.encoded.blocks.predicates.get_mut("p").unwrap().input = "g".into(),
            11 => p.encoded.blocks.predicates.get_mut("p").unwrap().strategy = b::Strategy::Point,
            12 => p.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().summary = b::Summary::None,
            13 => { p.encoded.blocks.reads.get_mut("g").unwrap().fields.clear(); }
            14 => p.encoded.blocks.reads.get_mut("answer").unwrap().fields.get_mut("y").unwrap().action = b::ReadAction::BorrowedScoped,
            15 => p.encoded.blocks.reads.get_mut("g").unwrap().purpose = ReadPurpose::PreserveCarrier,
            16 => { p.encoded.blocks.scans.get_mut("r").unwrap().columns.remove("b"); }
            _ => unreachable!(),
        }
        assert!(check(&p).is_err(), "blocked mutation {mutation}");
    }
}

#[test]
fn blocks_accept_legal_nonpreferred_physical_code_tests_and_plain() {
    use structures::blocks as b;
    let mut program = blocked_fixture();
    let predicate = program.encoded.blocks.predicates.get_mut("p").unwrap();
    predicate.prune = b::Prune::None;
    predicate.test = b::Test::DecodeCodes;
    predicate.strategy = b::Strategy::Formula;
    program.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().summary = b::Summary::None;
    for declaration in program.encoded.obligations.declarations.values_mut() {
        declaration.family = Family::Plain;
    }
    for column in program.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().columns.values_mut() {
        column.widths = [4].into();
    }
    check(&program).unwrap();
    program.encoded.blocks.reads.get_mut("g").unwrap().fields.get_mut("x").unwrap().action = b::ReadAction::BorrowedScoped;
    check(&program).unwrap();
}

#[test]
fn blocks_source_obligations_are_immutable_across_rewrites() {
    let source = blocked_fixture();
    let mut target = source.clone();
    target.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().block_rows = 3072;
    // A separately authored provider may use three vectors per block. It is
    // not permission for a rewrite to change the original bound source.
    check(&target).unwrap();
    assert!(structures::check_rewrite(&source, &target).is_err());
    assert!(structures::check_retention_rewrite(&source, &target).is_err());
}

fn blocked_external_fixture() -> structures::Program {
    use structures::blocks as b;
    let mut program = blocked_fixture();
    program.consumer = Some(structures::OutputConsumer {
        original_consumer: "outer".into(),
        head: vec!["x".into(), "y".into()],
        value_reads: vec![],
    });
    program.encoded.blocks.reads.remove("answer");
    program.encoded.blocks.reads.insert("output:answer:carrier".into(), b::Read {
        id: 2, input: "g".into(), consumer: Consumer::Relational("outer".into()),
        purpose: ReadPurpose::PreserveCarrier,
        fields: [
            ("x".into(), b::FieldRead { representation: RepresentationId(0), action: b::ReadAction::Ordinal }),
            ("y".into(), b::FieldRead { representation: RepresentationId(1), action: b::ReadAction::Ordinal }),
        ].into(),
        ordinary: BTreeMap::new(),
    });
    program.encoded.blocks.reads.insert("output:answer:read".into(), b::Read {
        id: 4, input: "g".into(), consumer: Consumer::OpaqueRust("outer".into()),
        purpose: ReadPurpose::OpaqueRustRead,
        fields: BTreeMap::new(),
        ordinary: BTreeMap::new(),
    });
    program
}

#[test]
fn blocks_external_consumer_checks_exact_roles_and_injective_value_codes() {
    use structures::blocks as b;
    let program = blocked_external_fixture();
    check(&program).unwrap();
    let parsed: structures::Program = syn::parse2(program.to_token_stream()).unwrap();
    assert_eq!(parsed, program);
    check(&parsed).unwrap();
    for mutation in 0..9 {
        let mut forged = program.clone();
        match mutation {
            0 => forged.consumer.as_mut().unwrap().value_reads.push("x".into()),
            1 => forged.encoded.blocks.reads.get_mut("output:answer:carrier").unwrap().fields.get_mut("y").unwrap().action = b::ReadAction::Owned,
            2 => forged.encoded.blocks.reads.get_mut("output:answer:read").unwrap().purpose = ReadPurpose::FinalHostAnswer,
            3 => forged.encoded.blocks.reads.get_mut("output:answer:read").unwrap().consumer = Consumer::OpaqueRust("invented".into()),
            4 => { forged.encoded.obligations.declarations.get_mut(&DeclarationId(2)).unwrap().preserves.remove(&Preservation::Equality); }
            5 => forged.encoded.obligations.declarations.get_mut(&DeclarationId(2)).unwrap().layout = Layout::ProviderIndex,
            6 => forged.encoded.blocks.reads.get_mut("output:answer:carrier").unwrap().fields.get_mut("y").unwrap().representation = RepresentationId(0),
            7 => { forged.encoded.blocks.reads.get_mut("output:answer:carrier").unwrap().fields.clear(); }
            8 => {
                forged.consumer.as_mut().unwrap().value_reads.push("x".into());
                let field = forged.encoded.blocks.reads.get_mut("output:answer:carrier").unwrap().fields.remove("x").unwrap();
                forged.encoded.blocks.reads.get_mut("output:answer:read").unwrap().fields.insert("x".into(), b::FieldRead { action: b::ReadAction::Owned, ..field });
            }
            _ => unreachable!(),
        }
        assert!(check(&forged).is_err(), "external consumer mutation {mutation}");
    }
}

#[test]
fn blocks_external_demand_cannot_be_forged_together_with_read_records() {
    let source = blocked_external_fixture();
    let mut target = source.clone();
    target.consumer.as_mut().unwrap().original_consumer = "different_outer".into();
    target.encoded.blocks.reads.get_mut("output:answer:carrier").unwrap().consumer = Consumer::Relational("different_outer".into());
    target.encoded.blocks.reads.get_mut("output:answer:read").unwrap().consumer = Consumer::OpaqueRust("different_outer".into());
    // Locally valid for a DIFFERENT original consumer; not a rewrite of this one.
    check(&target).unwrap();
    assert!(structures::check_rewrite(&source, &target).is_err());
    assert!(structures::check_retention_rewrite(&source, &target).is_err());
}

#[test]
fn blocks_external_value_codes_accept_plain_nonpreferred_representation() {
    let mut program = blocked_external_fixture();
    for declaration in program.encoded.obligations.declarations.values_mut() {
        declaration.family = Family::Plain;
    }
    for column in program.encoded.blocks.sources.get_mut(&SourceId(1)).unwrap().columns.values_mut() {
        column.widths = [4].into();
    }
    check(&program).unwrap();
}

/// Independently authored self-join records: no Structures selector is involved.
fn blocked_join_fixture() -> structures::Program {
    use structures::blocks as b;
    let mut program: structures::Program = syn::parse_quote! {
        physical {
            relation R(a: Alias [42], b: Alias [42]);
            r = pipe rename R@0 {a -> x, b -> y};
            s = pipe rename R@1 {a -> x, b -> z};
            j = pipe probe first [x] r,s;
            p = pipe select j if ((y < 3 and z > 9) or (y > 7 and z < 2));
            return canon p as answer(x,y,z);
        }
        structures {
            providers {
                input = ScanProvider { reads [], build typed };
                exact = ArbitraryExactProvider { reads [exact], build inferred };
                by_value = Dedup { reads [distinct, comparator_rows], build inferred, shared };
            }
            r host scan = @input as ScanProvider [];
            s host scan = @input as ScanProvider [];
            s host result(j) = @exact as ArbitraryExactProvider [x];
            p head = @by_value as Dedup;
            r repr = positions { R lineage {x -> a,y -> b} } owned [];
            s repr = positions { R lineage {x -> a,z -> b} } owned [];
            j repr = positions { R lineage {x -> a,y -> b}, R lineage {x -> a,z -> b} } owned [];
            p repr = positions { R lineage {x -> a,y -> b}, R lineage {x -> a,z -> b} } owned [];
        }
    };
    let base = blocked_fixture();
    program.encoded = base.encoded;
    program.encoded.blocks.predicates.clear();
    program.encoded.blocks.reads.clear();
    program.encoded.blocks.canonical_keys.clear();
    program.encoded.obligations.sources.insert(SourceId(2), Source {
        relation: "R".into(), occurrence: "s".into(), snapshot: SnapshotId(10),
    });
    let binding = program.encoded.blocks.sources[&SourceId(1)].clone();
    program.encoded.blocks.sources.insert(SourceId(2), binding);
    for (id, column, declaration) in [(2, "a", 1), (3, "b", 2)] {
        program.encoded.records.representations.insert(RepresentationId(id), Representation {
            declaration: DeclarationId(declaration), source: SourceId(2),
            column: column.into(), snapshot: SnapshotId(10),
        });
    }
    program.encoded.blocks.scans.insert("s".into(), b::Scan {
        source: SourceId(2), columns: [("a".into(), RepresentationId(2)), ("b".into(), RepresentationId(3))].into(),
    });
    program.encoded.blocks.joins.insert("j".into(), b::Join {
        left: "r".into(), right: "s".into(),
        binding: b::IndexBinding { kind: b::IndexKind::Host, provider: "exact".into(), levels: vec![vec!["x".into()]] },
        keys: vec![b::JoinKey { column: "x".into(),
            left: b::Operand::Blocked { representation: RepresentationId(0), position: 0 },
            right: b::Operand::Blocked { representation: RepresentationId(2), position: 0 } }],
        positions: vec![
            b::JoinPosition { side: b::Side::Left, occurrence: "r".into(), input: 0, output: 0 },
            b::JoinPosition { side: b::Side::Right, occurrence: "s".into(), input: 0, output: 1 },
        ],
        owned: vec![], payload: b::Payload { positions: vec!["s".into()], owned: vec![] },
    });
    program.encoded.blocks.carrier_predicates.insert("p".into(), b::CarrierPredicate {
        input: "j".into(),
        operands: [
            ("y".into(), b::Operand::Blocked { representation: RepresentationId(1), position: 0 }),
            ("z".into(), b::Operand::Blocked { representation: RepresentationId(3), position: 1 }),
        ].into(),
        preserves: [Preservation::Order].into(),
    });
    let reads = |action| [("x", 0), ("y", 1), ("z", 3)].into_iter().map(|(column, id)| {
        (column.into(), b::FieldRead { representation: RepresentationId(id), action })
    }).collect();
    program.encoded.blocks.canonical_keys.insert("p".into(), b::CanonicalKeys {
        input: "p".into(), fields: reads(b::ReadAction::Ordinal), ordinary: BTreeMap::new(),
    });
    program.encoded.blocks.reads.insert("answer".into(), b::Read {
        id: 2, input: "p".into(), consumer: Consumer::FinalAnswer("answer".into()),
        purpose: ReadPurpose::FinalHostAnswer, ordinary: BTreeMap::new(), fields: reads(b::ReadAction::Owned),
    });
    program
}

#[test]
fn blocked_join_independent_records_preserve_occurrences_and_exact_formula() {
    let program = blocked_join_fixture();
    let facts = check(&program).unwrap();
    assert_eq!(structures::blocks::position_slot(&program.physical.program.definitions, &facts, "p", "r"), Some(0));
    assert_eq!(structures::blocks::position_slot(&program.physical.program.definitions, &facts, "p", "s"), Some(1));
    assert_eq!(syn::parse2::<structures::Program>(program.to_token_stream()).unwrap(), program);
    assert!(program.encoded.blocks.predicates.is_empty(), "correlation is not a tile predicate");
    assert!(!program.encoded.blocks.reads.keys().any(|key| key.starts_with("canon:")));
    assert_eq!(program.encoded.blocks.canonical_keys["p"].input, "p");
}

#[test]
fn blocked_join_independent_mutations_reject_missing_and_forged_contracts() {
    use structures::blocks as b;
    let program = blocked_join_fixture();
    for mutate in [
        (|p: &mut structures::Program| { p.encoded.blocks.joins.clear(); }) as fn(&mut structures::Program),
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().keys.clear(); },
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().keys[0].column = "y".into(); },
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().keys[0].right = b::Operand::Blocked { representation: RepresentationId(0), position: 0 }; },
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().binding.provider = "input".into(); },
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().binding.levels.clear(); },
        |p| { let key = p.encoded.blocks.joins["j"].keys[0].clone(); p.encoded.blocks.joins.get_mut("j").unwrap().keys.push(key); },
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().payload.positions.clear(); },
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().payload.owned.push("x".into()); },
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().positions[1].occurrence = "r".into(); },
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().positions[1].input = 1; },
        |p| { p.encoded.blocks.joins.get_mut("j").unwrap().positions[1].output = 0; },
        |p| { p.encoded.blocks.carrier_predicates.clear(); },
        |p| { p.encoded.blocks.carrier_predicates.get_mut("p").unwrap().operands.remove("z"); },
        |p| { p.encoded.blocks.carrier_predicates.get_mut("p").unwrap().operands.insert("z".into(), b::Operand::Blocked { representation: RepresentationId(1), position: 0 }); },
        |p| { p.encoded.blocks.carrier_predicates.get_mut("p").unwrap().preserves.clear(); },
        |p| { p.encoded.blocks.canonical_keys.get_mut("p").unwrap().fields.remove("z"); },
        |p| { p.encoded.blocks.canonical_keys.clear(); },
        |p| { p.encoded.blocks.canonical_keys.get_mut("p").unwrap().input = "j".into(); },
        |p| { p.encoded.blocks.canonical_keys.get_mut("p").unwrap().fields.get_mut("z").unwrap().action = b::ReadAction::Owned; },
        |p| { let read = p.encoded.blocks.reads["answer"].clone(); p.encoded.blocks.reads.insert("canon:p".into(), read); },
        |p| { p.encoded.blocks.reads.get_mut("answer").unwrap().ordinary.insert("z".into(), b::Operand::Owned { column: "z".into() }); },
        |p| { p.encoded.records.representations.get_mut(&RepresentationId(3)).unwrap().snapshot = SnapshotId(9); },
    ] {
        let mut invalid = program.clone();
        mutate(&mut invalid);
        assert!(check(&invalid).is_err());
    }
}

#[test]
fn blocked_join_accepts_nonpreferred_provider_and_plain_family() {
    let mut program = blocked_join_fixture();
    for entry in &mut program.structures.entries {
        if let Some(provider) = entry.structure.provider_mut() {
            if provider.token == "exact" { provider.ty = syn::parse_quote!(AnotherLawfulIndex); }
        }
    }
    program.structures.providers.declarations.iter_mut().find(|d| d.token == "exact").unwrap().ty = syn::parse_quote!(AnotherLawfulIndex);
    for declaration in program.encoded.obligations.declarations.values_mut() { declaration.family = Family::Plain; }
    for source in program.encoded.blocks.sources.values_mut() {
        for column in source.columns.values_mut() { column.widths = [4].into(); }
    }
    check(&program).unwrap();
}

#[test]
fn blocked_join_accepts_an_independently_reordered_occurrence_layout() {
    use structures::blocks as b;
    let mut program = blocked_join_fixture();
    for entry in &mut program.structures.entries {
        if entry.name == "j" || entry.name == "p" {
            if let structures::Structure::Repr { repr: structures::Repr::Positions { bases, .. }, .. } = &mut entry.structure {
                *bases = bases.iter().rev().cloned().collect();
            }
        }
    }
    let join = program.encoded.blocks.joins.get_mut("j").unwrap();
    join.positions[0].output = 1;
    join.positions[1].output = 0;
    for (name, slot) in [("y", 1), ("z", 0)] {
        if let b::Operand::Blocked { position, .. } = program.encoded.blocks.carrier_predicates.get_mut("p").unwrap().operands.get_mut(name).unwrap() {
            *position = slot;
        }
    }
    let facts = check(&program).unwrap();
    assert_eq!(b::position_occurrence(&program.physical.program.definitions, &facts, "p", 0).as_deref(), Some("s"));
    assert_eq!(syn::parse2::<structures::Program>(program.to_token_stream()).unwrap(), program);
}
