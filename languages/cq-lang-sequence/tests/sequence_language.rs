use cq_lang_sequence::*;
use quote::{ToTokens, quote};

fn program(body: proc_macro2::TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        } structures {
            r host scan = Reader [];
            r canon = Canon;
        }
        sequence { #body }
    })
    .unwrap()
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
            r = pipe canon rename R@0 {a -> a};
            s = pipe canon project r keep {a};
            return s as answer(a);
        }
        structures { r host scan = Reader []; r canon = Canon; s canon = Canon; }
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
    keyed.ty = syn::parse_quote!(::cq_storage::sharded::Sharded<Provider, _>);
    let analysis = contract::check(&p).unwrap();
    assert_eq!(
        analysis.publications["r"],
        Publication::Parts {
            keys: vec!["a".into()]
        }
    );
    assert_eq!(
        analysis.routed_providers["r"][&0],
        syn::parse_quote!(Provider)
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
    keyed.ty = syn::parse_quote!(::cq_storage::sharded::Sharded);
    reject(&p, "BE-Publication");
}

#[test]
fn unrelated_sharded_type_is_not_a_native_routed_provider() {
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
    keyed.ty = syn::parse_quote!(::custom::Sharded<Provider>);
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
            r = pipe canon rename R@0 {a -> a,b -> b};
            selected = let select r if (b > 0);
            probe = pipe rename P@1 {a -> a};
            out = pipe canon probe first [a] probe,selected;
            return out as answer(a,b);
        }
        structures {
            r host scan = Reader [];
            r canon = Canon;
            selected = ::cq_storage::sharded::Sharded<Provider> [a];
            probe host scan = Reader [];
            out canon = Canon;
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
        (false, quote!(selected rows = Rows;)),
        (
            true,
            quote! {
                selected rows = Rows;
                selected = ::cq_storage::sharded::Sharded<OtherProvider> [a,b];
            },
        ),
    ] {
        let mut p = deferred_publication_program(quote! {
            input = store compact selected rows r;
            #cover
            out = flatten lookup selected [a] rows probe;
        });
        let additional: structures::Structures =
            syn::parse2(quote!(structures { #extra })).unwrap();
        p.structures.structures.entries.extend(additional.entries);
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
                .push(syn::parse_quote!(out = pipe canon probe first [a,b] first,selected;));
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
                r = pipe canon rename R@0 {a -> a,b -> b};
                selected = let select r if (b > 0);
                alias = let project selected keep {a,b};
                probe = pipe rename P@1 {a -> a};
                first = pipe probe first [a] probe,selected;
                out = pipe canon probe first [a,b] first,alias;
                return out as answer(a,b);
            }
            structures {
                r host scan = Reader [];
                r canon = Canon;
                probe host scan = Reader [];
                selected = ::cq_storage::sharded::Sharded<Provider> [a];
                selected rows = Rows;
                alias = ::cq_storage::sharded::Sharded<AliasProvider> [a,b];
                out canon = Canon;
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
            out = pipe canon probe first [a] probe,selected;
            return out as answer(a);
        }
        structures {
            l host scan = Reader [];
            r host scan = Reader [];
            b = ::cq_storage::HashIndex<'_, _, _> [a];
            l join_filter = Filter build b [a] covering selected marker ::cq_load::ColumnAt<0>;
            selected = ::cq_storage::sharded::Sharded<Provider> [a];
            probe host scan = Reader [];
            out canon = Canon;
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
            j = pipe canon probe first [a] l,r;
            return j as answer(a,b);
        } structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r = Provider [a];
            j canon = Canon;
        }
        sequence { #body }
    })
    .unwrap()
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
            rows = pipe canon rename R@0 {a -> a};
            return rows as answer(a);
        } structures {
            rows host scan = Reader [];
            rows canon = Canon;
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
    assert!(
        a.obligations
            .iter()
            .any(|o| matches!(o, Obligation::Canon { .. }))
    );
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
    assert!(a.obligations.iter().any(|o| matches!(o, Obligation::Provider { capability: Capability::Range, ty, .. } if ty.to_token_stream().to_string() == "Provider")));
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
    changed.structures.structures.entries[0] = syn::parse_quote!(r host scan = Other [] ;);
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
        } structures {
            r host scan = Reader [];
            b canon = Canon;
            b rows = Rows;
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
        } structures {
            r host scan = Reader [];
            g canon = Canon;
            g rows = Rows;
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
            j = pipe canon probe first [a] l,selected;
            return j as answer(a,tag);
        } structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r host result(selected) = Reader [tag];
            selected = Provider [a] membership payload [a] constants {tag = 7};
            j canon = Canon;
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
            s = pipe canon select r if (a >= 0);
            return s as answer(a);
        } structures {
            r host scan = Reader [];
            s canon = Canon;
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
        } structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r = Provider [a];
            j rows = Retained;
            j canon = Canon;
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
            j = pipe canon {operation} eager [a] l,r;
            return j as answer(a);
        }} structures {{
            l host scan = Reader []; r host scan = Reader [];
            r = Provider [a]; j canon = Canon;
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
fn empty_key_membership_uses_named_reader_not_scan_adapter() {
    for operation in ["semi", "anti"] {
        let program: Program = syn::parse_str(&format!(
            "physical {{
                relation L(a:i32); relation R(b:i32);
                l = pipe rename L@0 {{a -> a}};
                r = pipe rename R@1 {{b -> b}};
                scan = pipe canon {operation} eager [] l,r;
                return scan as answer(a);
            }} structures {{
                l host scan = ScanAdapter [];
                r host scan = ScanAdapter [];
                r host result(scan) = MembershipProvider [];
                scan canon = Canon;
            }} sequence {{
                answer = compact scan lookup r [] rows l;
            }}"
        ))
        .unwrap();
        let analysis = contract::check(&program).unwrap();
        assert!(analysis.obligations.iter().any(|obligation| matches!(
            obligation,
            Obligation::Provider { provider, keys, capability: Capability::Membership, ty }
                if provider == "r" && keys.is_empty()
                    && ty.to_token_stream().to_string() == "MembershipProvider"
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
            out = pipe canon union g,b;
            return out as answer(n);
        } structures { out canon = Canon; }
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
        evidence {
            count R = 0;
            count R = 999;
            R(a) key;
            foreign_key L [a] -> R [b];
            selectivity answer = 0 / 1;
            bounds R.b = 12 ..= -3;
            sorted R [b, a];
            key answer [a, b];
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
    assert!(evidence.has_key("answer", &["b".into(), "a".into()]));
    assert!(!evidence.has_key("R", &["b".into()]));
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
        quote!(evidence { selectivity r = 1 / 0; }),
        quote!(evidence { count r = -1; }),
        quote!(evidence { foreign_key R [a] -> R []; }),
        quote!(evidence { bounds R.a = 1; }),
        quote!(evidence { unknown R = 1; }),
    ] {
        assert!(syn::parse2::<Evidence>(syntax).is_err());
    }
    for syntax in [
        quote!(evidence { count missing = 1; }),
        quote!(evidence { key R [missing]; }),
        quote!(evidence { key R [a, a]; }),
        quote!(evidence { foreign_key R [a] -> missing [a]; }),
        quote!(evidence { foreign_key R [a] -> r [missing]; }),
        quote!(evidence { bounds R.missing = 0 ..= 10; }),
        quote!(evidence { sorted r [missing]; }),
    ] {
        let mut target = program(quote!(out = store rows r;));
        target.evidence = syn::parse2(syntax).unwrap();
        reject(&target, "evidence:");
    }
}

#[test]
fn evidence_bounds_cover_the_signed_integer_domain_and_empty_metadata() {
    let evidence: Evidence = syn::parse_quote! {
        evidence {
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

fn grouped_program(keys: proc_macro2::TokenStream, body: proc_macro2::TokenStream) -> Program {
    let head_keys = if keys.is_empty() {
        quote!()
    } else {
        quote!(#keys,)
    };
    let accumulator = if keys.is_empty() {
        quote!()
    } else {
        quote!(grouped group = ::std::collections::HashMap<_, (usize, i64, Option<i64>, Option<i64>,)>;)
    };
    syn::parse2(quote! {
        physical {
            relation R(k:i32, witness:i32, value:i64);
            r = pipe canon rename R@0 {k -> k, witness -> witness, value -> value};
            grouped = pipe group r by [#keys] {
                n: usize = count();
                total: i64 = sum(value);
                low: Option<i64> = min(value);
                high: Option<i64> = max(value);
            };
            return grouped as answer(#head_keys n, total, low, high);
        }
        structures {
            r host scan = Reader [];
            r canon = Canon;
            #accumulator
        }
        sequence { #body }
    })
    .unwrap()
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
    changed.evidence = syn::parse_quote!(evidence { count missing = 10; });
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
    source.structures.structures = syn::parse_quote!(structures {
        r host scan = Reader<{ opaque!(left) }> [];
        r canon = Canon;
    });
    let mut checker = contract::Checker::new(&source).unwrap();
    let original = checker.check_shared(&source).unwrap();

    let mut advice = source.clone();
    advice.evidence = syn::parse_quote!(evidence { count R = 10; });
    let advised = checker.check_shared(&advice).unwrap();
    assert_eq!(advised.evidence.count("R"), Some(10));
    assert_eq!(advised.obligations, original.obligations);
    advice.evidence = syn::parse_quote!(evidence { count missing = 10; });
    assert!(checker.check_shared(&advice).is_err());

    let mut changed = source.clone();
    changed.structures.structures = syn::parse_quote!(structures {
        r host scan = Reader<{ opaque!(right) }> [];
        r canon = Canon;
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
    assert!(error.contains("65 shared bytes"), "{error}");
    assert!(error.contains("capacity is 64"), "{error}");
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
            projected = pipe canon project r keep {a};
            return projected as answer(a);
        }
        structures { r host scan = Reader []; projected canon = Canon; }
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
                j = pipe canon probe first [a,b] l,r;
                return j as answer(a,b,c);
            }
            structures {
                l host scan = Reader []; r host scan = Reader [];
                r = Provider [a,b]; j canon = Canon;
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
    tiled.evidence = syn::parse_quote!(evidence { R(witness) key; });
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
    tiled.evidence = syn::parse_quote!(evidence { R(witness) key; });
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
    tiled.evidence = syn::parse_quote!(evidence { R(witness) key; count R = 0; });
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
            r = pipe canon rename R@0 {k -> k, witness -> witness, value -> value};
            by_key = pipe canon group r by [k] { subtotal:i64 = sum(value); };
            all_rows = pipe group by_key by [] { grand_total:i64 = sum(subtotal); };
            return all_rows as answer(grand_total);
        }
        structures {
            r host scan = Reader []; r canon = Canon; by_key canon = Canon;
            by_key group = ::std::collections::HashMap<_, (i64,)>;
        }
        sequence {
            input = store rows r;
            output_rows = store group all_rows group by_key tile 4 input;
        }
        evidence { key R [witness]; }
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
                keyed.ty = syn::parse_quote!(NativeProvider)
            }
            structures::Structure::Dedup { ty, .. } => *ty = syn::parse_quote!(NativeCanon),
            _ => {}
        }
    }
    contract::check_rewrite(&source, &target).unwrap();
    for entry in &mut target.structures.structures.entries {
        if let structures::Structure::Host { keyed, .. } = &mut entry.structure {
            keyed.ty = syn::parse_quote!(DifferentReader);
        }
    }
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
