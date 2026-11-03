use cq_lang_sequence::structures::Capability;
use cq_lang_sequence::*;
use quote::{ToTokens, quote};

fn program(body: proc_macro2::TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation R(a:i32);
            r = let canon rename R@0 {a -> a};
            return r as answer(a);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            r host scan = @reader as Reader [];
            r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows;
        }
        sequence { #body }
    })
    .unwrap()
}

#[test]
fn retired_optional_filter_statement_is_rejected() {
    assert!(
        syn::parse2::<Sequence>(quote! {
            sequence {
                result = store rows r;
                optional_filter r;
            }
        })
        .is_err()
    );
}

#[test]
fn publication_distinguishes_borrow_alias_and_real_retained_rows() {
    let p = program(quote! {
        borrowed = store rows r;
        view = borrowed;
        borrowed_again = store view;
        retained = store canon borrowed;
        retained_parts = partition_k[a](4) retained;
        alias = store retained;
        alias_parts = partition_k[a](4) alias;
    });
    let analysis = contract::check(&p).unwrap();
    let borrow = Publication::BorrowHost {
        result: "r".into(),
        host: "R".into(),
    };
    assert_eq!(analysis.publications["borrowed"], borrow);
    assert_eq!(analysis.publications["borrowed_again"], borrow);
    assert!(!analysis.publications.contains_key("view"));
    assert_eq!(analysis.host_sources["view"], "r");
    assert_eq!(
        analysis.publications["retained"],
        Publication::OwnRows { staged: true }
    );
    assert_eq!(
        analysis.publications["alias"],
        Publication::Alias {
            source: "retained".into()
        }
    );
    assert!(!analysis.host_sources.contains_key("retained"));
}

#[test]
fn publication_alias_preserves_original_carrier_across_reheading() {
    let p: Program = syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = let canon rename R@0 {a -> a};
            s = let canon project r keep {a};
            return s as answer(a);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; } r host scan = @reader as Reader []; r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows; s rows = @sort as ::cq_storage::dedup::Sort; s canon = rows; }
        sequence {
            input = store canon rows r;
            input_parts = partition_k[a](4) input;
            s = store input;
            out = flatten partition_k[a](4) s;
        }
    };
    let analysis = contract::check(&p).unwrap();
    assert_eq!(analysis.results["input"], "r");
    assert_eq!(analysis.results["s"], "s");
    assert_eq!(
        analysis.publications["s"],
        Publication::Alias {
            source: "input".into()
        }
    );
    assert!(!analysis.host_sources.contains_key("s"));
}

#[test]
fn routed_publication_checks_native_representation_and_stored_key_coverage() {
    let mut p = lookup_program(quote! {
        input = store rows r;
        r = store partition_k[a](4) input;
        out = rows l;
    });
    let entry = p
        .structures
        .structures
        .entries
        .iter_mut()
        .find(|entry| matches!(entry.structure, structures::Structure::Built { .. }))
        .unwrap();
    let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
        unreachable!()
    };
    keyed.provider = syn::parse_quote!(
        @sharded(@provider as Provider) as ::cq_storage::sharded::Sharded<Provider, _>
    );
    p.structures
        .structures
        .declare(&[syn::parse_quote!(
            sharded = ::cq_storage::sharded::Sharded<_> { reads [], build typed, routed 0, shared };
        )])
        .unwrap();
    let analysis = contract::check(&p).unwrap();
    assert_eq!(
        analysis.publications["r"],
        Publication::Parts {
            keys: vec!["a".into()]
        }
    );
    assert_eq!(
        analysis.routed_providers["r"][&0]
            .to_token_stream()
            .to_string(),
        quote!(@provider as Provider).to_string()
    );

    p.sequence.definitions[1].term = syn::parse_quote!(store partition_k[b](4) input);
    reject(&p, "BE-Publication");
    p.sequence.definitions[1].term = syn::parse_quote!(store partition_k[a](4) input);
    let entry = p
        .structures
        .structures
        .entries
        .iter_mut()
        .find(|entry| matches!(entry.structure, structures::Structure::Built { .. }))
        .unwrap();
    let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
        unreachable!()
    };
    keyed.provider = syn::parse_quote!(@sharded as ::cq_storage::sharded::Sharded);
    p.structures.structures.declare(&[]).unwrap();
    assert!(contract::check(&p).is_err());
}

#[test]
fn a_non_routed_token_is_not_a_routed_provider() {
    let mut p = lookup_program(quote! { input = store rows r; out = rows l; });
    let entry = p
        .structures
        .structures
        .entries
        .iter_mut()
        .find(|entry| matches!(entry.structure, structures::Structure::Built { .. }))
        .unwrap();
    let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
        unreachable!()
    };
    keyed.provider = syn::parse_quote!(@custom_sharded as ::custom::Sharded<Provider>);
    p.structures
        .structures
        .declare(&[syn::parse_quote!(
            custom_sharded = ::custom::Sharded<_> { reads [exact, membership], build typed };
        )])
        .unwrap();
    let analysis = contract::check(&p).unwrap();
    assert!(analysis.routed_providers.is_empty());
    assert_eq!(
        analysis.publications["input"],
        Publication::OwnRows { staged: false }
    );
}

fn deferred_publication_program(body: proc_macro2::TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation R(a:i32,b:i32); relation P(a:i32);
            r = let canon rename R@0 {a -> a,b -> b};
            selected = let select r if (b > 0);
            probe = pipe rename P@1 {a -> a};
            out = let canon probe first [a] probe,selected;
            return out as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; sharded = ::cq_storage::sharded::Sharded<_> { reads [], build typed, routed 0, shared }; provider = Provider { reads [exact, range, membership], build inferred }; }
            r host scan = @reader as Reader [];
            r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows;
            selected = @sharded(@provider as Provider) as ::cq_storage::sharded::Sharded<Provider> [a];
            probe host scan = @reader as Reader [];
            out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
        }
        sequence { #body }
    })
    .unwrap()
}

fn deferred_cover() -> proc_macro2::TokenStream {
    quote! {
        selected = store choose [
            workers < 2 => choose [
                len R < 16 => partition_k[a](1) input,
                len input < 16 => partition_k[a](1) input,
                else => partition_k[a](4) input
            ],
            len input < 8 => partition_k[a](1) input,
            else => partition_k[a](4) input
        ];
    }
}

#[test]
fn deferred_rows_accept_total_selection_and_pure_named_producer() {
    let cover = deferred_cover();
    for producer in [
        quote!(input = store compact selected rows r;),
        quote! {
            producer = compact selected rows r;
            input = store producer;
        },
        quote! {
            scan = rows r;
            producer = compact selected scan;
            input = store producer;
        },
        quote! {
            scan = store rows r;
            producer = compact selected scan;
            input = store producer;
        },
    ] {
        let p = deferred_publication_program(quote! {
            #producer
            #cover
            out = flatten lookup selected [a] rows probe;
        });
        let facts = contract::check(&p).unwrap();
        assert_eq!(
            facts.publications["input"],
            Publication::DeferredRows {
                consumer: "selected".into()
            }
        );
        assert_eq!(
            facts.publications["selected"],
            Publication::Parts {
                keys: vec!["a".into()]
            }
        );
    }
}

#[test]
fn deferred_rows_accept_direct_and_recursively_nested_worker_covers() {
    for worker in [
        quote!(partition_k[a](1) input),
        quote!(choose [
            len input < 16 => partition_k[a](1) input,
            else => partition_k[a](4) input
        ]),
        quote!(choose [
            len R < 16 => partition_k[a](1) input,
            else => choose [
                len input < 16 => partition_k[a](1) input,
                else => partition_k[a](4) input
            ]
        ]),
    ] {
        let p = deferred_publication_program(quote! {
            input = store compact selected rows r;
            selected = store choose [
                workers < 2 => #worker,
                else => choose [
                    len input < 8 => partition_k[a](1) input,
                    else => partition_k[a](4) input
                ]
            ];
            out = flatten lookup selected [a] rows probe;
        });
        assert_eq!(
            contract::check(&p).unwrap().publications["input"],
            Publication::DeferredRows {
                consumer: "selected".into()
            }
        );
    }
}

#[test]
fn deferred_rows_require_the_recorded_leading_worker_singleton() {
    for cover in [
        quote!(selected = store partition_k[a](4) input;),
        quote!(selected = store choose [
            len input < 8 => partition_k[a](1) input,
            else => partition_k[a](4) input
        ];),
        quote!(selected = store choose [
            workers < 3 => partition_k[a](1) input,
            else => partition_k[a](4) input
        ];),
        quote!(selected = store choose [
            workers < 2 => partition_k[a](4) input,
            else => partition_k[a](1) input
        ];),
        quote!(selected = store choose [
            len input < 8 => partition_k[a](1) input,
            workers < 2 => partition_k[a](1) input,
            else => partition_k[a](4) input
        ];),
        quote!(selected = store choose [
            workers < 2 => choose [
                len R < 16 => partition_k[a](4) input,
                else => partition_k[a](4) input
            ],
            else => partition_k[a](1) input
        ];),
    ] {
        let p = deferred_publication_program(quote! {
            input = store compact selected rows r;
            #cover
            out = flatten lookup selected [a] rows probe;
        });
        assert_eq!(
            contract::check(&p).unwrap().publications["input"],
            Publication::OwnRows { staged: true }
        );
    }
}

#[test]
fn deferred_rows_reject_shared_raw_rows_lengths_and_consumer_replay() {
    let cover = deferred_cover();
    for extra in [
        quote!(shared = input;),
        quote!(shared = store input;),
        quote!(shared = fold count input;),
        quote!(shared = choose [len input < 8 => rows r, else => rows r];),
        quote!(shared = flatten selected;),
        quote!(shared = selected;),
        quote!(shared = choose [len selected < 8 => rows r, else => rows r];),
        quote!(shared = rows selected;),
        quote!(shared = store partition_k[a](4) input;),
    ] {
        let p = deferred_publication_program(quote! {
            input = store compact selected rows r;
            #cover
            #extra
            out = flatten lookup selected [a] rows probe;
        });
        assert_eq!(
            contract::check(&p).unwrap().publications["input"],
            Publication::OwnRows { staged: true },
            "{extra}"
        );
    }
    let p = deferred_publication_program(quote! {
        input = store compact selected rows r;
        #cover
    });
    assert_eq!(
        contract::check(&p).unwrap().publications["input"],
        Publication::OwnRows { staged: true },
        "a final Parts output replays the publication"
    );
}

#[test]
fn deferred_rows_do_not_hide_prior_materialization_or_source_aliases() {
    let cover = deferred_cover();
    let p = deferred_publication_program(quote! {
        materialized = store compact selected rows r;
        producer = materialized;
        input = store producer;
        #cover
        out = flatten lookup selected [a] rows probe;
    });
    assert_eq!(
        contract::check(&p).unwrap().publications["input"],
        Publication::OwnRows { staged: true }
    );
    let p = deferred_publication_program(quote! {
    input = store compact selected rows r;
    alias = store input;
    selected = store choose [
        workers < 2 => choose [
            len R < 16 => partition_k[a](1) input,
            len input < 16 => partition_k[a](1) input,
            else => partition_k[a](4) alias
        ],
        else => partition_k[a](4) alias
    ];
    out = flatten lookup selected [a] rows probe;
    });
    assert_eq!(
        contract::check(&p).unwrap().publications["input"],
        Publication::OwnRows { staged: true }
    );
}

#[test]
fn deferred_rows_do_not_cross_source_publication_rebindings() {
    let cover = deferred_cover();
    for (between, expected) in [
        (
            quote!(view = rows r;),
            Publication::DeferredRows {
                consumer: "selected".into(),
            },
        ),
        (
            quote!(rebound = store rows r;),
            Publication::OwnRows { staged: true },
        ),
    ] {
        let p = deferred_publication_program(quote! {
            input = store compact selected rows r;
            #between
            #cover
            out = flatten lookup selected [a] rows probe;
        });
        assert_eq!(contract::check(&p).unwrap().publications["input"], expected);
    }
}

#[test]
fn deferred_rows_allow_total_rust_dependencies() {
    let cover = deferred_cover();
    for opaque in [
        quote!(g = pipe guard r if (opaque_predicate(a));),
        quote!(g = pipe bind r with c:i32 = (opaque_value(a));),
    ] {
        let mut p = deferred_publication_program(quote! {
            producer = compact selected rows r;
            input = store producer;
            #cover
            out = flatten lookup selected [a] rows probe;
        });
        p.structures
            .physical
            .program
            .definitions
            .insert(1, syn::parse2(opaque).unwrap());
        p.structures
            .physical
            .program
            .definitions
            .insert(2, syn::parse_quote!(clean = pipe project g keep {a,b};));
        p.structures.physical.program.definitions[3] =
            syn::parse_quote!(selected = let select clean if (b > 0););
        assert_eq!(
            contract::check(&p).unwrap().publications["input"],
            Publication::DeferredRows {
                consumer: "selected".into()
            }
        );
    }
}

#[test]
fn deferred_rows_reject_retained_rows_and_multiple_native_builds() {
    let cover = deferred_cover();
    for (multiple, extra) in [
        (false, quote!(selected rows = @rows as Rows;)),
        (
            true,
            quote! {
                selected rows = @rows as Rows;
                selected = @sharded(@other_provider as OtherProvider)
                    as ::cq_storage::sharded::Sharded<OtherProvider> [a,b];
            },
        ),
    ] {
        let mut p = deferred_publication_program(quote! {
            input = store compact selected rows r;
            #cover
            out = flatten lookup selected [a] rows probe;
        });
        let additional: structures::Structures = syn::parse2(quote!(structures {
            providers {
                rows = Rows { reads [], build inferred, accounting rows };
                other_provider = OtherProvider { reads [exact, range, membership], build inferred };
            }
            #extra
        }))
        .unwrap();
        p.structures.structures.entries.extend(additional.entries);
        p.structures
            .structures
            .declare(&additional.providers.declarations)
            .unwrap();
        if multiple {
            *p.structures
                .physical
                .program
                .definitions
                .last_mut()
                .unwrap() = syn::parse_quote!(first = pipe probe first [a] probe,selected;);
            p.structures
                .physical
                .program
                .definitions
                .push(syn::parse_quote!(out = let canon probe first [a,b] first,selected;));
            p.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(flatten lookup selected [a,b]
                    flatten lookup selected [a] rows probe);
        }
        assert_eq!(
            contract::check(&p).unwrap().publications["input"],
            Publication::OwnRows { staged: true }
        );
    }
}

#[test]
fn deferred_rows_reject_support_equal_source_and_consumer_reheading() {
    for source_alias in [false, true] {
        let input = if source_alias {
            quote!(alias)
        } else {
            quote!(input)
        };
        let consumer = if source_alias {
            quote!(selected)
        } else {
            quote!(alias)
        };
        let p: Program = syn::parse2(quote! {
            physical {
                relation R(a:i32,b:i32); relation P(a:i32);
                r = let canon rename R@0 {a -> a,b -> b};
                selected = let select r if (b > 0);
                alias = let project selected keep {a,b};
                probe = pipe rename P@1 {a -> a};
                first = pipe probe first [a] probe,selected;
                out = let canon probe first [a,b] first,alias;
                return out as answer(a,b);
            }
            structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; sharded = ::cq_storage::sharded::Sharded<_> { reads [], build typed, routed 0, shared }; provider = Provider { reads [exact, range, membership], build inferred }; rows = Rows { reads [exact, range, membership], build inferred }; alias_provider = AliasProvider { reads [exact, range, membership], build inferred }; }
                r host scan = @reader as Reader [];
                r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows;
                probe host scan = @reader as Reader [];
                selected = @sharded(@provider as Provider) as ::cq_storage::sharded::Sharded<Provider> [a];
                selected rows = @rows as Rows;
                alias = @sharded(@alias_provider as AliasProvider) as ::cq_storage::sharded::Sharded<AliasProvider> [a,b];
                out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
            }
            sequence {
                #input = store compact selected rows r;
                #consumer = store choose [
                    workers < 2 => choose [
                        len R < 16 => partition_k[a](1) #input,
                        len #input < 16 => partition_k[a](1) #input,
                        else => partition_k[a](4) #input
                    ],
                    else => partition_k[a](4) #input
                ];
                out = flatten lookup selected [a] rows probe;
            }
        })
        .unwrap();
        let facts = contract::check(&p).unwrap();
        assert_eq!(facts.results["alias"], "alias");
        assert_eq!(
            facts.publications[&input.to_string()],
            Publication::OwnRows { staged: true }
        );
    }
}

#[test]
fn deferred_rows_pin_sources_with_shadowed_join_filter_bindings() {
    let mut p: Program = syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64); relation O(a:i64);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a};
            b = let select r if (a > 0);
            selected = let probe first [a] l,b;
            probe = pipe rename O@2 {a -> a};
            out = let canon probe first [a] probe,selected;
            return out as answer(a);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; hash_index = ::cq_storage::HashIndex<'_, _, _> { reads [exact, membership], supports [keys], build typed, join_filter }; filter = Filter { reads [exact, range, membership], build inferred }; sharded = ::cq_storage::sharded::Sharded<_> { reads [], build typed, routed 0, shared }; provider = Provider { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            l host scan = @reader as Reader [];
            r host scan = @reader as Reader [];
            b = @hash_index as ::cq_storage::HashIndex<'_, _, _> [a] supports [keys];
            l join_filter = @filter as Filter  build b [a] covering selected marker ::cq_load::ColumnAt<0>;
            selected = @sharded(@provider as Provider) as ::cq_storage::sharded::Sharded<Provider> [a];
            probe host scan = @reader as Reader [];
            out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
        }
        sequence {
            source = rows l;
            b = store compact b rows r;
            input = store flatten map selected source;
            selected = store choose [
                workers < 2 => choose [
                    len L < 16 => partition_k[a](1) input,
                    len input < 16 => partition_k[a](1) input,
                    else => partition_k[a](4) input
                ],
                else => partition_k[a](4) input
            ];
            answer = flatten lookup selected [a] rows probe;
        }
    };
    assert_eq!(
        contract::check(&p).unwrap().publications["input"],
        Publication::OwnRows { staged: true }
    );
    p.structures
        .structures
        .entries
        .retain(|entry| !matches!(entry.structure, structures::Structure::JoinFilter { .. }));
    p.structures.structures.declare(&[]).unwrap();
    assert_eq!(
        contract::check(&p).unwrap().publications["input"],
        Publication::DeferredRows {
            consumer: "selected".into()
        },
        "the same total join is eligible without a shadowed scan-filter binding"
    );
}

fn lookup_program(body: proc_macro2::TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = let canon probe first [a] l,r;
            return j as answer(a,b);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; provider = Provider { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            l host scan = @reader as Reader [];
            r host scan = @reader as Reader [];
            r = @provider as Provider [a];
            j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
        }
        sequence { #body }
    })
    .unwrap()
}

#[test]
fn rewrite_rejects_equivalent_physical_join_reordering() {
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a};
            j = let canon probe first [a] l,r;
            return j as answer(a);
        }
        structures {
            providers {
                reader = Reader { reads [exact, range, membership], build inferred };
                provider = Provider { reads [exact, range, membership], build inferred };
                sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared };
            }
            l host scan = @reader as Reader [];
            r host scan = @reader as Reader [];
            r = @provider as Provider [a];
            j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
        }
        sequence { out = store canon flatten lookup r [a] rows l; }
    };
    let mut target = source.clone();
    target.structures.physical.program.definitions[0] =
        syn::parse_quote!(l = let rename L@0 {a -> a};);
    target.structures.physical.program.definitions[1] =
        syn::parse_quote!(r = pipe rename R@1 {a -> a};);
    target.structures.physical.program.definitions[2] =
        syn::parse_quote!(j = let canon probe first [a] r,l;);
    target.structures.structures.entries[2] = syn::parse_quote!(l = @provider as Provider [a];);
    target.sequence.definitions[0].term =
        syn::parse_quote!(store canon flatten lookup l [a] rows r);
    contract::check(&source).unwrap();
    contract::check(&target).unwrap();
    for (source, target) in [(&source, &target), (&target, &source)] {
        let error = check_rewrite(source, target).unwrap_err().to_string();
        assert!(error.contains("BE-PhysicalPreserved"), "{error}");
    }
}

fn reject(program: &Program, rule: &str) {
    let error = contract::check(program).unwrap_err().to_string();
    assert!(error.contains(rule), "expected {rule}, got {error}");
}

fn node(name: &str, collaborative: bool, dynamic: bool, children: Vec<LevelNode>) -> LevelNode {
    LevelNode {
        shared_bytes: None,
        name: syn::parse_str(name).unwrap(),
        host_rust: matches!(name, "host" | "pool" | "task"),
        collaborative,
        dynamic,
        width: (name == "block").then_some(256),
        children,
    }
}

fn sequential() -> Machine {
    Machine {
        gpu: None,
        layout: None,
        name: syn::parse_quote!(target),
        items: 1,
        key_partition_threshold: 65536,
        key_partition_cache_threshold: 6 * 1024 * 1024,
        root: node("host", false, false, vec![]),
    }
}

fn gpu() -> Machine {
    Machine {
        gpu: None,
        layout: None,
        name: syn::parse_quote!(target),
        items: 4,
        key_partition_threshold: 65536,
        key_partition_cache_threshold: 6 * 1024 * 1024,
        root: node(
            "host",
            false,
            false,
            vec![node(
                "device",
                false,
                true,
                vec![node(
                    "block",
                    true,
                    false,
                    vec![node("lane", false, false, vec![])],
                )],
            )],
        ),
    }
}

fn grade(program: &mut Program, grades: proc_macro2::TokenStream) {
    program.graded = Some(syn::parse2(quote!(graded target { #grades })).unwrap());
}

fn reject_grade(program: &Program, machine: &Machine, rule: &str) {
    let error = check_grading(program, machine).unwrap_err().to_string();
    assert!(error.contains(rule), "expected {rule}, got {error}");
}

fn gpu_launch_fixture(deferred: bool) -> (Program, Analysis) {
    let mut p = lookup_program(quote! {
        a = store flatten lookup r [a] rows l;
        output count;
    });
    grade(&mut p, quote!(a at device;));
    let mut analysis = check_grading(&p, &gpu()).unwrap();
    assert_eq!(analysis.dispatches.len(), 1);
    let source = gpu::Strip {
        rows: None,
        items: 3,
        nested: vec![],
        fold: None,
    };
    let mut strips = vec![source];
    if !deferred {
        strips.push(gpu::Strip {
            rows: Some(17),
            items: 2,
            nested: vec![],
            fold: None,
        });
    }
    // Supply launch facts directly: checking must not reproduce the producer's
    // chosen defaults (these positive implicit sizes deliberately differ).
    analysis.dispatches[0].gpu = Some(gpu::Execution {
        partition: gpu::Partition::Tiles { rows: 19 },
        block_threads: 256,
        publication: if deferred {
            gpu::Publication::DeferredRange {
                provider: "r".into(),
                keys: vec!["a".into()],
            }
        } else {
            gpu::Publication::Rows
        },
        offsets: if deferred {
            gpu::OffsetScan::DeviceExclusive
        } else {
            gpu::OffsetScan::None
        },
        fold: gpu::FoldCompletion::Local,
        synchronization: if deferred {
            vec![
                gpu::Synchronization::CountAndFailure,
                gpu::Synchronization::RangeOffsets,
            ]
        } else {
            vec![gpu::Synchronization::CountAndFailure]
        },
        capacity: gpu::InitialCapacity::Zero,
        reservation: gpu::Reservation::StripeBatch,
        chains: vec![gpu::ChainExecution { strips }],
        share: gpu::Share::None,
        meta: gpu::Meta::Fresh,
    });
    gpu::check(&p, &analysis).unwrap();
    (p, analysis)
}

fn reject_gpu_launch(p: &Program, analysis: &Analysis) {
    let error = gpu::check(p, analysis).unwrap_err().to_string();
    assert!(error.contains("GPU-Launch:"), "{error}");
}

#[test]
fn gpu_launch_checker_accepts_recorded_positive_implicit_sizes() {
    for deferred in [false, true] {
        let (p, mut analysis) = gpu_launch_fixture(deferred);
        for rows in [1, 37, 1024] {
            analysis.dispatches[0].gpu.as_mut().unwrap().partition = gpu::Partition::Tiles { rows };
            gpu::check(&p, &analysis).unwrap();
        }
        analysis.dispatches[0].gpu.as_mut().unwrap().partition = gpu::Partition::One;
        gpu::check(&p, &analysis).unwrap();
    }
}

#[test]
fn gpu_launch_checker_rejects_deferred_range_without_offsets() {
    let (p, mut analysis) = gpu_launch_fixture(true);
    analysis.dispatches[0].gpu.as_mut().unwrap().offsets = gpu::OffsetScan::None;
    reject_gpu_launch(&p, &analysis);
}

#[test]
fn gpu_launch_checker_rejects_row_publication_with_offsets() {
    let (p, mut analysis) = gpu_launch_fixture(false);
    analysis.dispatches[0].gpu.as_mut().unwrap().offsets = gpu::OffsetScan::DeviceExclusive;
    reject_gpu_launch(&p, &analysis);
}

#[test]
fn gpu_launch_checker_rejects_missing_count_synchronization() {
    for deferred in [false, true] {
        let (p, mut analysis) = gpu_launch_fixture(deferred);
        analysis.dispatches[0]
            .gpu
            .as_mut()
            .unwrap()
            .synchronization
            .remove(0);
        reject_gpu_launch(&p, &analysis);
    }
}

#[test]
fn gpu_launch_checker_rejects_missing_range_synchronization() {
    let (p, mut analysis) = gpu_launch_fixture(true);
    analysis.dispatches[0]
        .gpu
        .as_mut()
        .unwrap()
        .synchronization
        .pop();
    reject_gpu_launch(&p, &analysis);
}

#[test]
fn gpu_launch_checker_rejects_row_range_synchronization() {
    let (p, mut analysis) = gpu_launch_fixture(false);
    analysis.dispatches[0]
        .gpu
        .as_mut()
        .unwrap()
        .synchronization
        .push(gpu::Synchronization::RangeOffsets);
    reject_gpu_launch(&p, &analysis);
}

#[test]
fn gpu_launch_checker_rejects_reordered_synchronization() {
    let (p, mut analysis) = gpu_launch_fixture(true);
    analysis.dispatches[0]
        .gpu
        .as_mut()
        .unwrap()
        .synchronization
        .reverse();
    reject_gpu_launch(&p, &analysis);
}

#[test]
fn gpu_launch_checker_rejects_duplicate_synchronization() {
    for deferred in [false, true] {
        let (p, mut analysis) = gpu_launch_fixture(deferred);
        let points = &mut analysis.dispatches[0].gpu.as_mut().unwrap().synchronization;
        points.push(points.last().unwrap().clone());
        reject_gpu_launch(&p, &analysis);
    }
}

#[test]
fn gpu_launch_checker_rejects_zero_partition_rows() {
    let (p, mut analysis) = gpu_launch_fixture(false);
    analysis.dispatches[0].gpu.as_mut().unwrap().partition = gpu::Partition::Tiles { rows: 0 };
    reject_gpu_launch(&p, &analysis);
}

#[test]
fn gpu_launch_checker_accepts_any_legal_spread_share_and_meta() {
    let (mut p, mut analysis) = gpu_launch_fixture(false);
    p.sequence.roots = Roots::Spread;
    let execution = analysis.dispatches[0].gpu.as_mut().unwrap();
    // Any positive bound and tile cap is sound; neither is the producer's default.
    execution.partition = gpu::Partition::Spread { max_rows: 5 };
    execution.share = gpu::Share::FirstExpansion { tiles: 7 };
    gpu::check(&p, &analysis).unwrap();
    // Sharing and resident words are optional.
    analysis.dispatches[0].gpu.as_mut().unwrap().share = gpu::Share::None;
    gpu::check(&p, &analysis).unwrap();
}

#[test]
fn gpu_launch_checker_rejects_illegal_spread_share_and_meta() {
    let (mut p, analysis) = gpu_launch_fixture(false);
    let mutated = |p: &Program, mutate: &dyn Fn(&mut gpu::Execution)| {
        let mut analysis = analysis.clone();
        mutate(analysis.dispatches[0].gpu.as_mut().unwrap());
        gpu::check(p, &analysis).is_err()
    };
    // Without `roots spread` a dispatch neither spreads nor shares.
    assert!(mutated(&p, &|execution| {
        execution.partition = gpu::Partition::Spread { max_rows: 5 }
    }));
    assert!(mutated(&p, &|execution| {
        execution.share = gpu::Share::FirstExpansion { tiles: 7 }
    }));
    // Resident words need the arena terminal protocol.
    assert!(mutated(&p, &|execution| execution.meta = gpu::Meta::Resident));
    p.sequence.roots = Roots::Spread;
    // Under `roots spread` a fold-free dispatch spreads with a positive bound.
    assert!(mutated(&p, &|_| {}));
    assert!(mutated(&p, &|execution| {
        execution.partition = gpu::Partition::Spread { max_rows: 0 }
    }));
    assert!(mutated(&p, &|execution| {
        execution.partition = gpu::Partition::Spread { max_rows: 5 };
        execution.share = gpu::Share::FirstExpansion { tiles: 0 };
    }));
}

#[test]
fn every_grammar_form_round_trips() {
    let mut p = program(quote! {
        a = rows r;
        b = map r a;
        c = flatten tile 8 items 4 b;
        d = compact selected c;
        e = lookup provider [a,b] d;
        f = fold count a;
        g = fold or a;
        h = fold sum a a;
        grouped = group aggregate_result a;
        i = canon store a;
        j = choose [len a + 2 + len b < 7 => tile 2 a, len a + 2 + len b >= 7 => tile 4 a, else => tile 8 a];
        k = store capacity 42 flatten e;
        output bounded;
    });
    grade(&mut p, quote! { a at host; b at device.block.lane; });
    assert_eq!(syn::parse2::<Program>(p.to_token_stream()).unwrap(), p);
    for body in [
        quote!(a = store rows r; output count;),
        quote!(a = store rows r; output reserve;),
        quote!(),
    ] {
        let p = program(body);
        assert_eq!(syn::parse2::<Program>(p.to_token_stream()).unwrap(), p);
    }
    for term in [
        quote!(lookup r [] rows l),
        quote!(choose [else => rows r]),
        quote!(tile 2 rows r),
        quote!(partition_k[a,b](4) rows r),
        quote!(partition_k[](4) rows r),
    ] {
        let term: Term = syn::parse2(term).unwrap();
        assert_eq!(syn::parse2::<Term>(term.to_token_stream()).unwrap(), term);
    }
}

#[test]
fn constructor_names_remain_definition_names_and_references() {
    for spelling in [
        "rows",
        "map",
        "flatten",
        "compact",
        "tile",
        "lookup",
        "fold",
        "group",
        "canon",
        "store",
        "union",
        "choose",
        "capacity",
        "output",
        "partition_k",
    ] {
        let name: syn::Ident = syn::parse_str(spelling).unwrap();
        let sequence: Sequence = syn::parse2(quote! {
            sequence {
                #name = rows r;
                direct = #name;
                nested = store #name;
                arms = union [#name, canon #name];
                choices = choose [len r < 1 => #name, else => store #name];
                output count;
            }
        })
        .unwrap();
        let reference = Term::Ref(name.clone());
        assert_eq!(sequence.definitions[0].name, name);
        assert_eq!(sequence.definitions[1].term, reference);
        assert_eq!(
            sequence.definitions[2].term,
            Term::Store {
                capacity: None,
                inner: Box::new(reference.clone()),
            }
        );
        assert_eq!(
            sequence.definitions[3].term,
            Term::Union(vec![
                reference.clone(),
                Term::Canon(Box::new(reference.clone())),
            ])
        );
        let Term::Choose(alternatives) = &sequence.definitions[4].term else {
            panic!("expected choose constructor");
        };
        assert_eq!(alternatives[0].1, reference);
        assert_eq!(alternatives[1].1, sequence.definitions[2].term);
        assert_eq!(syn::parse2::<Term>(quote!(#name)).unwrap(), reference);
        assert_eq!(
            syn::parse2::<Sequence>(sequence.to_token_stream()).unwrap(),
            sequence
        );
    }
}

#[test]
fn physical_rows_name_is_preserved_in_sequence_references() {
    let p: Program = syn::parse2(quote! {
        physical {
            relation R(a:i32);
            rows = let canon rename R@0 {a -> a};
            return rows as answer(a);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            rows host scan = @reader as Reader [];
            rows rows = @sort as ::cq_storage::dedup::Sort; rows canon = rows;
        }
        sequence {
            source = rows rows;
            result = store rows;
        }
    })
    .unwrap();
    assert_eq!(
        p.sequence.definitions[0].term,
        Term::Rows(syn::parse_quote!(rows))
    );
    assert_eq!(
        p.sequence.definitions[1].term,
        Term::Store {
            capacity: None,
            inner: Box::new(Term::Ref(syn::parse_quote!(rows))),
        }
    );
    assert_eq!(syn::parse2::<Program>(p.to_token_stream()).unwrap(), p);
}

#[test]
fn lengths_views_stores_and_output_are_derived() {
    let p = program(quote! { a = rows r; b = tile 8 a; c = flatten b; d = store canon c; });
    let a = contract::check(&p).unwrap();
    assert_eq!(a.lengths["a"], Len::Input("r".into()));
    assert_eq!(a.lengths["b"], a.lengths["a"]);
    assert_eq!(a.lengths["c"], Len::Scan("c".into()));
    assert_eq!(a.stores["d"], Len::Scan("d".into()));
    assert_eq!(a.views.iter().cloned().collect::<Vec<_>>(), ["a", "b", "c"]);
    assert_eq!(a.output, Some(("d".into(), OutputProtocol::Count)));
    assert!(a.levels.is_empty());
    // Contract checking records no canon level; grading records the graded one.
    assert!(a.obligations.iter().any(|o| matches!(
        o,
        Obligation::Canon { result, level: None, .. } if result == "d"
    )));
    let graded = check_grading(&p, &sequential()).unwrap();
    assert!(graded.obligations.iter().any(|o| matches!(
        o,
        Obligation::Canon { result, level: Some(level), .. }
            if result == "d" && *level == graded.levels["d"]
    )));
    assert!(
        a.obligations
            .iter()
            .any(|o| matches!(o, Obligation::Codec { .. }))
    );
}

#[test]
fn alpha_rejects_unknown_results() {
    reject(&program(quote!(a = rows missing;)), "BE-Alpha");
}

#[test]
fn alpha_rejects_nonselect_compact_and_wrong_map_input() {
    reject(&program(quote!(a = compact r rows r;)), "BE-Alpha");
    reject(&lookup_program(quote!(a = map l rows r;)), "BE-Alpha");
}

#[test]
fn bind_rejects_provider_without_requested_key_capability() {
    reject(
        &lookup_program(quote!(a = flatten lookup l [a] rows l;)),
        "BE-Bind",
    );
}

#[test]
fn bind_records_range_obligations_without_inspecting_type_names() {
    let mut p = lookup_program(quote!(a = flatten lookup r [a] tile 8 rows l;));
    grade(&mut p, quote!(a at device.block;));
    let a = check_grading(&p, &gpu()).unwrap();
    assert_eq!(
        a.capabilities[&("r".into(), vec!["a".into()])],
        Capability::Range
    );
    assert!(a.obligations.iter().any(|o| matches!(o, Obligation::Provider { capability: Capability::Range, instance, .. } if instance.token == "provider")));
}

fn native_lookup_program(body: proc_macro2::TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation L(a:i32);
            relation R(a:i32,b:i32) host [a] reads [exact];
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,b -> b};
            j = let canon probe first [a] l,r;
            return j as answer(a,b);
        }
        structures {
            providers {
                reader = Reader { reads [exact, range, membership], build inferred };
                sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared };
            }
            l host scan = @reader as Reader [];
            r host scan = @reader as Reader [];
            r native result(j) = exact [a];
            j rows = @sort as ::cq_storage::dedup::Sort;
            j canon = rows;
        }
        sequence { #body }
    })
    .unwrap()
}

#[test]
fn native_lookup_is_checked_again_after_range_grading() {
    let mut p = native_lookup_program(quote!(a = flatten lookup r [a] tile 8 rows l;));
    let analysis = contract::check(&p).unwrap();
    assert_eq!(
        analysis.capabilities[&("r".into(), vec!["a".into()])],
        Capability::Exact
    );
    check_grading(&p, &sequential()).unwrap();
    grade(&mut p, quote!(a at device.block;));
    assert!(check_grading(&p, &gpu()).is_err());
}

#[test]
fn native_lookup_and_map_use_their_own_reader_record() {
    for body in [
        quote!(a = flatten lookup r [a] rows l;),
        quote!(a = map j rows l;),
    ] {
        let p = native_lookup_program(body);
        let analysis = contract::check(&p).unwrap();
        assert_eq!(
            analysis.capabilities[&("r".into(), vec!["a".into()])],
            Capability::Exact
        );
        assert!(!analysis.obligations.iter().any(|obligation| {
            matches!(obligation, Obligation::Provider { provider, .. } if provider == "r")
        }));
    }
}

#[test]
fn mixed_native_exact_and_merge_fallback_keep_reader_requirements_separate() {
    for probe in [
        quote!(),
        quote!(exact = map j rows l;),
        quote!(exact = flatten lookup r [a] rows l;),
    ] {
        let p: Program = syn::parse2(quote! {
            physical {
                relation L(a:i32);
                relation S(a:i32);
                relation R(a:i32,b:i32) host [a] reads [exact];
                l = pipe rename L@0 {a -> a};
                s = pipe rename S@1 {a -> a};
                r = pipe rename R@2 {a -> a,b -> b};
                j = pipe probe first [a] l,r;
                m = let canon merge[a] s,r;
                answer = let canon union j,m;
                return answer as answer(a,b);
            }
            structures {
                providers {
                    reader = Reader { reads [exact, range, membership], build inferred };
                    ordered = Ordered { reads [exact, range, membership, ordered], build inferred };
                    sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared };
                }
                l host scan = @reader as Reader [];
                s host scan = @reader as Reader [];
                r host scan = @reader as Reader [];
                r native result(j) = exact [a];
                s host result(m) = @ordered as Ordered [a];
                r host result(m) = @ordered as Ordered [a];
                m rows = @sort as ::cq_storage::dedup::Sort;
                m canon = rows;
                answer rows = @sort as ::cq_storage::dedup::Sort;
                answer canon = rows;
            }
            sequence {
                #probe
                out = flatten lookup r [a] rows s;
            }
        })
        .unwrap();
        let analysis = check_grading(&p, &sequential()).unwrap();
        let key = |reader: &str| {
            (
                "r".into(),
                cq_lang_structures::ReaderRef::Result(reader.into()),
                vec!["a".into()],
            )
        };
        assert_eq!(analysis.lookup_capabilities[&key("m")], Capability::Ordered);
        if probe.is_empty() {
            assert!(!analysis.lookup_capabilities.contains_key(&key("j")));
        } else {
            assert_eq!(analysis.lookup_capabilities[&key("j")], Capability::Exact);
        }
    }
}

#[test]
fn tiled_host_sources_cannot_discard_native_scan_records() {
    for (body, tiled) in [
        (quote!(a = rows r;), false),
        (quote!(a = tile 8 rows r;), true),
        (
            quote!(source = store rows r; a = partition_k[b](4) source;),
            true,
        ),
    ] {
        let p: Program = syn::parse2(quote! {
            physical {
                relation R(a:i32,b:i32) host [a] reads [exact];
                r = let rename R@0 {b -> b} constants {a = 7};
                out = let canon project r keep {b};
                return out as answer(b);
            }
            structures {
                providers {
                    sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared };
                    rows = ::std::vec::Vec<_> { reads [], build typed, accounting rows };
                    scan = ::core::iter::Map<_, _> { reads [], build typed };
                }
                r native scan = exact [a];
                r rows = @rows as ::std::vec::Vec<_>;
                r host scan = @scan as ::core::iter::Map<_, _> [];
                out rows = @sort as ::cq_storage::dedup::Sort;
                out canon = rows;
            }
            sequence { #body }
        })
        .unwrap();
        structures::contract::check(&p.structures).unwrap();
        if tiled {
            assert!(contract::check(&p).is_err());
        } else {
            contract::check(&p).unwrap();
        }
    }
}

#[test]
fn partition_rejects_zero_tile_and_invalid_sum() {
    reject(&program(quote!(a = tile 0 rows r;)), "BE-Partition");
    reject(&program(quote!(a = tile 8 items 0 rows r;)), "BE-Partition");
    reject(
        &program(quote!(a = fold sum missing rows r;)),
        "BE-Partition",
    );
}

#[test]
fn offsets_rejects_flat_input() {
    reject(&program(quote!(a = flatten rows r;)), "BE-Offsets");
}

#[test]
fn canon_preserves_outer_tile_partitions() {
    let analysis = contract::check(&program(quote!(a = canon tile 8 rows r;))).unwrap();
    assert_eq!(analysis.shapes["a"].len(), 2);
}

#[test]
fn unknown_host_store_grows_but_separated_store_needs_capacity() {
    let mut p = lookup_program(quote!(a = store lookup r [a] rows l;));
    check_grading(&p, &sequential()).unwrap();
    grade(&mut p, quote!(a at device;));
    reject_grade(&p, &gpu(), "BE-Length");
    let p = lookup_program(quote!(a = store capacity 100 lookup r [a] rows l; output bounded;));
    let a = contract::check(&p).unwrap();
    assert_eq!(a.stores["a"], Len::Unknown(Some(100)));
    assert_eq!(a.output, Some(("a".into(), OutputProtocol::Bounded)));
}

#[test]
fn output_requires_final_store_and_bounded_capacity() {
    reject(&program(quote!(a = rows r; output reserve;)), "BE-Length");
    reject(
        &program(quote!(a = store rows r; output bounded;)),
        "BE-Length",
    );
    for protocol in [
        quote!(count),
        quote!(reserve),
        quote!(reserve_previous),
        quote!(bounded),
    ] {
        contract::check(&program(
            quote!(a = store capacity 8 rows r; output #protocol;),
        ))
        .unwrap();
    }
}

#[test]
fn reserve_previous_round_trips_without_claiming_capacity() {
    let p = program(quote!(a = store rows r; output reserve_previous;));
    assert_eq!(p.sequence.output, Some(OutputProtocol::ReservePrevious));
    assert_eq!(
        OutputProtocol::ReservePrevious
            .to_token_stream()
            .to_string(),
        "reserve_previous"
    );
    assert_eq!(syn::parse2::<Program>(p.to_token_stream()).unwrap(), p);
    let analysis = contract::check(&p).unwrap();
    assert_eq!(
        analysis.output,
        Some(("a".into(), OutputProtocol::ReservePrevious))
    );
    reject(
        &program(quote!(a = rows r; output reserve_previous;)),
        "BE-Length",
    );
    let default = program(quote!(a = store rows r;));
    assert_eq!(default.sequence.output, None);
    assert_eq!(analysis.stores, contract::check(&default).unwrap().stores);
    assert_eq!(
        contract::check(&default).unwrap().output,
        Some(("a".into(), OutputProtocol::Count))
    );
}

#[test]
fn roots_spread_round_trips_before_output() {
    let p = program(quote!(a = store rows r; roots spread; output reserve_previous;));
    assert_eq!(p.sequence.roots, Roots::Spread);
    assert_eq!(syn::parse2::<Program>(p.to_token_stream()).unwrap(), p);
    contract::check(&p).unwrap();
    assert_eq!(
        program(quote!(a = store rows r;)).sequence.roots,
        Roots::Declared
    );
    // The statement follows every definition and precedes output.
    for misplaced in [
        quote!(sequence { roots spread; a = store rows r; }),
        quote!(sequence { a = store rows r; output count; roots spread; }),
    ] {
        assert!(syn::parse2::<Sequence>(misplaced).is_err());
    }
}

#[test]
fn choose_rejects_nonexhaustive_and_unequal_alternatives() {
    reject(
        &program(quote!(a = choose [len r < 3 => rows r];)),
        "BE-Choose",
    );
    reject(
        &program(quote!(a = choose [len r < 3 => rows r, else => fold count rows r];)),
        "BE-Choose",
    );
    reject(
        &program(quote!(a = choose [len missing < 3 => rows r, else => rows r];)),
        "BE-Choose",
    );
    contract::check(&program(
        quote!(a = choose [len r < 3 => tile 2 rows r, len r >= 3 => tile 4 rows r];),
    ))
    .unwrap();
}

#[test]
fn worker_guards_roundtrip_and_preserve_support_without_claiming_size() {
    let source = program(quote!(a = rows r;));
    let target = program(quote!(a = choose [
        workers < 2 => tile 1 rows r,
        len r < 3 => tile 2 rows r,
        else => tile 4 rows r
    ];));
    assert_eq!(target, syn::parse2(target.to_token_stream()).unwrap());
    check_rewrite(&source, &target).unwrap();
    let facts = contract::check(&target).unwrap();
    assert_eq!(
        facts.lengths["a"],
        contract::check(&source).unwrap().lengths["a"]
    );
    for body in [
        quote!(a = choose [workers < 0 => rows r, else => rows r];),
        quote!(a = choose [workers < 2 => rows r];),
        quote!(a = choose [workers < 2 => rows r, else => fold count rows r];),
        quote!(a = choose [else => rows r, workers < 2 => rows r];),
    ] {
        reject(&program(body), "BE-Choose");
    }
}

#[test]
fn choose_unknown_lengths_need_capacity_only_when_stored() {
    let body = quote!(choose [len r < 3 => rows r, else => canon rows r]);
    let p = program(quote!(a = #body;));
    assert_eq!(
        contract::check(&p).unwrap().lengths["a"],
        Len::Unknown(None)
    );
    let mut stored = program(quote!(a = store #body;));
    check_grading(&stored, &sequential()).unwrap();
    grade(&mut stored, quote!(a at device;));
    reject_grade(&stored, &gpu(), "BE-Length");
    contract::check(&program(quote!(a = store capacity 42 #body;))).unwrap();
}

#[test]
fn separated_store_uses_checked_lengths_through_maps_and_choices() {
    let mut unknown = program(quote! {
        a = store map r choose [len r < 3 => rows r, else => canon rows r];
    });
    grade(&mut unknown, quote!(a at device;));
    reject_grade(&unknown, &gpu(), "BE-Length");

    let mut equal = program(quote! {
        a = store choose [len r < 3 => rows r, else => map r rows r];
    });
    grade(&mut equal, quote!(a at device;));
    check_grading(&equal, &gpu()).unwrap();
}

#[test]
fn rewrites_preserve_support_and_structures() {
    let source = program(quote!(a = store rows r;));
    let target = program(quote!(a = store flatten tile 8 items 2 rows r; output reserve;));
    check_rewrite(&source, &target).unwrap();
    assert!(check_rewrite(&source, &program(quote!(a = store fold count rows r;))).is_err());
    let mut changed = target.clone();
    changed.structures.structures.entries[0] =
        syn::parse_quote!(r host scan = @other as Other [] ;);
    changed
        .structures
        .structures
        .declare(&[syn::parse_quote!(
            other = Other { reads [exact, range, membership], build inferred };
        )])
        .unwrap();
    assert!(check_rewrite(&source, &changed).is_err());
}

#[test]
fn references_must_be_earlier_and_unique() {
    reject(&program(quote!(a = b; b = rows r;)), "BE-Alpha");
    reject(&program(quote!(a = rows r; a = rows r;)), "BE-Alpha");
}

#[test]
fn language_contract_is_available() {
    let p = program(quote!(a = store rows r;));
    <Program as cq_stage::Language>::check(&p).unwrap();
    <Program as cq_stage::Language>::check_rewrite(&p, &p).unwrap();
}

#[test]
fn scope_rejects_missing_and_unknown_grades() {
    let mut p = program(quote!(a = rows r;));
    grade(&mut p, quote!());
    reject_grade(&p, &gpu(), "BE-Scope");
    grade(&mut p, quote!(a at absent;));
    reject_grade(&p, &gpu(), "BE-Scope");
}

#[test]
fn boundary_derives_dispatch_and_failure_path() {
    let mut p = program(quote!(a = store rows r; b = map r a; c = store b;));
    grade(&mut p, quote!(a at host; b at device; c at device;));
    let a = check_grading(&p, &gpu()).unwrap();
    assert_eq!(a.dispatches.len(), 1);
    let d = &a.dispatches[0];
    assert_eq!(d.outer_store, "a");
    assert!(d.members.contains(&"b".into()));
    assert!(d.inner_stores.contains(&"c".into()));
    assert_eq!(d.failure_channel.last().map(String::as_str), Some("host"));
    assert!(
        a.obligations
            .iter()
            .any(|o| matches!(o, Obligation::Failure { dispatch: 0, .. }))
    );
    reject_grade(&p, &sequential(), "BE-Scope");
}

#[test]
fn collectives_obey_machine_labels_not_machine_names() {
    let mut p = program(quote!(a = store rows r; b = fold count a;));
    grade(&mut p, quote!(a at host; b at host;));
    check_grading(&p, &sequential()).unwrap();
    check_grading(&p, &gpu()).unwrap();
    grade(&mut p, quote!(a at host; b at device;));
    reject_grade(&p, &gpu(), "BE-Collective");
    let mut collaborative = gpu();
    collaborative.root.children[0].collaborative = true;
    check_grading(&p, &collaborative).unwrap();
}

#[test]
fn worker_guards_are_uniform_for_collective_participation() {
    let mut p = program(quote!(
        a = store rows r;
        b = choose [workers < 2 => fold count a, else => fold count a];
    ));
    grade(&mut p, quote!(a at host; b at device;));
    let mut collaborative = gpu();
    collaborative.root.children[0].collaborative = true;
    let facts = check_grading(&p, &collaborative).unwrap();
    assert_eq!(facts.lengths["b"], Len::Lit(1));
    assert_eq!(facts.collectives.len(), 2);
}

#[test]
fn abort_rejects_machine_without_host_failure_root() {
    let mut p = program(quote!(a = rows r;));
    grade(&mut p, quote!(a at host;));
    let mut m = gpu();
    m.root.name = syn::parse_quote!(device);
    reject_grade(&p, &m, "BE-Abort");
}

#[test]
fn scope_rejects_unstored_ancestor_views() {
    let mut p = program(quote!(a = rows r; b = a;));
    grade(&mut p, quote!(a at host; b at device;));
    reject_grade(&p, &gpu(), "BE-Scope");
}

#[test]
fn separate_rejects_direct_sibling_exchange() {
    let mut p = program(quote!(a = store rows r; b = a;));
    grade(&mut p, quote!(a at left; b at right;));
    let mut m = sequential();
    m.root.children = vec![
        node("left", false, true, vec![]),
        node("right", false, true, vec![]),
    ];
    reject_grade(&p, &m, "BE-Separate");
}

#[test]
fn separated_publication_keeps_collaborative_store_in_its_dispatch() {
    let mut p = program(quote!(a = store store rows r;));
    grade(&mut p, quote!(a at device;));
    let analysis = check_grading(&p, &gpu()).unwrap();
    assert_eq!(analysis.dispatches.len(), 1);
    assert_eq!(
        analysis.dispatches[0].producer_level,
        ["host", "device", "block"]
    );
}

#[test]
fn rust_without_identity_representation_stays_host() {
    let mut p: Program = syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe rename R@0 {a -> a};
            b = let canon bind r with n:i32 = (1 / *a);
            return b as answer(a,n);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            r host scan = @reader as Reader [];
            b canon = rows;
            b rows = @sort as ::cq_storage::dedup::Sort;
        } sequence { a = store map b rows r; }
    };
    grade(&mut p, quote!(a at device;));
    reject_grade(&p, &gpu(), "BE-RustRepresentation");
    p.representation.encoded.insert("a".into());
    reject_grade(&p, &gpu(), "BE-RustRepresentation");
    grade(&mut p, quote!(a at host;));
    check_grading(&p, &gpu()).unwrap();
}

#[test]
fn arbitrary_rust_on_identity_columns_earns_device_placement() {
    let mut p: Program = syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe rename R@0 {a -> a};
            b = pipe bind r with n:i32 = (row_function(a));
            g = let canon guard b if (*n / -1 != 0);
            return g as answer(a,n);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            r host scan = @reader as Reader [];
            g canon = rows;
            g rows = @sort as ::cq_storage::dedup::Sort;
        } sequence { a = store map g rows r; }
        representation { identity [a,n]; }
    };
    grade(&mut p, quote!(a at device;));
    check_grading(&p, &gpu()).unwrap();
}

#[test]
fn representation_facts_round_trip_and_reject_unknown_or_conflicting_columns() {
    use quote::ToTokens;
    let mut p = program(quote!(out = store rows r;));
    p.representation.identity.insert("a".into());
    contract::check(&p).unwrap();
    let printed = p.to_token_stream();
    assert_eq!(syn::parse2::<Program>(printed).unwrap(), p);
    let mut conflict = p.clone();
    conflict.representation.encoded.insert("a".into());
    assert!(contract::check(&conflict).is_err());
    let mut unknown = p.clone();
    unknown.representation.identity.insert("missing".into());
    assert!(contract::check(&unknown).is_err());
    let mut changed = p.clone();
    changed.representation.identity.clear();
    assert!(check_rewrite(&p, &changed).is_err());
}

#[test]
fn rewrites_refine_representation_facts_monotonically() {
    let unknown = program(quote!(out = store rows r;));
    let mut identity = unknown.clone();
    identity.representation.identity.insert("a".into());
    let mut encoded = unknown.clone();
    encoded.representation.encoded.insert("a".into());

    for refined in [&identity, &encoded] {
        check_rewrite(&unknown, refined).unwrap();
        check_rewrite(refined, refined).unwrap();
        let error = check_rewrite(refined, &unknown).unwrap_err();
        assert!(error.to_string().contains("BE-RustRepresentation"));
    }
    for (source, target) in [(&identity, &encoded), (&encoded, &identity)] {
        let error = check_rewrite(source, target).unwrap_err();
        assert!(error.to_string().contains("BE-RustRepresentation"));
    }

    let mut conflict = identity.clone();
    conflict.representation.encoded.insert("a".into());
    assert!(check_rewrite(&identity, &conflict).is_err());
    for encoded_fact in [false, true] {
        let mut missing = unknown.clone();
        if encoded_fact {
            missing.representation.encoded.insert("missing".into());
        } else {
            missing.representation.identity.insert("missing".into());
        }
        assert!(check_rewrite(&unknown, &missing).is_err());
    }
}

#[test]
fn bind_rejects_membership_as_range_cursor() {
    let p: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,tag:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,tag -> tag};
            selected = let point select r if (tag = 7);
            j = let canon probe first [a] l,selected;
            return j as answer(a,tag);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; provider = Provider { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            l host scan = @reader as Reader [];
            r host scan = @reader as Reader [];
            r host result(selected) = @reader as Reader [tag];
            selected = @provider as Provider [a] membership payload [a] constants {tag = 7};
            j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
        } sequence {
            s = store map selected rows r;
            a = flatten lookup selected [a] rows l;
        }
    };
    let mut missing = p.clone();
    missing.sequence.definitions.remove(0);
    reject(&missing, "BE-Scope");
    missing.sequence.definitions[0].term = syn::parse_quote!(rows selected);
    reject(&missing, "BE-Scope");
    reject(&p, "BE-Bind");
}

#[test]
fn choose_can_change_partition_shape_without_changing_support() {
    let p = program(quote!(a = store choose [len r < 8 => rows r, else => tile 8 rows r];));
    contract::check(&p).unwrap();
    check_rewrite(&program(quote!(a = store rows r;)), &p).unwrap();
    reject(
        &program(quote!(a = flatten choose [len r < 8 => rows r, else => tile 8 rows r];)),
        "BE-Offsets",
    );
}

#[test]
fn raw_identifiers_in_length_guards_round_trip_and_check() {
    let p =
        program(quote!(r#type = rows r; a = choose [len r#type < 3 => r#type, else => r#type];));
    contract::check(&p).unwrap();
    assert_eq!(syn::parse2::<Program>(p.to_token_stream()).unwrap(), p);
}

#[test]
fn compact_and_total_map_derive_the_physical_result() {
    let p: Program = syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe rename R@0 {a -> a};
            s = let canon select r if (a >= 0);
            return s as answer(a);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            r host scan = @reader as Reader [];
            s rows = @sort as ::cq_storage::dedup::Sort; s canon = rows;
        } sequence { a = store compact s rows r; }
    };
    assert_eq!(
        contract::check(&p).unwrap().lengths["a"],
        Len::Scan("a".into())
    );
    let mut mapped = p.clone();
    mapped.sequence.definitions[0].term = syn::parse_quote!(store map s rows r);
    check_rewrite(&p, &mapped).unwrap();
}

#[test]
fn ungraded_programs_default_to_host_only() {
    let p = program(quote!(a = store rows r;));
    let analysis = check_grading(&p, &sequential()).unwrap();
    assert_eq!(analysis.levels["a"], ["host"]);
    reject_grade(&p, &gpu(), "BE-Scope");
}

#[test]
fn leaf_flatten_is_sequential_but_parent_fold_needs_collaboration() {
    let tasks = Machine {
        gpu: None,
        layout: None,
        name: syn::parse_quote!(target),
        items: 1,
        key_partition_threshold: 65536,
        key_partition_cache_threshold: 6 * 1024 * 1024,
        root: node(
            "host",
            false,
            false,
            vec![node(
                "pool",
                false,
                false,
                vec![node("task", false, false, vec![])],
            )],
        ),
    };
    let mut p = lookup_program(quote!(a = flatten lookup r [a] rows l;));
    grade(&mut p, quote!(a at pool.task;));
    assert!(check_grading(&p, &tasks).unwrap().collectives.is_empty());
    let mut p = program(quote!(a = fold count rows r;));
    grade(&mut p, quote!(a at pool;));
    reject_grade(&p, &tasks, "BE-Collective");
    grade(&mut p, quote!(a at device.block;));
    check_grading(&p, &gpu()).unwrap();
}

#[test]
fn host_retained_probe_stores_rows_without_requiring_range_offsets() {
    let p: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = let canon probe first [a] l,r;
            return j as answer(a,b);
        } structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; provider = Provider { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            l host scan = @reader as Reader [];
            r host scan = @reader as Reader [];
            r = @provider as Provider [a];
            j rows = @sort as ::cq_storage::dedup::Sort;
            j canon = rows;
        } sequence {
            input = rows l;
            joined = store flatten lookup r [a] input;
            answer = store joined;
            output count;
        }
    };
    let key = ("r".into(), vec!["a".into()]);
    let analysis = check_grading(&p, &sequential()).unwrap();
    assert_eq!(analysis.capabilities[&key], Capability::Exact);
    assert!(analysis.obligations.iter().any(|obligation| matches!(
        obligation,
        Obligation::Provider { provider, capability: Capability::Exact, .. }
            if provider == "r"
    )));
    assert!(analysis.obligations.iter().all(|obligation| !matches!(
        obligation,
        Obligation::Provider {
            capability: Capability::Range,
            ..
        }
    )));
    // Placement-independent structural inference agrees with host grading.
    assert_eq!(
        contract::check(&p).unwrap().capabilities[&key],
        Capability::Exact
    );
    let tiled = lookup_program(quote!(a = store tile 8 flatten lookup r [a] rows l;));
    assert_eq!(
        check_grading(&tiled, &sequential()).unwrap().capabilities[&key],
        Capability::Exact
    );
}

#[test]
fn row_stores_need_exact_and_collective_or_distributed_reads_need_range() {
    let streaming = lookup_program(quote!(a = flatten lookup r [a] rows l;));
    let key = ("r".into(), vec!["a".into()]);
    assert_eq!(
        contract::check(&streaming).unwrap().capabilities[&key],
        Capability::Exact
    );
    let stored = lookup_program(quote!(a = store flatten lookup r [a] rows l;));
    assert_eq!(
        contract::check(&stored).unwrap().capabilities[&key],
        Capability::Exact
    );
    let tiled = lookup_program(quote!(a = tile 8 flatten lookup r [a] rows l;));
    assert_eq!(
        contract::check(&tiled).unwrap().capabilities[&key],
        Capability::Exact
    );
    let segmented = lookup_program(quote!(a = flatten lookup r [a] tile 8 rows l;));
    assert_eq!(
        contract::check(&segmented).unwrap().capabilities[&key],
        Capability::Exact
    );
    let mut distributed = lookup_program(quote!(a = store tile 8 flatten lookup r [a] rows l;));
    grade(&mut distributed, quote!(a at device;));
    assert_eq!(
        check_grading(&distributed, &gpu()).unwrap().capabilities[&key],
        Capability::Range
    );
    let mut collective = streaming;
    grade(&mut collective, quote!(a at device.block;));
    assert_eq!(
        check_grading(&collective, &gpu()).unwrap().capabilities[&key],
        Capability::Range
    );
}

#[test]
fn membership_compact_records_polarity_and_never_enumerates() {
    for operation in ["semi", "anti"] {
        let text = format!(
            "physical {{
            relation L(a:i32); relation R(a:i32);
            l = pipe rename L@0 {{a -> a}};
            r = let rename R@1 {{a -> a}};
            j = let canon {operation} eager [a] l,r;
            return j as answer(a);
        }} structures {{ providers {{
            reader = Reader {{ reads [exact, range, membership], build inferred }};
            provider = Provider {{ reads [exact, range, membership], build inferred }};
            sort = ::cq_storage::dedup::Sort {{ reads [distinct], build inferred, shared }};
        }}
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            r = @provider as Provider [a]; j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
        }} sequence {{ a = compact j lookup r [a] rows l; }}"
        );
        let p: Program = syn::parse_str(&text).unwrap();
        let analysis = contract::check(&p).unwrap();
        assert_eq!(
            analysis.capabilities[&("r".into(), vec!["a".into()])],
            Capability::Membership
        );
        assert_eq!(analysis.results["a"], "j");
        let mut changed = p.clone();
        changed.sequence.definitions[0].term = syn::parse_quote!(compact j lookup r [] rows l);
        reject(&changed, "BE-Alpha");
    }
}

#[test]
fn empty_key_membership_reads_the_whole_relation_without_an_index_obligation() {
    for operation in ["semi", "anti"] {
        let program: Program = syn::parse_str(&format!(
            "physical {{
                relation L(a:i32); relation R(b:i32);
                l = pipe rename L@0 {{a -> a}};
                r = pipe rename R@1 {{b -> b}};
                scan = let canon {operation} eager [] l,r;
                return scan as answer(a);
            }} structures {{ providers {{
                scan_adapter = ScanAdapter {{ reads [], build inferred }};
                sort = ::cq_storage::dedup::Sort {{ reads [distinct], build inferred, shared }};
            }}
                l host scan = @scan_adapter as ScanAdapter [];
                r host scan = @scan_adapter as ScanAdapter [];
                scan rows = @sort as ::cq_storage::dedup::Sort; scan canon = rows;
            }} sequence {{
                answer = compact scan lookup r [] rows l;
            }}"
        ))
        .unwrap();
        let analysis = contract::check(&program).unwrap();
        assert_eq!(
            analysis.structures.read_accesses["r"][&Vec::<String>::new()],
            cq_lang_structures::ReadAccess::Whole
        );
        assert!(!analysis.obligations.iter().any(|obligation| matches!(
            obligation,
            Obligation::Provider { provider, keys, .. }
                if provider == "r" && keys.is_empty()
        )));
    }
}

#[test]
fn unit_opaque_maps_and_union_preserve_explicit_arms() {
    let mut p: Program = syn::parse_quote! {
        physical {
            u = pipe unit;
            v = pipe unit;
            a = pipe bind u with n:i32 = (1);
            g = pipe guard a if (*n > 0);
            b = pipe bind v with n:i32 = (1);
            out = let canon union g,b;
            return out as answer(n);
        } structures { providers { sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; } out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows; }
        sequence {
            su = rows u;
            sv = rows v;
            sa = map a su;
            sg = map g sa;
            sb = map b sv;
            result = canon union [sg,sb];
        }
    };
    let analysis = check_grading(&p, &sequential()).unwrap();
    assert_eq!(analysis.results["result"], "out");
    assert_eq!(analysis.lengths["su"], Len::Lit(1));
    assert_eq!(syn::parse2::<Program>(p.to_token_stream()).unwrap(), p);
    grade(
        &mut p,
        quote!(su at host; sv at host; sa at host; sg at host; sb at host; result at host;),
    );
    check_grading(&p, &gpu()).unwrap();
    p.sequence.definitions[5].term = syn::parse_quote!(union[sg]);
    reject(&p, "BE-Alpha");
    p.sequence.definitions[5].term = syn::parse_quote!(map out sg);
    reject(&p, "BE-Alpha");
}

#[test]
fn lookup_and_flatten_are_lifted_under_tiles() {
    let mut p = lookup_program(quote!(a = flatten lookup r [a] tile 256 rows l;));
    let analysis = contract::check(&p).unwrap();
    assert_eq!(analysis.results["a"], "j");
    assert_eq!(analysis.shapes["a"].len(), 2);
    assert_eq!(
        analysis.shapes["a"][0],
        Len::CeilDiv(Box::new(Len::Input("l".into())), 256)
    );
    grade(&mut p, quote!(a at device.block;));
    let analysis = check_grading(&p, &gpu()).unwrap();
    assert_eq!(analysis.dispatches.len(), 1);
    assert_eq!(analysis.dispatches[0].outer_store, "l");
}

#[test]
fn device_store_publishes_tiled_parts_at_block() {
    let mut p = lookup_program(quote!(a = store tile 256 flatten lookup r [a] rows l;));
    grade(&mut p, quote!(a at device;));
    let analysis = check_grading(&p, &gpu()).unwrap();
    assert_eq!(
        analysis.dispatches[0].producer_level,
        ["host", "device", "block"]
    );
    assert_eq!(analysis.dispatches[0].parts, Some(256));
}

#[test]
fn device_store_publishes_collaborative_flatten() {
    let mut p = lookup_program(quote!(a = store flatten lookup r [a] rows l;));
    grade(&mut p, quote!(a at device;));
    let analysis = check_grading(&p, &gpu()).unwrap();
    assert_eq!(
        analysis.dispatches[0].producer_level,
        ["host", "device", "block"]
    );
    assert_eq!(analysis.collectives[0].2, ["host", "device", "block"]);
    assert_eq!(analysis.dispatches[0].parts, None);
}

#[test]
fn publication_distributes_once_and_inner_tiles_strip_mine() {
    let mut p = program(quote!(a = store tile 1024 items 4 tile 256 rows r;));
    grade(&mut p, quote!(a at device;));
    let analysis = check_grading(&p, &gpu()).unwrap();
    assert_eq!(analysis.tilings["a"], [Tiling::Strip, Tiling::Distribute]);
    assert_eq!(analysis.dispatches[0].parts, Some(256));
    grade(&mut p, quote!(a at device.block;));
    let analysis = check_grading(&p, &gpu()).unwrap();
    assert_eq!(analysis.tilings["a"], [Tiling::Strip, Tiling::Strip]);
    grade(&mut p, quote!(a at device.block.lane;));
    reject_grade(&p, &gpu(), "BE-Scope");
}

#[test]
fn publication_resolves_named_views_without_changing_tile_order() {
    let mut p = program(quote! {
        source = store rows r;
        view = tile 1024 items 4 tile 256 source;
        alias = view;
        result = store alias;
    });
    grade(
        &mut p,
        quote! {
            source at host;
            view at device.block;
            alias at device.block;
            result at device;
        },
    );
    let analysis = check_grading(&p, &gpu()).unwrap();
    assert_eq!(
        analysis.tilings["view"],
        [Tiling::Strip, Tiling::Distribute]
    );
    assert_eq!(analysis.dispatches.len(), 1);
    assert_eq!(analysis.dispatches[0].parts, Some(256));
}

#[test]
fn publication_tile_resolution_stops_at_named_storage() {
    let mut p = program(quote! {
        source = store rows r;
        first = store flatten tile 32 source;
        view = tile 256 first;
        result = store view;
    });
    grade(
        &mut p,
        quote! {
            source at host;
            first at device;
            view at device.block;
            result at device;
        },
    );
    let analysis = check_grading(&p, &gpu()).unwrap();
    assert_eq!(analysis.dispatches.len(), 2);
    assert_eq!(analysis.dispatches[0].parts, Some(32));
    assert_eq!(analysis.dispatches[1].parts, Some(256));
    assert_eq!(analysis.tilings["view"], [Tiling::Distribute]);
}

#[test]
fn evidence_forms_round_trip_without_changing_lengths_or_capabilities() {
    let source = lookup_program(quote! {
        left = rows l;
        expanded = flatten lookup r [a] left;
        answer = store canon expanded;
    });
    let mut target = source.clone();
    target.evidence = syn::parse_quote! {
        evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") {
            count R = 0;
            count R = 999;
            planning_key R [b];
            trusted_key R [a];
            foreign_key L [a] -> R [b];
            selectivity answer = 0 / 1;
            bounds R.b = 12 ..= -3;
            sorted R [b, a];
            trusted_key answer [a, b];
        }
    };
    let evidence = &target.evidence;
    assert_eq!(
        syn::parse2::<Evidence>(evidence.to_token_stream()).unwrap(),
        *evidence
    );
    assert_eq!(
        syn::parse2::<Program>(target.to_token_stream()).unwrap(),
        target
    );
    assert_eq!(evidence.count("R"), Some(999));
    assert_eq!(evidence.count("missing"), None);
    assert_eq!(
        evidence.selectivity("answer"),
        Some(Selectivity {
            numerator: 0,
            denominator: 1
        })
    );
    assert!(evidence.has_trusted_key("answer", &["b".into(), "a".into()]));
    assert!(!evidence.has_trusted_key("R", &["b".into()]));
    assert!(evidence.has_planning_key("R", &["b".into()]));
    assert!(!evidence.has_trusted_key("R", &["b".into()]));
    let before = contract::check(&source).unwrap();
    let after = contract::check(&target).unwrap();
    assert_eq!(after.evidence, target.evidence);
    assert_eq!(before.lengths, after.lengths);
    assert_eq!(before.shapes, after.shapes);
    assert_eq!(before.capabilities, after.capabilities);
    assert_eq!(before.obligations, after.obligations);
    assert_eq!(
        before.structures.physical.discharged_host_sets,
        after.structures.physical.discharged_host_sets
    );
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn evidence_rejects_bad_syntax_and_references_not_false_claims() {
    for syntax in [
        quote!(evidence { count r = 1; }),
        quote!(evidence from authored("") { count r = 1; }),
        quote!(evidence from vo("   ") { count r = 1; }),
        quote!(evidence from authored("legacy key syntax") { key R [a]; }),
        quote!(evidence from authored("legacy postfix syntax") { R(a) key; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { selectivity r = 1 / 0; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { count r = -1; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { foreign_key R [a] -> R []; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { bounds R.a = 1; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { unknown R = 1; }),
    ] {
        assert!(syn::parse2::<Evidence>(syntax).is_err());
    }
    for syntax in [
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { count missing = 1; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { trusted_key R [missing]; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { trusted_key R [a, a]; }),
        quote!(evidence from authored("planning-key mutation fixture") { planning_key R [missing]; }),
        quote!(evidence from authored("planning-key mutation fixture") { planning_key R [a, a]; }),
        quote!(evidence from vo("VO advice mutation fixture") { trusted_key R [a]; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { foreign_key R [a] -> missing [a]; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { foreign_key R [a] -> r [missing]; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { bounds R.missing = 0 ..= 10; }),
        quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { sorted r [missing]; }),
    ] {
        let mut target = program(quote!(out = store rows r;));
        target.evidence = syn::parse2(syntax).unwrap();
        assert!(contract::check(&target).is_err());
    }
}

#[test]
fn evidence_bounds_cover_the_signed_integer_domain_and_empty_metadata() {
    let evidence: Evidence = syn::parse_quote! {
        evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") {
            bounds R.a = -170141183460469231731687303715884105728 ..= 170141183460469231731687303715884105727;
        }
    };
    assert_eq!(
        syn::parse2::<Evidence>(evidence.to_token_stream()).unwrap(),
        evidence
    );
    let source = program(quote!(out = store rows r;));
    let parsed: Program = syn::parse2(quote!(#source evidence {})).unwrap();
    assert_eq!(parsed, source);
}

#[test]
fn evidence_mapping_and_composition_preserve_original_sources() {
    let authored: Evidence = syn::parse_quote! {
        evidence from authored("mapped authored fixture") {
            count Original = 0;
            foreign_key Original [a] -> Other [a];
        }
    };
    let vo: Evidence = syn::parse_quote! {
        evidence from vo("original VO manifest provenance") {
            count Original = 999;
            planning_key Original [a];
        }
    };
    let mut combined = authored.clone();
    combined.facts.extend(vo.facts.clone());
    let mapped = combined.map_subjects(|subject| {
        if subject == "Original" {
            syn::parse_quote!(R)
        } else {
            syn::parse_quote!(r)
        }
    });
    assert_eq!(mapped.facts[0].source(), authored.facts[0].source());
    assert_eq!(mapped.facts[2].source(), vo.facts[0].source());
    assert_eq!(mapped.count("R"), Some(999));
    assert_eq!(mapped.count("Original"), None);
    assert!(mapped.has_planning_key("R", &["a".into()]));
    assert!(!mapped.has_trusted_key("R", &["a".into()]));
    assert_eq!(
        syn::parse2::<Evidence>(mapped.to_token_stream()).unwrap(),
        mapped
    );
    let mut target = program(quote!(out = store rows r;));
    target.evidence = mapped;
    contract::check(&target).unwrap();
}

#[test]
fn evidence_wf_rechecks_mutated_sources_and_trusted_key_owners() {
    use cq_lang_sequence::{EvidenceFact, EvidenceSource};
    let mut target = program(quote!(out = store rows r;));
    target.evidence = syn::parse_quote! {
        evidence from authored("trusted-key mutation fixture") { trusted_key R [a]; }
    };
    contract::check(&target).unwrap();
    let EvidenceFact::TrustedKey { source, .. } = &mut target.evidence.facts[0] else {
        unreachable!()
    };
    *source = EvidenceSource::Vo {
        provenance: syn::parse_quote!("original VO source"),
    };
    assert!(contract::check(&target).is_err());

    target.evidence = syn::parse_quote! {
        evidence from authored("count mutation fixture") { count R = 1; }
    };
    for provenance in ["", "  "] {
        let EvidenceFact::Count { source, .. } = &mut target.evidence.facts[0] else {
            unreachable!()
        };
        *source = EvidenceSource::Authored {
            provenance: syn::LitStr::new(provenance, proc_macro2::Span::call_site()),
        };
        assert!(contract::check(&target).is_err());
    }
}

fn grouped_program(keys: proc_macro2::TokenStream, body: proc_macro2::TokenStream) -> Program {
    let head_keys = if keys.is_empty() {
        quote!()
    } else {
        quote!(#keys,)
    };
    let accumulator = if keys.is_empty() {
        quote!()
    } else {
        quote!(grouped group = @group_map as ::std::collections::HashMap<_, (usize, i64, Option<i64>, Option<i64>,)>;)
    };
    let mut program: Program = syn::parse2(quote! {
        physical {
            relation R(k:i32, witness:i32, value:i64);
            r = let canon rename R@0 {k -> k, witness -> witness, value -> value};
            grouped = pipe group r by [#keys] {
                n: usize = count();
                total: i64 = sum(value);
                low: Option<i64> = min(value);
                high: Option<i64> = max(value);
            };
            return grouped as answer(#head_keys n, total, low, high);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            r host scan = @reader as Reader [];
            r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows;
            #accumulator
        }
        sequence { #body }
    })
    .unwrap();
    program.structures.structures.declare(&[syn::parse_quote!(
        group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed };
    )]).unwrap();
    program
}

#[test]
fn grouped_fold_records_result_cardinality_and_store_owner() {
    for (keys, global) in [(quote!(k), false), (quote!(), true)] {
        let program = grouped_program(
            keys,
            quote! {
                r = canon rows r;
                groups = group grouped r;
                output_rows = store groups;
            },
        );
        assert_eq!(
            syn::parse2::<Program>(program.to_token_stream()).unwrap(),
            program
        );
        let analysis = check_grading(&program, &sequential()).unwrap();
        assert_eq!(analysis.results["groups"], "grouped");
        assert!(!analysis.views.contains("groups"));
        assert_eq!(analysis.stores["output_rows"], analysis.lengths["groups"]);
        assert_eq!(analysis.shapes["groups"].len(), 1);
        if global {
            assert_eq!(analysis.lengths["groups"], Len::Lit(1));
        } else {
            assert_eq!(analysis.lengths["groups"], Len::Scan("groups".into()));
        }
    }
}

#[test]
fn nested_group_does_not_certify_outer_group_canon_removal() {
    let p: Program = syn::parse_quote! {
        physical {
            relation R(k:i32);
            r = let canon rename R@0 {k -> k};
            inner = pipe group r by [k] { n: usize = count(); };
            projected = let canon project inner keep {n};
            outer = pipe group projected by [] { total: usize = count(); };
            return outer as answer(total);
        }
        structures {
            providers {
                reader = Reader { reads [exact, range, membership], build inferred };
                sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared };
                groups = ::std::collections::HashMap<_, _> { reads [], build typed };
            }
            r host scan = @reader as Reader [];
            r rows = @sort as ::cq_storage::dedup::Sort;
            r canon = rows;
            inner group = @groups as ::std::collections::HashMap<_, _>;
            projected rows = @sort as ::cq_storage::dedup::Sort;
            projected canon = rows;
        }
        sequence {
            input = store canon rows r;
            out = store group outer canon map projected group inner rows r;
        }
    };
    let analysis = check_grading(&p, &sequential()).unwrap();
    assert_eq!(analysis.partial_groups["out"].len(), 2);
    assert!(!analysis.canon_free_groups.contains("out"));
    let mut without_outer_canon = p.clone();
    without_outer_canon.sequence.definitions[1].term =
        syn::parse_quote!(store group outer map projected group inner rows r);
    reject(&without_outer_canon, "BE-Group");
}

#[test]
fn grouped_fold_requires_complete_globally_distinct_valuations() {
    for body in [
        quote!(r = rows r; groups = group grouped r;),
        quote!(r = canon rows r; groups = map grouped r;),
        quote!(r = canon rows r; groups = group r r;),
        quote!(r = rows r; groups = group grouped tile 2 r;),
        quote!(r = rows r; groups = group grouped canon tile 2 r;),
        quote!(r = canon rows r; groups = group grouped fold count r;),
        quote!(r = flatten canon tile 2 rows r; groups = group grouped r;),
    ] {
        reject(&grouped_program(quote!(k), body), "BE-Group");
    }
    // Global canonicalization establishes uniqueness across the contiguous
    // parts; this is now a legal keyed partial fold with a flat merged result.
    let accepted = grouped_program(
        quote!(k),
        quote!(
            r = canon rows r; groups = group grouped tile 2 r;
        ),
    );
    let facts = contract::check(&accepted).unwrap();
    assert_eq!(facts.partial_groups["groups"], [true]);
    assert_eq!(facts.shapes["groups"].len(), 1);
}

#[test]
fn grouped_fold_cannot_move_into_a_separated_or_leaf_execution_level() {
    for level in [
        quote!(device),
        quote!(device.block),
        quote!(device.block.lane),
    ] {
        let mut program = grouped_program(
            quote!(k),
            quote! {
                r = store canon rows r;
                groups = group grouped r;
            },
        );
        grade(&mut program, quote!(r at host; groups at #level;));
        reject_grade(&program, &gpu(), "BE-Group");
    }
}

#[test]
fn shared_checker_invalidates_changed_terms_structures_and_evidence() {
    let source = lookup_program(quote! {
        sr = store rows r;
        out = store canon flatten lookup r [a] rows l;
    });
    let mut checker = contract::Checker::default();
    let first = checker.check(&source).unwrap();
    assert_eq!(checker.check(&source).unwrap().shapes, first.shapes);

    let mut changed = source.clone();
    changed.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(store rows missing);
    assert!(checker.check(&changed).is_err());

    let mut changed = source.clone();
    changed.structures.structures.entries.clear();
    assert!(checker.check(&changed).is_err());

    let mut changed = source.clone();
    changed.evidence = syn::parse_quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { count missing = 10; });
    assert!(checker.check(&changed).is_err());
    assert_eq!(checker.check(&source).unwrap().shapes, first.shapes);
}

#[test]
fn shared_checker_retains_source_analysis_across_candidate_exploration() {
    let source = program(quote!(out = store rows r;));
    let mut checker = contract::Checker::new(&source).unwrap();
    let original = checker.check_shared(&source).unwrap();
    assert!(std::rc::Rc::ptr_eq(
        &original,
        &checker.check_shared(&source.clone()).unwrap(),
    ));
    for width in 1..8 {
        let mut candidate = source.clone();
        candidate.sequence.definitions[0].term =
            syn::parse2(quote!(store flatten tile #width rows r)).unwrap();
        checker.check_rewrite_in(&source, &candidate).unwrap();
        checker.check_shared(&candidate).unwrap();
    }
    assert!(std::rc::Rc::ptr_eq(
        &original,
        &checker.check_shared(&source).unwrap(),
    ));

    let mut output = source.clone();
    output.sequence.output = Some(OutputProtocol::Reserve);
    let reserved = checker.check_shared(&output).unwrap();
    assert!(!std::rc::Rc::ptr_eq(&original, &reserved));
    assert_eq!(
        reserved.output,
        Some(("out".into(), OutputProtocol::Reserve))
    );
    output.sequence.output = Some(OutputProtocol::ReservePrevious);
    let previous = checker.check_shared(&output).unwrap();
    assert!(!std::rc::Rc::ptr_eq(&reserved, &previous));
    assert_eq!(
        previous.output,
        Some(("out".into(), OutputProtocol::ReservePrevious))
    );
    output.sequence.output = Some(OutputProtocol::Bounded);
    assert!(checker.check_shared(&output).is_err());
}

#[test]
fn shared_checker_rechecks_evidence_and_complete_opaque_prefix_syntax() {
    let mut source = program(quote!(out = store rows r;));
    source.structures.structures = syn::parse_quote!(structures { providers { reader = Reader<{ opaque!(left) }> { reads [exact, range, membership], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
        r host scan = @reader as Reader<{ opaque!(left) }> [];
        r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows;
    });
    let mut checker = contract::Checker::new(&source).unwrap();
    let original = checker.check_shared(&source).unwrap();

    let mut advice = source.clone();
    advice.evidence = syn::parse_quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { count R = 10; });
    let advised = checker.check_shared(&advice).unwrap();
    assert_eq!(advised.evidence.count("R"), Some(10));
    assert_eq!(advised.obligations, original.obligations);
    advice.evidence = syn::parse_quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { count missing = 10; });
    assert!(checker.check_shared(&advice).is_err());

    let mut changed = source.clone();
    changed.structures.structures = syn::parse_quote!(structures { providers { reader = Reader<{ opaque!(right) }> { reads [exact, range, membership], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
        r host scan = @reader as Reader<{ opaque!(right) }> [];
        r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows;
    });
    assert!(!std::rc::Rc::ptr_eq(
        &original,
        &checker.check_shared(&changed).unwrap(),
    ));
    assert!(checker.check_rewrite_in(&source, &changed).is_err());
}

#[test]
fn shared_structural_proof_does_not_cache_machine_grading() {
    let mut source = program(quote!(out = store rows r;));
    grade(&mut source, quote!(out at host;));
    let mut checker = contract::Checker::default();
    checker.check_grading(&source, &gpu()).unwrap();
    let original = source.clone();
    grade(&mut source, quote!(out at unknown;));
    checker.check_rewrite_in(&original, &source).unwrap();
    assert!(checker.check_grading(&source, &gpu()).is_err());
    let invalid = program(quote!(out = store rows missing;));
    assert!(checker.check_rewrite_in(&invalid, &invalid).is_err());
}

#[test]
fn resource_rejects_oversized_authored_dispatches_before_translation() {
    let mut p = lookup_program(quote! {
        sr = store rows r;
        out = store flatten lookup r [a] tile 256 rows l;
    });
    grade(&mut p, quote!(sr at host; out at device;));
    let mut machine = gpu();
    machine.root.children[0].children[0].shared_bytes = Some(64);
    machine.layout = Some(|_| Ok(65));
    let error = check_grading(&p, &machine).unwrap_err().to_string();
    assert!(error.contains("BE-Resource"), "{error}");
    machine.root.children[0].children[0].shared_bytes = Some(65);
    let analysis = check_grading(&p, &machine).unwrap();
    assert_eq!(analysis.resources["out"].required_bytes, 65);
    machine.layout = None;
    assert!(
        check_grading(&p, &machine)
            .unwrap_err()
            .to_string()
            .contains("BE-Resource")
    );
}

#[test]
fn shared_choices_reuse_provider_obligations_without_repeating_reads() {
    let mut source = lookup_program(quote! {
        l = rows l;
        r = store rows r;
        joined = canon flatten lookup r [a] l;
    });
    let mut previous: syn::Ident = syn::parse_quote!(joined);
    for index in 0..12 {
        let name = syn::Ident::new(&format!("choice_{index}"), previous.span());
        source.sequence.definitions.push(syn::parse_quote!(
            #name = choose [len l < 1 => #previous, else => #previous];
        ));
        previous = name;
    }
    source
        .sequence
        .definitions
        .push(syn::parse_quote!(out = store #previous;));
    let facts = contract::check(&source).unwrap();
    assert_eq!(facts.provider_reads.len(), 1);
    assert!(facts.provider_reads["joined"].contains("r"));
    assert_eq!(facts.obligations.iter().filter(|obligation|
        matches!(obligation, Obligation::Provider { provider, .. } if provider == "r")
    ).count(), 1);
    assert!(facts.obligations.len() <= 5, "{:?}", facts.obligations);
}

fn key_pool() -> Machine {
    let mut pool = node(
        "pool",
        false,
        true,
        vec![node("task", false, false, vec![])],
    );
    pool.width = Some(4);
    Machine {
        gpu: None,
        key_partition_threshold: 65536,
        root: node("host", false, false, vec![pool]),
        ..sequential()
    }
}

#[test]
fn key_partition_roundtrip_canon_and_group_laws() {
    let source = grouped_program(
        quote!(k),
        quote!(
            input = store rows r;
            out = store group grouped canon input;
        ),
    );
    let mut target = grouped_program(
        quote!(k),
        quote!(
            input = store rows r;
            out = store flatten group grouped canon partition_k[k](4) input;
        ),
    );
    let printed = target.to_token_stream();
    assert_eq!(target, syn::parse2::<Program>(printed).unwrap());
    contract::check_rewrite(&source, &target).unwrap();
    grade(&mut target, quote!(input at host; out at pool;));
    let facts = check_grading(&target, &key_pool()).unwrap();
    assert_eq!(facts.shapes["out"].len(), 1);

    let source = program(quote!(input = store rows r; out = store canon input;));
    let mut target = program(quote!(
        input = store rows r;
        out = store flatten canon partition_k[a](4) input;
    ));
    contract::check_rewrite(&source, &target).unwrap();
    grade(&mut target, quote!(input at host; out at pool;));
    check_grading(&target, &key_pool()).unwrap();
}

#[test]
fn key_partition_rejects_missing_ancestor_store_and_invalid_width() {
    reject(
        &program(quote!(out = partition_k[a](4) rows r;)),
        "BE-KeyPart",
    );
    reject(
        &program(quote!(input = rows r; out = partition_k[a](4) input;)),
        "BE-KeyPart",
    );
    reject(
        &program(quote!(input = store rows r; out = partition_k[a](0) input;)),
        "BE-KeyPart",
    );
    let mut p = program(quote!(input = store rows r; out = partition_k[a](4) input;));
    grade(&mut p, quote!(input at pool; out at pool;));
    reject_grade(&p, &key_pool(), "BE-KeyPart");
    grade(&mut p, quote!(input at host; out at host;));
    reject_grade(&p, &key_pool(), "BE-KeyPart");
    grade(&mut p, quote!(input at host; out at pool;));
    let mut machine = key_pool();
    machine.root.children[0].width = Some(8);
    reject_grade(&p, &machine, "BE-KeyPart");
}

#[test]
fn runtime_provider_cover_keeps_routing_and_grades_both_branches() {
    let mut p = lookup_program(quote!(
        input = store rows r;
        provider = store choose [
            len input < 65536 => partition_k[a](1) input,
            else => partition_k[a](4) input
        ];
        out = flatten lookup r[a] rows l;
    ));
    grade(
        &mut p,
        quote!(input at host; provider at pool; out at host;),
    );
    let facts = check_grading(&p, &key_pool()).unwrap();
    assert_eq!(facts.shapes["provider"][0], Len::Unknown(None));
    assert_eq!(p, syn::parse2(p.to_token_stream()).unwrap());

    let mut wrong_width = key_pool();
    wrong_width.root.children[0].width = Some(8);
    reject_grade(&p, &wrong_width, "BE-KeyPart");
    grade(
        &mut p,
        quote!(input at pool; provider at pool; out at host;),
    );
    reject_grade(&p, &key_pool(), "BE-KeyPart");
}

#[test]
fn key_partition_rejects_undetermined_valuation_group_and_lookup_keys() {
    reject(
        &program(quote!(
            input = store rows r;
            out = canon partition_k[missing](4) input;
        )),
        "BE-KeyPart",
    );
    reject(
        &grouped_program(
            quote!(k),
            quote!(
                input = store rows r;
                out = flatten group grouped canon partition_k[value](4) input;
            ),
        ),
        "BE-KeyPartGroup",
    );
    reject(
        &lookup_program(quote!(
            input = store rows r;
            provider = store partition_k[b](4) input;
            out = flatten lookup r[a] rows l;
        )),
        "BE-KeyPartProvider",
    );
    contract::check(&lookup_program(quote!(
        input = store rows r;
        provider = store partition_k[](4) input;
        out = flatten lookup r[a] rows l;
    )))
    .unwrap();
    contract::check(&lookup_program(quote!(
        input = store rows r;
        provider = store partition_k[a](4) input;
        out = flatten lookup r[a] rows l;
    )))
    .unwrap();
}

#[test]
fn key_partition_does_not_authorize_a_collective_at_separated_pool() {
    let mut p = program(quote!(
        input = store rows r;
        out = fold count partition_k[a](4) input;
    ));
    grade(&mut p, quote!(input at host; out at pool;));
    reject_grade(&p, &key_pool(), "BE-Collective");
}

#[test]
fn key_partition_canon_rejects_projection_losing_the_routing_key() {
    let p: Program = syn::parse_quote! {
        physical {
            relation R(a:i32, b:i32);
            r = pipe rename R@0 {a -> a, b -> b};
            projected = let canon project r keep {a};
            return projected as answer(a);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; } r host scan = @reader as Reader []; projected rows = @sort as ::cq_storage::dedup::Sort; projected canon = rows; }
        sequence {
            input = store rows r;
            out = flatten canon map projected partition_k[b](4) input;
        }
    };
    reject(&p, "BE-KeyPartCanon");
}

#[test]
fn key_partition_does_not_promote_subtile_canon_to_complete_valuations() {
    reject(
        &grouped_program(
            quote!(k),
            quote!(
                input = store rows r;
                out = flatten group grouped flatten canon tile 2 partition_k[k](4) input;
            ),
        ),
        "BE-Group",
    );
}

#[test]
fn key_partition_provider_accepts_projected_and_reordered_lookup_keys() {
    for routing in [quote!(b), quote!(b, a)] {
        let p: Program = syn::parse2(quote! {
            physical {
                relation L(a:i32,b:i32); relation R(a:i32,b:i32,c:i32);
                l = pipe rename L@0 {a -> a,b -> b};
                r = let rename R@1 {a -> a,b -> b,c -> c};
                j = let canon probe first [a,b] l,r;
                return j as answer(a,b,c);
            }
            structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; provider = Provider { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
                l host scan = @reader as Reader []; r host scan = @reader as Reader [];
                r = @provider as Provider [a,b]; j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
            }
            sequence {
                input = store rows r;
                provider = store partition_k[#routing](4) input;
                out = flatten lookup r[a,b] rows l;
            }
        })
        .unwrap();
        contract::check(&p).unwrap();
    }
}

#[test]
fn global_partial_group_accepts_key_canon_or_globally_unique_tiles() {
    let mut routed = grouped_program(
        quote!(),
        quote!(
            input = store rows r;
            out = store group grouped canon partition_k[k,witness,value](4) input;
        ),
    );
    grade(&mut routed, quote!(input at host; out at pool;));
    let facts = check_grading(&routed, &key_pool()).unwrap();
    assert_eq!(facts.shapes["out"], vec![Len::Lit(1)]);
    let source = grouped_program(
        quote!(),
        quote!(
            input = store rows r;
            out = store group grouped canon input;
        ),
    );
    contract::check_rewrite(&source, &routed).unwrap();

    let mut tiled = grouped_program(
        quote!(),
        quote!(
            input = store rows r;
            out = store group grouped tile 4 input;
        ),
    );
    tiled.evidence = syn::parse_quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { trusted_key R [witness]; });
    grade(&mut tiled, quote!(input at host; out at pool;));
    let facts = check_grading(&tiled, &key_pool()).unwrap();
    assert_eq!(facts.shapes["out"], vec![Len::Lit(1)]);
}

#[test]
fn keyed_partial_groups_merge_globally_unique_contiguous_parts() {
    let mut tiled = grouped_program(
        quote!(k),
        quote!(
            input = store rows r;
            out = store group grouped tile 4 input;
        ),
    );
    tiled.evidence = syn::parse_quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { trusted_key R [witness]; });
    grade(&mut tiled, quote!(input at host; out at pool;));
    let facts = check_grading(&tiled, &key_pool()).unwrap();
    assert_eq!(facts.shapes["out"], vec![Len::Scan("out".into())]);
    assert_eq!(facts.partial_groups["out"], [true]);
    let mut source = grouped_program(
        quote!(k),
        quote!(
            input = store rows r;
            out = store group grouped input;
        ),
    );
    source.evidence = tiled.evidence.clone();
    contract::check_rewrite(&source, &tiled).unwrap();

    // Empty keyed input yields no groups, unlike the unkeyed identity row.
    tiled.evidence = syn::parse_quote!(evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { trusted_key R [witness]; count R = 0; });
    assert_ne!(
        check_grading(&tiled, &key_pool()).unwrap().lengths["out"],
        Len::Lit(1)
    );
    for input in [quote!(tile 4 input), quote!(canon tile 4 input)] {
        reject(
            &grouped_program(
                quote!(k),
                quote!(
                    input = store rows r;
                    out = store group grouped #input;
                ),
            ),
            "BE-GroupPartial",
        );
    }
    // Key routing still represents independent groups, not host map merging.
    let routed = grouped_program(
        quote!(k),
        quote!(
            input = store rows r;
            out = store group grouped canon partition_k[k](4) input;
        ),
    );
    assert_eq!(contract::check(&routed).unwrap().shapes["out"].len(), 2);
    assert_eq!(
        contract::check(&routed).unwrap().partial_groups["out"],
        [false]
    );
}

#[test]
fn partial_group_metadata_uses_constructor_preorder_and_definition_ownership() {
    let mut p: Program = syn::parse_quote! {
        physical {
            relation R(k:i32, witness:i64, value:i64);
            r = let canon rename R@0 {k -> k, witness -> witness, value -> value};
            by_key = let canon group r by [k] { subtotal:i64 = sum(value); };
            all_rows = pipe group by_key by [] { grand_total:i64 = sum(subtotal); };
            return all_rows as answer(grand_total);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed }; }
            r host scan = @reader as Reader []; r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows;
            by_key rows = @sort as ::cq_storage::dedup::Sort; by_key canon = rows;
            by_key group = @group_map as ::std::collections::HashMap<_, (i64,)>;
        }
        sequence {
            input = store rows r;
            output_rows = store group all_rows group by_key tile 4 input;
        }
        evidence from authored("languages/cq-lang-sequence/tests/sequence_language.rs fixture") { trusted_key R [witness]; }
    };
    assert_eq!(
        contract::check(&p).unwrap().partial_groups["output_rows"],
        [false, true]
    );
    p.sequence.definitions[1].term = syn::parse_quote!(store group all_rows choose [
        len input < 1 => group by_key tile 4 input,
        else => group by_key input
    ]);
    p.sequence
        .definitions
        .push(syn::parse_quote!(alias = output_rows;));
    let facts = contract::check(&p).unwrap();
    assert_eq!(facts.partial_groups["output_rows"], [false, true, false]);
    assert!(!facts.partial_groups.contains_key("input"));
    assert!(!facts.partial_groups.contains_key("alias"));
    grade(
        &mut p,
        quote!(input at host; output_rows at host; alias at host;),
    );
    let graded = check_grading(&p, &sequential()).unwrap();
    assert_eq!(graded.partial_groups, facts.partial_groups);
}

#[test]
fn global_partial_group_rejects_nonunique_tiles_even_with_local_canon() {
    for input in [
        quote!(tile 4 input),
        quote!(canon tile 4 input),
        quote!(partition_k[k](4) input),
    ] {
        reject(
            &grouped_program(
                quote!(),
                quote!(
                    input = store rows r;
                    out = store group grouped #input;
                ),
            ),
            "BE-GroupPartial",
        );
    }
    // Canonicalizing globally before contiguous tiling is also sound.
    contract::check(&grouped_program(
        quote!(),
        quote!(
            input = store canon rows r;
            out = store group grouped tile 4 input;
        ),
    ))
    .unwrap();
}

#[test]
fn storage_type_rewrites_preserve_adapters_and_key_layouts() {
    let source = lookup_program(quote!(
        input = store rows r; out = store canon flatten lookup r[a] rows l;
    ));
    let mut target = source.clone();
    for entry in &mut target.structures.structures.entries {
        match &mut entry.structure {
            structures::Structure::Built { keyed, .. } => {
                keyed.provider = syn::parse_quote!(@native_provider as NativeProvider)
            }
            structures::Structure::Rows { provider, .. } => {
                *provider = syn::parse_quote!(@native_canon as NativeCanon)
            }
            _ => {}
        }
    }
    target.structures.structures.declare(&[
        syn::parse_quote!(native_provider = NativeProvider { reads [exact, range, membership], build inferred };),
        syn::parse_quote!(native_canon = NativeCanon { reads [distinct], build inferred };),
    ]).unwrap();
    contract::check_rewrite(&source, &target).unwrap();
    for entry in &mut target.structures.structures.entries {
        if let structures::Structure::Host { keyed, .. } = &mut entry.structure {
            keyed.provider = syn::parse_quote!(@different_reader as DifferentReader);
        }
    }
    target
        .structures
        .structures
        .declare(&[syn::parse_quote!(
            different_reader = DifferentReader { reads [exact, range, membership], build inferred };
        )])
        .unwrap();
    assert!(contract::check_rewrite(&source, &target).is_err());
}

#[test]
fn task_local_nested_lookup_flatten_requires_only_exact_provider_access() {
    let mut p = lookup_program(quote!(
        source = store rows r;
        provider = store partition_k[a](4) source;
        input = store rows l;
        out = store flatten flatten lookup r[a] tile 4 input;
    ));
    grade(
        &mut p,
        quote!(
            source at host; provider at pool; input at host; out at pool;
        ),
    );
    let key = ("r".into(), vec!["a".into()]);
    let facts = check_grading(&p, &key_pool()).unwrap();
    assert_eq!(facts.capabilities[&key], Capability::Exact);
    assert!(facts.obligations.iter().all(|obligation| !matches!(
        obligation,
        Obligation::Provider {
            capability: Capability::Range,
            ..
        }
    )));
}
