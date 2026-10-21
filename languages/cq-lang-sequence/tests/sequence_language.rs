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
        children,
    }
}

fn sequential() -> Machine {
    Machine {
        name: syn::parse_quote!(target),
        root: node("host", false, false, vec![]),
    }
}

fn gpu() -> Machine {
    Machine {
        name: syn::parse_quote!(target),
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
    let p = lookup_program(quote!(a = store flatten lookup r [a] rows l;));
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
fn canon_rejects_local_support_without_flatten() {
    reject(&program(quote!(a = canon tile 8 rows r;)), "BE-Canon");
}

#[test]
fn length_rejects_unknown_store_without_capacity() {
    reject(
        &lookup_program(quote!(a = store lookup r [a] rows l;)),
        "BE-Length",
    );
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
    reject(&program(quote!(a = store #body;)), "BE-Length");
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
fn boundary_rejects_unnamed_nested_separated_store() {
    let mut p = program(quote!(a = store store rows r;));
    grade(&mut p, quote!(a at device;));
    reject_grade(&p, &gpu(), "BE-Boundary");
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
        } sequence { a = store rows b; }
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
        } sequence { a = flatten lookup selected [a] rows l; }
    };
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
