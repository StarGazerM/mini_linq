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
