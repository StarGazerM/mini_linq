use cq_lang_sequence::Program;
use cq_sequence_conformance::{Target, assert_conformant};

fn projected_host_reader() -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            consumer canonical;
            l = pipe rename L {a -> a};
            raw = pipe rename R {a -> a,b -> b};
            r = let project raw keep {a};
            joined = pipe probe first [a] l,r;
            return joined as answer(a);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    syn::parse_quote! {
        #structures
        sequence {
            l = rows l;
            raw = rows raw;
            r = store map r raw;
            joined = compact joined lookup r[a] l;
            output = store joined;
        }
    }
}

#[test]
fn host_only_provider_survives_an_unavailable_shareable_rewrite() {
    let source = projected_host_reader();
    let mut catalog = cq_sequence_tasks::catalog();
    catalog.providers.retain_mut(|token| {
        token.levels.retain(|level| level == &["host"]);
        !token.levels.is_empty()
    });
    let target = cq_sequence_tasks::select_providers(&source, &catalog).unwrap();
    cq_sequence_plan::decompose(&target, &cq_sequence_tasks::machine(), &catalog).unwrap();
    assert_eq!(source.structures.physical, target.structures.physical);
    let original: Vec<_> = source
        .structures
        .structures
        .entries
        .iter()
        .filter(|entry| entry.name == "r")
        .collect();
    let selected: Vec<_> = target
        .structures
        .structures
        .entries
        .iter()
        .filter(|entry| entry.name == "r")
        .collect();
    assert_eq!(original, selected);
    assert_conformant(&target, &[Target::Sequential, Target::Tasks]);
}

#[test]
fn inapplicable_retention_still_rejects_an_unadmitted_native_reader() {
    let source = projected_host_reader();
    let mut catalog = cq_sequence_tasks::catalog();
    catalog.providers.clear();
    assert!(cq_sequence_tasks::select_providers(&source, &catalog).is_err());
}

#[test]
fn positional_spine_preserves_raw_host_membership_at_host_when_retention_is_illegal() {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32); relation S(a:i32);
            r = pipe rename R {a -> a,b -> b};
            s = let rename S {a -> a};
            selected = let semi eager [a] r,s;
            expanded = pipe probe first [a] s,selected;
            member = pipe probe first [a,b] expanded,r;
            answer = let canon project member keep {a,b};
            return answer as answer(a,b);
        }
    };
    let structures =
        cq_structure_selection::compile_with(&physical, quote::quote!(positional(R, S))).unwrap();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    // Keep the authored reader publication separate from join-order advice.
    let source = cq_sequence_plan::rewrites::store(&source, quote::quote!(member)).unwrap();
    let before = cq_lang_sequence::contract::check(&source).unwrap();
    assert!(before.structures.reads.contains_key("selected"));
    assert!(
        before
            .structures
            .membership
            .contains(&("r".into(), vec!["a".into(), "b".into()]))
    );
    let catalog = cq_sequence_tasks::catalog();
    let target = cq_sequence_tasks::select_providers(&source, &catalog).unwrap();
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert!(facts.structures.host_read("r"));
    assert!(facts.structures.reads.contains_key("selected"));
    cq_lang_sequence::check_rewrite(&source, &target).unwrap();
    let planned =
        cq_sequence_plan::decompose(&target, &cq_sequence_tasks::machine(), &catalog).unwrap();
    let facts =
        cq_lang_sequence::check_grading(&planned.program, &cq_sequence_tasks::machine()).unwrap();
    assert_eq!(facts.levels["member"], ["host"]);
    assert!(matches!(
        facts.publications.get("s"),
        Some(cq_lang_sequence::Publication::BorrowHost { result, .. }) if result == "s"
    ));
    let expression = cq_sequence_tasks::translate(&planned.program, &facts).unwrap();
    let rendered = quote::ToTokens::to_token_stream(&expression).to_string();
    let keyset = rendered
        .find("let __cq_s_keys_a =")
        .expect("borrowed source keyset");
    let read = rendered
        .find("ExactMembership :: contains (__cq_s_keys_a")
        .expect("partition producer membership read");
    assert!(
        keyset < read,
        "publish keysets before dependent partition producers"
    );
    assert_eq!(rendered.matches("let __cq_s_keys_a =").count(), 1);
    assert!(
        !rendered.contains("let __cq_s_rows"),
        "borrowed rows need no tuple buffer"
    );
    // Each backend selects its own valid catalog; positional markers need
    // the caller's exact cq-load dependency in the generated-Rust runtime.
    let runtime = cq_testkit::rustc::Runtime::with_dependencies(vec![
        cq_testkit::rustc::linked_dependency::<cq_storage::sharded::FullKey>(
            "cq_storage",
            "cq_storage::sharded::FullKey",
        ),
        cq_testkit::rustc::linked_dependency::<cq_load::ColumnAt<0>>(
            "cq_load",
            "cq_load::ColumnAt<0>",
        ),
    ]);
    for setup in [
        quote::quote! { let R: Vec<(i32,i32)> = vec![]; let S: Vec<(i32,)> = vec![]; },
        quote::quote! { let R = vec![(1i32,2i32),(1,2),(2,3)]; let S = vec![(1i32,),(1,)]; },
    ] {
        cq_sequence_conformance::assert_conformant_with_runtime(
            &source,
            &[Target::Sequential, Target::Tasks],
            &setup,
            &runtime,
            &cq_lang_sequence::Catalog::default(),
        );
    }
}

#[test]
fn positional_selection_sharded_index_executes_exact_answers() {
    // Minimize retail_h07_block's positional filter with a whole-relation
    // consumer: zero-key probes retain a Selection, not a zero-column index.
    // This is the shape exposed by the introducing zero-key-read cutover.
    let physical = syn::parse_quote! {
        physical {
            relation Lineitem(order_key:i64,supplier_key:i64,ship_date:i32);
            relation Orders(order_key:i64,customer_key:i64);
            lineitem = pipe rename Lineitem {
                order_key -> order_key,supplier_key -> supplier_key,ship_date -> ship_date
            };
            orders = pipe rename Orders {
                order_key -> wanted_key,customer_key -> customer_key
            };
            selected = let range select lineitem if
                (ship_date >= 19950101, ship_date <= 19961231);
            expanded = pipe probe first [] orders,selected;
            joined = pipe guard expanded if (*order_key == *wanted_key);
            answer = let canon project joined keep {order_key,supplier_key,customer_key};
            return answer as answer(order_key,supplier_key,customer_key);
        }
    };
    run_positional_selection(physical);
}

#[test]
fn kp_1_kp_2_key_partitioned_positional_selection() {
    run_positional_selection(syn::parse_quote! {
        physical {
            relation Lineitem(order_key:i64,supplier_key:i64,ship_date:i32);
            relation Orders(order_key:i64,customer_key:i64);
            lineitem = pipe rename Lineitem {
                order_key -> order_key,supplier_key -> supplier_key,ship_date -> ship_date
            };
            orders = let rename Orders {
                order_key -> order_key,customer_key -> customer_key
            };
            selected = let range select lineitem if
                (ship_date >= 19950101, ship_date <= 19961231);
            expanded = pipe probe first [order_key] selected,orders;
            keys = pipe project expanded keep {order_key,customer_key};
            joined = pipe probe first [order_key] keys,selected;
            answer = let canon project joined keep {order_key,supplier_key,customer_key};
            return answer as answer(order_key,supplier_key,customer_key);
        }
    });
}

fn run_positional_selection(physical: cq_lang_sequence::structures::physical::Program) {
    let structures = cq_structure_selection::compile_with(
        &physical,
        quote::quote!(positional(Lineitem, Orders)),
    )
    .unwrap();
    let selection = structures
        .structures
        .entries
        .iter()
        .find(|entry| {
            entry.name == "selected"
                && matches!(
                    entry.structure,
                    cq_lang_sequence::structures::Structure::Selection { .. }
                )
        })
        .expect("fixture must retain the positional Selection")
        .clone();
    // Membership does not certify a position snapshot. The legality checker
    // must reject this authored substitution without rerunning selection.
    let mut invalid = structures.clone();
    let sharded_index =
        cq_lang_sequence::structures::provider::base::declaration("sharded_index");
    invalid.structures.providers.declare(&sharded_index);
    for entry in &mut invalid.structures.entries {
        if entry.name == "selected"
            && let cq_lang_sequence::structures::Structure::Selection { provider, .. } =
                &mut entry.structure
        {
            *provider = sharded_index.instance();
        }
    }
    let error = cq_lang_sequence::structures::contract::check(&invalid).unwrap_err();
    assert!(error.to_string().contains("STR-Positions"), "{error}");
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let catalog = cq_sequence_tasks::catalog();
    let mut missing_positions = catalog.clone();
    missing_positions.providers.retain(|entry| {
        !catalog.facts(&entry.provider).unwrap().reads.contains(
            &cq_lang_sequence::structures::provider::Capability::Positions,
        )
    });
    assert!(
        cq_sequence_plan::rewrites::select_from_catalog(&structures, &missing_positions).is_err(),
        "a catalog without positions cannot substitute an ordinary membership provider"
    );
    let target = cq_sequence_tasks::select_providers(&source, &catalog).unwrap();
    assert!(
        target.structures.structures.entries.contains(&selection),
        "the positional Selection must not be replaced with a membership-only sharded index"
    );
    cq_lang_sequence::check_rewrite(&source, &target).unwrap();
    assert!(
        cq_sequence_plan::rewrites::key_partition(
            &target,
            quote::quote!(selected, selected_parts, [order_key], 4),
        )
        .is_err(),
        "authored routing must preserve the native position carrier too"
    );
    let mut invalid = target.clone();
    let index = invalid.sequence.definitions.iter()
        .position(|definition| definition.name == "selected").unwrap();
    let original = invalid.sequence.definitions[index].term.clone();
    invalid.sequence.definitions.insert(index, syn::parse_quote! {
        selected_snapshot = #original;
    });
    invalid.sequence.definitions[index + 1].term = syn::parse_quote! {
        store partition_k [order_key] (4) selected_snapshot
    };
    assert!(cq_lang_sequence::contract::check(&invalid).is_err());
    let planned =
        cq_sequence_plan::decompose(&target, &cq_sequence_tasks::machine(), &catalog).unwrap();
    let facts =
        cq_lang_sequence::check_grading(&planned.program, &cq_sequence_tasks::machine()).unwrap();
    let expression = cq_sequence_tasks::translate(&planned.program, &facts).unwrap();
    let runtime = cq_testkit::rustc::Runtime::with_dependencies(vec![
        cq_testkit::rustc::linked_dependency::<rayon::ThreadPool>("rayon", "rayon::ThreadPool"),
        cq_testkit::rustc::linked_dependency::<cq_sequence_tasks::Config>(
            "cq_sequence_tasks",
            "cq_sequence_tasks::Config",
        ),
        cq_testkit::rustc::linked_dependency::<cq_storage::sharded::FullKey>(
            "cq_storage",
            "cq_storage::sharded::FullKey",
        ),
        cq_testkit::rustc::linked_dependency::<cq_load::ColumnAt<0>>(
            "cq_load",
            "cq_load::ColumnAt<0>",
        ),
    ]);
    runtime.run(
        &quote::quote! {
            #![allow(non_snake_case, unused_variables)]
            fn main() {
                // Keep excluded rows between selected positions, duplicate
                // witnesses, both inclusive boundaries, and a missing order.
                let populated = [
                    (10i64, 1i64, 19950101i32),
                    (11, 2, 19941231),
                    (12, 3, 19961231),
                    (10, 1, 19950101),
                    (13, 4, 19970101),
                    (14, 5, 19960101),
                    (15, 6, 19960101),
                ];
                let Orders = &[(10i64, 100i64), (11, 101), (12, 102), (13, 103), (14, 104)][..];
                for threads in [1, 4] {
                    let pool = rayon::ThreadPoolBuilder::new()
                        .num_threads(threads).build().unwrap();
                    for Lineitem in [&populated[..], &[][..]] {
                        let mut actual: Vec<_> = pool.install(|| {
                            (#expression).into_iter()
                                .map(|(order, supplier, customer)| (*order, *supplier, *customer))
                                .collect()
                        });
                        actual.sort();
                        let expected = if Lineitem.is_empty() {
                            vec![]
                        } else {
                            vec![(10, 1, 100), (12, 3, 102), (14, 5, 104)]
                        };
                        assert_eq!(actual, expected);
                    }
                }
            }
        }
        .to_string(),
    );
}

fn retained_positional_selection() -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i64,b:i64);
            raw = pipe rename R {a -> a,b -> b};
            selected = let range select raw if (a >= 1);
            answer = let canon project selected keep {a,b};
            return answer as answer(a,b);
        }
    };
    let structures =
        cq_structure_selection::compile_with(&physical, quote::quote!(positional(R))).unwrap();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let facts = cq_lang_sequence::contract::check(&source).unwrap();
    assert!(facts.structures.selections.contains_key("selected"));
    source
}

#[test]
fn positional_selection_keeps_its_recorded_constructor_through_tasks_catalog() {
    let source = retained_positional_selection();
    let before = cq_lang_sequence::contract::check(&source).unwrap();
    let recorded = &before.structures.selections["selected"].provider;
    let catalog = cq_sequence_tasks::catalog();
    let target = cq_sequence_tasks::select_providers(&source, &catalog).unwrap();
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert_eq!(&facts.structures.selections["selected"].provider, recorded);
    cq_lang_sequence::check_rewrite(&source, &target).unwrap();
    let planned =
        cq_sequence_plan::decompose(&target, &cq_sequence_tasks::machine(), &catalog).unwrap();
    let facts =
        cq_lang_sequence::check_grading(&planned.program, &cq_sequence_tasks::machine()).unwrap();
    let expression = cq_sequence_tasks::translate(&planned.program, &facts).unwrap();
    let rendered = quote::ToTokens::to_token_stream(&expression).to_string();
    let ty = &recorded.ty;
    let constructor = quote::quote!(<#ty>::from_positions).to_string();
    assert!(rendered.contains(&constructor), "{rendered}");
    assert_eq!(rendered.matches("from_positions").count(), 1);

    let runtime = cq_testkit::rustc::Runtime::with_dependencies(vec![
        cq_testkit::rustc::linked_dependency::<cq_storage::sharded::FullKey>(
            "cq_storage",
            "cq_storage::sharded::FullKey",
        ),
        cq_testkit::rustc::linked_dependency::<cq_load::ColumnAt<0>>(
            "cq_load",
            "cq_load::ColumnAt<0>",
        ),
    ]);
    for setup in [
        quote::quote! { let R: Vec<(i64,i64)> = vec![]; },
        quote::quote! { let R = vec![(-1i64,9i64),(1,2),(1,2),(2,3),(0,8)]; },
    ] {
        cq_sequence_conformance::assert_conformant_with_runtime(
            &source,
            &[Target::Sequential, Target::Tasks],
            &setup,
            &runtime,
            &cq_lang_sequence::Catalog::default(),
        );
    }
}

#[test]
fn membership_catalog_entry_cannot_replace_a_positional_constructor() {
    let source = retained_positional_selection();
    let facts = cq_lang_sequence::contract::check(&source).unwrap();
    let recorded = &facts.structures.selections["selected"].provider;
    let mut catalog = cq_sequence_tasks::catalog();
    catalog.providers.retain(|entry| !entry.covers(recorded));
    let error = cq_sequence_tasks::select_providers(&source, &catalog).unwrap_err();
    assert!(error.to_string().contains("positional selection constructor"));
    let error =
        cq_sequence_plan::decompose(&source, &cq_sequence_tasks::machine(), &catalog).unwrap_err();
    assert!(error.to_string().contains("positional selection constructor"));
}
