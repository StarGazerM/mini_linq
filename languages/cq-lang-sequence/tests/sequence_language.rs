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
        name: syn::parse_str(name).unwrap(),
        collaborative,
        dynamic,
        width: (name == "block").then_some(256),
        children,
    }
}

fn sequential() -> Machine {
    Machine {
        name: syn::parse_quote!(target),
        items: 1,
        root: node("host", false, false, vec![]),
    }
}

fn gpu() -> Machine {
    Machine {
        name: syn::parse_quote!(target),
        items: 4,
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
    ] {
        let term: Term = syn::parse2(term).unwrap();
        assert_eq!(syn::parse2::<Term>(term.to_token_stream()).unwrap(), term);
    }
}

#[test]
fn constructor_names_remain_definition_names_and_references() {
    for spelling in [
        "rows", "map", "flatten", "compact", "tile", "lookup", "fold", "group", "canon", "store",
        "union", "choose", "capacity", "output",
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
    let p = lookup_program(quote!(a = flatten lookup r [a] tile 8 rows l;));
    let a = contract::check(&p).unwrap();
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
    for protocol in [quote!(count), quote!(reserve), quote!(bounded)] {
        contract::check(&program(
            quote!(a = store capacity 8 rows r; output #protocol;),
        ))
        .unwrap();
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
fn partial_rejects_opaque_evaluation_below_host() {
    let mut p: Program = syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe rename R@0 {a -> a};
            b = let canon bind r with n:i32 = (1);
            return b as answer(a,n);
        } structures {
            r host scan = Reader [];
            b canon = Canon;
            b rows = Rows;
        } sequence { a = store map b rows r; }
    };
    grade(&mut p, quote!(a at device;));
    reject_grade(&p, &gpu(), "BE-Partial");
    grade(&mut p, quote!(a at host;));
    check_grading(&p, &gpu()).unwrap();
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
            r host selected = Reader [tag];
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
        name: syn::parse_quote!(target),
        items: 1,
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
fn row_stores_need_exact_and_segmented_or_distributed_reads_need_range() {
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
        Capability::Range
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
fn evidence_forms_round_trip_without_becoming_checked_facts() {
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
            key R [a];
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
fn grouped_fold_requires_complete_distinct_flat_valuations() {
    for body in [
        quote!(r = rows r; groups = group grouped r;),
        quote!(r = canon rows r; groups = map grouped r;),
        quote!(r = canon rows r; groups = group r r;),
        quote!(r = canon rows r; groups = group grouped tile 2 r;),
        quote!(r = canon rows r; groups = group grouped fold count r;),
        quote!(r = flatten canon tile 2 rows r; groups = group grouped r;),
    ] {
        reject(&grouped_program(quote!(k), body), "BE-Group");
    }
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
