//! Structures → Sequence → Rust preserves authored answers and storage choices.
//! Generated programs are checked against independent CQ Set oracles and
//! explicit expected rows, including head order and shared storage reads.
use cq_lang_structures as structures;
use cq_testkit::{cases, corpus as scenarios};
use quote::{ToTokens, quote};

fn text(tokens: impl ToTokens) -> String {
    tokens.to_token_stream().to_string()
}

fn sequence_rows(module: &structures::Program) -> syn::Result<syn::Expr> {
    let program = cq_sequence_lowering::compile(module)?;
    let analysis = cq_lang_sequence::check_grading(&program, &cq_sequence_pull::machine())?;
    let expression = cq_sequence_pull::translate(&program, &analysis)?;
    Ok(syn::parse_quote!((#expression).into_iter()))
}

fn program(module: &structures::Program) -> String {
    let complete = cq_structure_selection::compile(&module.physical).unwrap();
    let expr = sequence_rows(&complete).unwrap();
    text(expr)
}

#[test]
fn recorded_statistic_reads_use_completed_provider_and_missing_value_fallback() {
    use cq_lang_sequence::{Guard, Term};
    let physical = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> a};
        r = let rename R@1 {a -> a,b -> b};
        j = let canon probe first [a] l,r;
        return j as answer(a,b);
    });
    let mut selected = cq_structure_selection::compile(&physical).unwrap();
    let keyed = selected
        .structures
        .entries
        .iter()
        .find_map(|entry| match &entry.structure {
            structures::Structure::Built { keyed, .. } if entry.name == "r" => Some(keyed.clone()),
            _ => None,
        })
        .unwrap();
    let token = &keyed.provider.token;
    let layout = &keyed.levels;
    let statistic: structures::stats::StatDeclaration = syn::parse_quote!(
        count: rows occurrences estimate run_finalize rows_stat host optional build single
    );
    selected
        .structures
        .providers
        .declarations
        .iter_mut()
        .find(|declaration| declaration.token == *token)
        .unwrap()
        .facts
        .statistics
        .push(statistic.clone());
    let publication: structures::stats::Publication = syn::parse_quote!(
        rows_r = build r provider #token layout #(#layout)* columns [] domains [] stat #statistic;
    );
    selected.structures.publications.0.push(publication.clone());
    structures::stats::record_constructions(&mut selected).unwrap();
    let mut program = cq_sequence_lowering::compile(&selected).unwrap();
    let reference = publication.reference();
    let guard: Guard = syn::parse_quote!(stat r (#reference) < 10);
    let last = program.sequence.definitions.last_mut().unwrap();
    let original = last.term.clone();
    last.term = syn::parse_quote!(flatten choose [
        #guard => tile 1 #original,
        else => tile 2 #original
    ]);
    let analysis = cq_lang_sequence::check_grading(&program, &cq_sequence_pull::machine()).unwrap();
    let expression = cq_sequence_pull::translate_with(
        &program,
        &analysis,
        syn::parse_quote!(1usize),
        cq_sequence_pull::CanonTarget::Sequential,
        |term, _, _, output, _, _| {
            if let Term::Tile { n, .. } = term {
                let rows = &output.rows;
                output.rows = quote!({ selected_cover.set(#n); #rows });
            }
            Ok(())
        },
    )
    .unwrap();
    let printed = expression.to_token_stream().to_string();
    assert!(
        printed.contains("cq_storage :: sequence :: Choice :: Left"),
        "{printed}"
    );
    assert!(
        printed.contains("cq_storage :: sequence :: Choice :: Right"),
        "{printed}"
    );
    let source = quote! {
        use std::cell::Cell;
        thread_local! {
            static STAT: Cell<Option<u64>> = const { Cell::new(None) };
            static READS: Cell<usize> = const { Cell::new(0) };
        }
        trait PublishedRows {
            fn rows_stat(&self) -> Option<u64> {
                READS.with(|count| count.set(count.get() + 1));
                STAT.with(Cell::get)
            }
        }
        impl<T> PublishedRows for T {}
        fn main() {
            let L = [(1i32,), (2,), (2,)];
            let R = [(1i32, 4i32), (2, 5), (2, 5)];
            for (value, expected_cover) in [(None, 2), (Some(0), 1), (Some(9), 1), (Some(10), 2), (Some(u64::MAX), 2)] {
                STAT.with(|stat| stat.set(value));
                READS.with(|count| count.set(0));
                let selected_cover = Cell::new(0usize);
                let mut rows: Vec<_> = (#expression).into_iter().map(|(a,b)| (*a,*b)).collect();
                rows.sort();
                assert_eq!(rows, vec![(1,4), (2,5)]);
                assert_eq!(selected_cover.get(), expected_cover);
                assert_eq!(READS.with(Cell::get), 1);
            }
        }
    };
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}

#[test]
fn selected_construction_publishes_real_rows_and_group_maxima() {
    use cq_lang_sequence::{Guard, Term};
    for field in [structures::stats::Field::Rows, structures::stats::Field::MaxGroup] {
        let physical = syn::parse_quote!(physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = let canon probe first [a] l,r;
            return j as answer(a,b);
        });
        let selected = cq_structure_selection::compile(&physical).unwrap();
        let mut selected = cq_structure_selection::retype(&selected, quote!(
            r => @sorted_run as ::cq_storage::SortedRun<_, _> supports [keys]
        )).unwrap();
        let declaration = selected.structures.providers.declarations.iter()
            .find(|declaration| declaration.token == "sorted_run").unwrap();
        let statistic = declaration.facts.statistics.iter()
            .find(|statistic| statistic.field == field).unwrap();
        let projection = if field == structures::stats::Field::Rows {
            quote!(columns [] domains [])
        } else {
            quote!(columns [a] domains [a: identity integers 0])
        };
        let publication: structures::stats::Publication = syn::parse_quote!(
            selected_stat = build r provider sorted_run layout [a]
                #projection stat #statistic;
        );
        selected.structures.publications.0.push(publication.clone());
        structures::stats::record_constructions(&mut selected).unwrap();
        let mut program = cq_sequence_lowering::compile(&selected).unwrap();
        let reference = publication.reference();
        let guard: Guard = syn::parse_quote!(stat r (#reference) < 3);
        let last = program.sequence.definitions.last_mut().unwrap();
        let original = last.term.clone();
        last.term = syn::parse_quote!(flatten choose [
            #guard => tile 1 #original,
            else => tile 2 #original
        ]);
        let facts = cq_lang_sequence::check_grading(&program, &cq_sequence_pull::machine()).unwrap();
        let expression = cq_sequence_pull::translate_with(
            &program, &facts, syn::parse_quote!(1usize),
            cq_sequence_pull::CanonTarget::Sequential,
            |term, _, _, output, _, _| {
                if let Term::Tile { n, .. } = term {
                    let rows = &output.rows;
                    output.rows = quote!({ selected_cover.set(#n); #rows });
                }
                Ok(())
            },
        ).unwrap();
        let populated_cover = if field == structures::stats::Field::Rows { 2usize } else { 1usize };
        let source = quote! {
            fn main() {
                let L = [(1i32,), (2,)];
                let populated = [(1i32, 4i32), (1, 4), (2, 5)];
                for R in [&populated[..], &[][..]] {
                    let selected_cover = std::cell::Cell::new(0usize);
                    let mut rows: Vec<_> = (#expression).into_iter()
                        .map(|(a,b)| (*a,*b)).collect();
                    rows.sort();
                    assert_eq!(rows, if R.is_empty() { vec![] } else { vec![(1,4), (2,5)] });
                    assert_eq!(selected_cover.get(), if R.is_empty() { 1 } else { #populated_cover });
                }
            }
        };
        let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../runtime/cq-storage/src/lib.rs");
        cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
    }
}

#[test]
fn ordered_merge_uses_selected_fallback_and_rejects_authored_native() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32) host [a] reads [ordered];
            relation R(a:i32,b:i32) host [a] reads [ordered];
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,b -> b};
            j = let canon merge [a] l,r;
            return j as answer(a,b);
        }
    };
    let mut selected = cq_structure_selection::compile(&physical).unwrap();
    let analysis = structures::contract::check(&selected).unwrap();
    for result in ["l", "r"] {
        let key = (
            result.into(),
            structures::ReaderRef::Result("j".into()),
            vec!["a".into()],
        );
        assert!(analysis.fallbacks.contains_key(&key));
        assert!(!analysis.natives.contains_key(&key));
    }
    sequence_rows(&selected).unwrap();
    let mut replaced = 0;
    for entry in &mut selected.structures.entries {
        if matches!(&entry.structure,
            structures::Structure::Host { reader: structures::ReaderRef::Result(reader), .. }
                if reader == "j")
        {
            let name = &entry.name;
            *entry = syn::parse_quote!(#name native result(j) = ordered [a];);
            replaced += 1;
        }
    }
    assert_eq!(replaced, 2);
    selected.structures.declare(&[]).unwrap();
    structures::contract::check(&selected).unwrap();
    assert!(sequence_rows(&selected).is_err());
}

#[test]
fn zero_heading_exists_can_be_scanned_and_probed_against_itself() {
    let physical = syn::parse_quote!(physical {
        relation rows(a:i32);
        r = pipe rename rows@0 {a -> a};
        p = exists project r keep {};
        out = let canon semi eager [] p,p;
        return out as answer();
    });
    let selected = cq_structure_selection::compile(&physical).unwrap();
    let expression = sequence_rows(&selected).unwrap();
    let source = quote!(
        fn main() {
            for (rows, expected) in [
                (&[(1i32,), (1,), (2,)][..], vec![()]),
                (&[][..], vec![]),
            ] {
                let actual: Vec<()> = (#expression).collect();
                assert_eq!(actual, expected);
            }
        }
    );
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}

#[test]
fn equal_support_choice_reheads_before_a_downstream_projection() {
    let physical = syn::parse_quote!(physical {
        relation keys(a:i32); relation values(a:i32,b:i32);
        r = let rename keys@0 {a -> a};
        s = let rename values@1 {a -> a,b -> b};
        filtered = pipe semi eager [a] r,s;
        yes = pipe probe first [a] filtered,s;
        no = pipe probe first [a] r,s;
        combined = pipe union yes,no;
        observed = pipe guard combined if ({ visits.set(visits.get() + 1); true });
        out = let canon project observed keep {b};
        return out as answer(b);
    });
    let selected = cq_structure_selection::compile(&physical).unwrap();
    let source = cq_sequence_lowering::compile(&selected).unwrap();
    let mut target = source.clone();
    target
        .sequence
        .definitions
        .iter_mut()
        .find(|d| d.name == "combined")
        .unwrap()
        .term = cq_lang_sequence::Term::Choose(vec![
        (
            cq_lang_sequence::Guard::Lt(cq_lang_sequence::Len::Input("s".into()), 2),
            cq_lang_sequence::Term::Ref(syn::parse_quote!(yes)),
        ),
        (
            cq_lang_sequence::Guard::Else,
            cq_lang_sequence::Term::Ref(syn::parse_quote!(no)),
        ),
    ]);
    cq_lang_sequence::check_rewrite(&source, &target).unwrap();
    let analysis = cq_lang_sequence::contract::check(&target).unwrap();
    assert_eq!(analysis.results["combined"], "combined");
    let expression = cq_sequence_pull::translate(&target, &analysis).unwrap();
    let source = quote!(
        fn main() {
            let keys = &[(1i32,), (2,)][..];
            let values = &[(1i32, 10i32), (2, 20)][..];
            let visits = &::std::cell::Cell::new(0usize);
            let mut actual: Vec<(i32,)> = (#expression).map(|(b,)| (*b,)).collect();
            actual.sort();
            // Canonical output cannot expose duplicate evaluation, so count the
            // pinned downstream guard as well as checking the projected values.
            assert_eq!(visits.get(), 2);
            assert_eq!(actual, vec![(10,), (20,)]);
        }
    );
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}

#[test]
fn attached_sidecar_and_index_survive_the_selected_lazy_choice_arm() {
    let physical = syn::parse_quote!(physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        s = pipe range select r if (z > 1);
        b = let project s keep {x,z};
        j = pipe probe first [x] l,b;
        out = let canon project j keep {x,y,z};
        return out as answer(x,y,z);
    });
    let mut selected = cq_structure_selection::compile(&physical).unwrap();
    let declaration = structures::provider::base::declaration("attached_join_filter_membership");
    let provider = declaration.instance();
    selected.structures.providers.declare(&declaration);
    selected.structures.entries.push(syn::parse_quote!(
        l join_filter = #provider build b [x] covering j marker ::cq_load::ColumnAt<0>;
    ));
    structures::record_join_filter_abandon(&mut selected).unwrap();
    let facts = structures::contract::check(&selected).unwrap();
    assert!(facts.join_filters["l"][0].abandon.is_none());
    let mut context = cq_print::Context::new(&selected, &facts);
    context.register("b");
    context.begin_deferred_bindings();
    let mut arms = Vec::new();
    for _ in 0..2 {
        let start = context.statements.len();
        let built = context.materialize("b", quote!({
            builds.set(builds.get() + 1);
            R.iter().filter(|row| row.1 > 1).map(|(x,z)| (x,z))
        }));
        let cq_print::Repr::Let { structures, .. } = built else {
            panic!("fixture requires ordinary payload storage");
        };
        let index = &structures[0].1;
        let filter = context.join_filter_ident("l", 0);
        let statements = context.statements.split_off(start);
        arms.push(quote!({ #(#statements)* (#index, #filter) }));
    }
    context.end_deferred_bindings();
    let statements = &context.statements;
    let left = &arms[0];
    let right = &arms[1];
    let source = quote! {
        fn main() {
            let L = [(1i64,10i64), (2,20), (8,80)];
            let R = [(1i64,2i64), (1,3), (2,4), (8,0)];
            for select_left in [true, false] {
                let builds = std::cell::Cell::new(0usize);
                #(#statements)*
                let (index, filter) = if select_left { #left } else { #right };
                assert_eq!(builds.get(), 1);
                assert!(!cq_storage::ReadPredicate::test(filter, &8i64));
                let mut actual = Vec::new();
                for (x,y) in &L {
                    if cq_storage::ReadPredicate::test(filter, x) {
                        for (z,) in cq_storage::IndexRead::lookup(index, &(x,)) {
                            actual.push((*x,*y,*z));
                        }
                    }
                }
                actual.sort();
                assert_eq!(actual, vec![(1,10,2), (1,10,3), (2,20,4)]);
            }
        }
    };
    cq_testkit::rustc::Runtime::with_dependencies(vec![
        cq_testkit::rustc::linked_dependency::<cq_storage::sharded::FullKey>(
            "cq_storage", "cq_storage::sharded::FullKey",
        ),
        cq_testkit::rustc::linked_dependency::<cq_load::ColumnAt<0>>(
            "cq_load", "cq_load::ColumnAt<0>",
        ),
    ]).run(&source.to_string());
}

#[test]
fn worker_guards_use_serial_or_explicit_target_worker_counts() {
    use cq_lang_sequence::Term;
    let physical = syn::parse_quote!(physical {
        relation input(a:i32);
        r = let canon rename input@0 {a -> a};
        return r as answer(a);
    });
    let selected = cq_structure_selection::compile(&physical).unwrap();
    let mut program = cq_sequence_lowering::compile(&selected).unwrap();
    program.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(
        store flatten choose [
            workers < 2 => tile 1 rows r,
            else => tile 2 rows r
        ]
    );
    let analysis = cq_lang_sequence::check_grading(&program, &cq_sequence_pull::machine()).unwrap();
    let serial = cq_sequence_pull::translate(&program, &analysis).unwrap();
    assert!(text(&serial).contains(&text(quote!((1usize) < 2usize))));
    let target = cq_sequence_pull::translate_with(
        &program,
        &analysis,
        syn::parse_quote!(available_workers),
        cq_sequence_pull::CanonTarget::Sequential,
        |term, _, _, output, _, _| {
            if let Term::Tile { n, .. } = term {
                let rows = &output.rows;
                output.rows = quote!({ selected_cover.set(#n); #rows });
            }
            Ok(())
        },
    )
    .unwrap();
    let profiled = cq_sequence_pull::translate_profile_with(
        &program,
        &analysis,
        syn::parse_quote!(available_workers),
        cq_sequence_pull::CanonTarget::Sequential,
        |_, _, _, _, _, _| Ok(()),
    )
    .unwrap();
    assert!(text(profiled).contains(&text(quote!((available_workers) < 2usize))));
    let source = quote!(
        fn main() {
            for input in [&[][..], &[(1i32,), (2,), (3,)][..]] {
                let serial_rows: Vec<_> = (#serial).into_iter().map(|(a,)| *a).collect();
                assert_eq!(serial_rows, input.iter().map(|(a,)| *a).collect::<Vec<_>>());
                for available_workers in [1usize, 2, 8] {
                    let selected_cover = ::std::cell::Cell::new(0usize);
                    let rows: Vec<_> = (#target).into_iter().map(|(a,)| *a).collect();
                    assert_eq!(rows, serial_rows);
                    assert_eq!(selected_cover.get(), if available_workers < 2 { 1 } else { 2 });
                }
            }
        }
    );
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}

#[test]
fn projected_membership_probes_keep_flat_rows_inside_outer_tiles() {
    use cq_lang_sequence::Term;
    let mut checks = Vec::new();
    for (producer, selected) in [
        (
            quote!(r = let rename right@1 {a -> a,unused -> unused};),
            false,
        ),
        (
            quote!(
                raw = pipe rename right@1 {a -> a,unused -> unused};
                r = let select raw if (unused = 7);
            ),
            true,
        ),
        (
            quote!(
                first = pipe rename right@1 {a -> a,unused -> unused};
                second = pipe rename more@2 {a -> a,unused -> unused};
                r = let union first,second;
            ),
            false,
        ),
    ] {
        for tiled in [false, true] {
            let physical = syn::parse_quote! {
                physical {
                    relation left(a:i32,b:i32); relation right(a:i32,unused:i32);
                    relation more(a:i32,unused:i32); relation payload(b:i32,c:i32);
                    l = pipe rename left@0 {a -> a,b -> b};
                    #producer
                    t = let rename payload@3 {b -> b,c -> c};
                    member = pipe probe first [a] l,r;
                    expanded = pipe probe first [b] member,t;
                    out = let canon project expanded keep {a,b,c};
                    return out as answer(a,b,c);
                }
            };
            let mut structures = cq_structure_selection::compile(&physical).unwrap();
            if tiled {
                // The explicit outer tile materializes expansion offsets.
                // Record a range-capable provider before lowering that shape.
                for entry in &mut structures.structures.entries {
                    if entry.name == "t"
                        && let structures::Structure::Built { keyed, .. } = &mut entry.structure
                    {
                        keyed.provider = cq_lang_structures::provider::base::instance(
                            "sorted_run",
                            syn::parse_quote!(::cq_storage::SortedRun<_, _>),
                        );
                    }
                }
                structures
                    .structures
                    .declare(&[cq_lang_structures::provider::base::declaration(
                        "sorted_run",
                    )])
                    .unwrap();
            }
            let mut program = cq_sequence_lowering::compile(&structures).unwrap();
            if tiled {
                program.sequence.definitions[0].term = syn::parse_quote!(tile 2 rows l);
            }
            let analysis =
                cq_lang_sequence::check_grading(&program, &cq_sequence_pull::machine()).unwrap();
            assert!(
                analysis
                    .structures
                    .membership
                    .contains(&("r".into(), vec!["a".into()]))
            );
            let expression = cq_sequence_pull::translate_with(
                &program,
                &analysis,
                syn::parse_quote!(1usize),
                cq_sequence_pull::CanonTarget::Sequential,
                |term, _, input, output, _, _| {
                    if matches!(term, Term::Compact(name, _) if name == "member") {
                        assert_eq!(output.depth, input.unwrap().depth);
                    }
                    Ok(())
                },
            )
            .unwrap();
            checks.push(quote! {
                for empty in [false, true] {
                    let left: &[(i32,i32)] = &[(1,10),(1,10),(2,20),(3,30),(4,40)];
                    let right: &[(i32,i32)] = if empty { &[] } else { &[(1,7),(1,7),(2,8),(3,7)] };
                    let more: &[(i32,i32)] = if empty { &[] } else { &[(2,9),(5,7)] };
                    let payload: &[(i32,i32)] = &[(10,100),(20,200),(30,300),(40,400)];
                    let actual: ::std::collections::BTreeSet<_> = (#expression)
                        .into_iter().map(|(a,b,c)| (*a,*b,*c)).collect();
                    let expected = if empty {
                        vec![]
                    } else if #selected {
                        vec![(1,10,100),(3,30,300)]
                    } else {
                        vec![(1,10,100),(2,20,200),(3,30,300)]
                    };
                    assert_eq!(actual, expected.into_iter().collect());
                }
            });
        }
    }
    let source = quote!(fn main() { #(#checks)* });
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}

#[test]
fn recorded_hash_types_print_verbatim_including_explicit_standard_hashers() {
    use structures::Structure;
    // Spellings are opaque: the recorded `hash_set` tokens stay, only their
    // Rust text changes, and composites re-spell around the new inner text.
    fn respell(provider: &mut structures::Provider, ty: &syn::Type) -> bool {
        if provider.token == "hash_set" {
            provider.ty = ty.clone();
            return true;
        }
        let Some(inner) = &mut provider.inner else {
            return false;
        };
        if respell(inner, ty) {
            *provider =
                cq_lang_structures::provider::base::declaration(&provider.token.to_string())
                    .compose((**inner).clone())
                    .unwrap();
            return true;
        }
        false
    }
    let fast: syn::Type = syn::parse_quote!(::cq_storage::HashSet<_>);
    let standard: syn::Type = syn::parse_quote!(::std::collections::HashSet<_>);
    let explicit: syn::Type = syn::parse_quote!(
        ::std::collections::HashSet<_, ::std::collections::hash_map::RandomState>
    );
    for case in scenarios::SCENARIOS {
        let source = case.parse_physical().unwrap();
        let selected = cq_structure_selection::compile(&source).unwrap();
        assert!(!text(&selected.structures).contains(&text(&standard)));
        for ty in [&fast, &standard, &explicit] {
            let mut recorded = selected.clone();
            for entry in &mut recorded.structures.entries {
                let provider = match &mut entry.structure {
                    Structure::Built { keyed, .. } | Structure::Host { keyed, .. } => {
                        &mut keyed.provider
                    }
                    Structure::Rows { provider, .. }
                    | Structure::Group { provider, .. }
                    | Structure::Selection { provider, .. }
                    | Structure::KeySet { provider, .. } => provider,
                    Structure::Read { .. }
                    | Structure::Dedup { .. }
                    | Structure::HeadDedup { .. }
                    | Structure::JoinFilter { .. }
                    | Structure::Repr { .. }
                    | Structure::Stage { .. }
                    | Structure::Share { .. }
                    | Structure::Native { .. } => continue,
                };
                respell(provider, ty);
            }
            structures::check_rewrite(&selected, &recorded).unwrap();
            let printed = text(sequence_rows(&recorded).unwrap());
            if text(&recorded.structures).contains(&text(ty)) {
                assert!(printed.contains(&text(ty)), "{}: {printed}", case.name);
            }
            if ty != &fast {
                assert!(!printed.contains(&text(&fast)), "{}: {printed}", case.name);
            }
        }
    }
}

/// Default selection and printed Rust must agree with an independent CQ Set
/// oracle for every host/retained orientation, including shared storage reads.
fn mixed_smallest_checks() -> Vec<proc_macro2::TokenStream> {
    let query: cq_testkit::cq::Program = syn::parse_quote!(
        struct Mixed;
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        answer(a,b,c) :- L(a,b), R(b,c), S(a,c);
    );
    let mut inputs = vec![[
        vec![
            vec![1, 1],
            vec![1, 1],
            vec![2, 2],
            vec![3, 3],
            vec![4, 4],
            vec![5, 5],
            vec![6, 6],
        ],
        vec![
            vec![1, 10],
            vec![1, 11],
            vec![1, 12],
            vec![1, 12],
            vec![2, 20],
            vec![3, 30],
            vec![3, 31],
            vec![4, 40],
            vec![6, 60],
        ],
        vec![
            vec![1, 10],
            vec![2, 20],
            vec![2, 21],
            vec![2, 21],
            vec![3, 32],
            vec![5, 50],
            vec![6, 60],
        ],
    ]];
    for seed in 0..24 {
        let database = cq_testkit::databases::binary_database(&query, seed);
        inputs.push(["L", "R", "S"].map(|name| {
            let mut rows: Vec<_> = database[name].iter().cloned().collect();
            if let Some(first) = rows.first().cloned() {
                rows.push(first);
            }
            rows
        }));
    }
    let fixtures: Vec<_> = inputs
        .into_iter()
        .map(|inputs| {
            let database = ["L", "R", "S"]
                .into_iter()
                .zip(&inputs)
                .map(|(name, rows)| (name.to_owned(), rows.iter().cloned().collect()))
                .collect();
            let expected = cq_testkit::cq_semantics::evaluate_cq(&query, &database);
            (inputs, expected)
        })
        .collect();
    assert!(!fixtures[0].1.is_empty());

    let mut programs = Vec::new();
    for (r_retained, s_retained) in [(false, true), (true, false), (true, true), (false, false)] {
        let r = if r_retained {
            "r0 = pipe rename R@0 {b -> b,c -> c}; r = let project r0 keep {b,c};"
        } else {
            "r = pipe rename R@0 {b -> b,c -> c};"
        };
        let s = if s_retained {
            "s0 = pipe rename S@0 {a -> a,c -> c}; s = let project s0 keep {a,c};"
        } else {
            "s = pipe rename S@0 {a -> a,c -> c};"
        };
        let physical: structures::physical::Program = syn::parse_str(&format!(
            "physical {{
                relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
                l = pipe rename L@0 {{a -> a,b -> b}}; {r} {s}
                candidate = pipe probe smallest [b] l,r;
                checked = let canon semi eager [a,c] candidate,s;
                return checked as answer(a,b,c);
            }}"
        ))
        .unwrap();
        programs.push((format!("retained R={r_retained}, S={s_retained}"), physical));
    }
    programs.push((
        "shared payload, membership and ordered reads".into(),
        syn::parse_quote!(physical {
            relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
            l0 = pipe rename L@0 {a -> a,b -> b};
            r0 = pipe rename R@1 {b -> b,c -> c}; r = let project r0 keep {b,c};
            s = pipe rename S@2 {a -> a,c -> c};
            payload = pipe probe first [b] l0,r;
            l1 = pipe project payload keep {a,b};
            members = let semi eager [c] s,r;
            l2 = pipe semi eager [a] l1,members;
            ordered = let merge[b,c] r,r;
            l = pipe semi eager [b] l2,ordered;
            candidate = pipe probe smallest [b] l,r;
            checked = let canon semi eager [a,c] candidate,s;
            return checked as answer(a,b,c);
        }),
    ));

    let mut checks = Vec::new();
    for (label, physical) in programs {
        let target = cq_structure_selection::compile(&physical).unwrap();
        let mut reversed = target.clone();
        reversed.structures.entries.reverse();
        for target in [target, reversed] {
            let expression = sequence_rows(&target).unwrap();
            let emitted = text(&expression);
            assert!(
                emitted.contains("__cq_choose_a"),
                "binary emission keeps its existing selector"
            );
            assert!(
                !emitted.contains("__cq_choice"),
                "k-way selector must not change binary plans"
            );
            assert!(
                !emitted.contains("if true &&"),
                "binary membership has no added conjunction"
            );
            let assertions = fixtures.iter().enumerate().map(|(case, (inputs, expected))| {
                    let bindings = ["L", "R", "S"].into_iter().zip(inputs).map(|(name, rows)| {
                        let name = syn::Ident::new(name, proc_macro2::Span::call_site());
                        let rows = rows.iter().map(|row| quote!((#(#row,)*)));
                        quote!(let #name: &[(i32, i32)] = &[#(#rows),*];)
                    });
                    let expected = expected.iter().map(|row| quote!((#(#row,)*)));
                    quote!({
                        #(#bindings)*
                        let expected: ::std::collections::BTreeSet<_> = [#(#expected),*].into_iter().collect();
                        assert_eq!(run(L, R, S), expected, "{} case {}", #label, #case);
                    })
                });
            checks.push(quote!({
                    fn run(L: &[(i32, i32)], R: &[(i32, i32)], S: &[(i32, i32)]) -> ::std::collections::BTreeSet<(i32, i32, i32)> {
                        (#expression).map(|(a, b, c)| (*a, *b, *c)).collect()
                    }
                    #(#assertions)*
                }));
        }
    }
    checks
}

fn filtered_checks() -> Vec<proc_macro2::TokenStream> {
    let mut checks = Vec::new();
    for (selection, kept, expected) in [
        (
            "point select r if (a = 7)",
            "a,k,v",
            quote!([(7, 1, 10), (7, 2, 30)]),
        ),
        (
            "point select r if (a = @(chosen))",
            "a,k,v",
            quote!([(7, 1, 10), (7, 2, 30)]),
        ),
        (
            "point select r if (a = @(make_chosen()))",
            "a,k,v",
            quote!([(7, 1, 10), (7, 2, 30)]),
        ),
        (
            "point select r if (a in {7})",
            "a,k,v",
            quote!([(7, 1, 10), (7, 2, 30)]),
        ),
        (
            "point select r if (a in {7,8})",
            "a,k,v",
            quote!([(7, 1, 10), (8, 1, 20), (7, 2, 30)]),
        ),
        ("point select r if (a = 7, a = 8)", "a,k,v", quote!([])),
        (
            "range select r if (v > 15)",
            "a,k,v",
            quote!([(8, 1, 20), (7, 2, 30)]),
        ),
    ] {
        for stream in [false, true] {
            for sorted in [false, true] {
                let rename = if stream {
                    "r0 = pipe rename R@1 {a -> a,k -> k,v -> v,dead -> dead}; r = pipe project r0 keep {a,k,v,dead};"
                } else {
                    "r = pipe rename R@1 {a -> a,k -> k,v -> v,dead -> dead};"
                };
                let source: structures::physical::Program = syn::parse_str(&format!(
                    "physical {{
                    relation L(k:i32); relation R(a:i32,k:i32,v:i32,dead:i32);
                    l = pipe rename L@0 {{k -> k}}; {rename}
                    f = let {selection}; j = pipe probe first [k] l,f;
                    keys = pipe project j keep {{k}};
                    members = pipe semi eager [k] keys,f;
                    shared = pipe probe first [k] members,f;
                    out = let canon project shared keep {{{kept}}}; return out as answer(a,k,v);
                }}"
                ))
                .unwrap();
                let mut target = cq_structure_selection::compile(&source).unwrap();
                assert!(
                    structures::contract::check(&target)
                        .unwrap()
                        .filtered
                        .contains_key("f")
                );
                if sorted {
                    for entry in &mut target.structures.entries {
                        if entry.name == "f" {
                            if let structures::Structure::Built { keyed, .. } = &mut entry.structure
                            {
                                keyed.provider = cq_lang_structures::provider::base::instance(
                                    "btree_map_grouped",
                                    syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>),
                                );
                            }
                        }
                    }
                    target
                        .structures
                        .declare(&[cq_lang_structures::provider::base::declaration(
                            "btree_map_grouped",
                        )])
                        .unwrap();
                }
                let expression = sequence_rows(&target).unwrap();
                assert!(!text(&expression).contains("__cq_f_rows"));
                checks.push(quote!({
                    let L: &[(i32,)] = &[(1,), (1,), (2,), (9,)];
                    let R: &[(i32,i32,i32,i32)] = &[(7,1,10,999), (7,1,10,999), (8,1,20,888), (7,2,30,777)];
                    let chosen = 7;
                    fn make_chosen() -> i32 { 7 }
                    let actual: ::std::collections::BTreeSet<(i32,i32,i32)> = (#expression).map(|(a,k,v)| (*a,*k,*v)).collect();
                    let expected: ::std::collections::BTreeSet<(i32,i32,i32)> = #expected.into_iter().collect();
                    assert_eq!(actual, expected, "{}", #selection);
                }));
            }
        }
    }
    for key in ["a", "a,k"] {
        let source: structures::physical::Program = syn::parse_str(&format!(
            "physical {{
            relation L(a:i32,k:i32); relation R(a:i32,k:i32);
            l = pipe rename L@0 {{a -> a,k -> k}}; r = pipe rename R@1 {{a -> a,k -> k}};
            f = let point select r if (a = 7);
            j = pipe probe first [{key}] l,f;
            out = let canon semi eager [{key}] j,f; return out as answer(a,k);
        }}"
        ))
        .unwrap_or_else(|e| panic!("{e}"));
        // For a unary key the two relations must share only that attribute.
        let source = if key == "a" {
            syn::parse_quote!(physical {
                relation L(a:i32); relation R(a:i32,k:i32);
                l = pipe rename L@0 {a -> a}; r = pipe rename R@1 {a -> a,k -> k};
                f = let point select r if (a = 7);
                j = pipe probe first [a] l,f;
                out = let canon project j keep {a,k}; return out as answer(a,k);
            })
        } else {
            source
        };
        let target = cq_structure_selection::compile(&source).unwrap();
        let expression = sequence_rows(&target).unwrap();
        let input = if key == "a" {
            quote!(let L: &[(i32,)] = &[(7,), (8,)];)
        } else {
            quote!(let L: &[(i32,i32)] = &[(7,1), (8,1), (7,2)];)
        };
        checks.push(quote!({
            #input
            let R: &[(i32,i32)] = &[(7,1), (7,2), (8,1)];
            let actual: ::std::collections::BTreeSet<_> = (#expression).map(|(a,k)| (*a,*k)).collect();
            assert_eq!(actual, [(7,1), (7,2)].into_iter().collect());
        }));
    }
    let source: structures::physical::Program = syn::parse_quote!(physical {
        relation L(k:i32); relation R(tag:u32,k:i32);
        l = pipe rename L@0 {k -> k}; r = pipe rename R@1 {tag -> tag,k -> k};
        f = let point select r if (tag = 0u32);
        j = let canon probe first [k] l,f; return j as answer(k,tag);
    });
    let expression = sequence_rows(&cq_structure_selection::compile(&source).unwrap()).unwrap();
    checks.push(quote!({
        let L: &[(i32,)] = &[(1,), (2,)];
        let R = vec![(0_u32,1), (1_u32,2)];
        let actual: Vec<(&i32,&u32)> = (#expression).collect();
        assert_eq!(actual, vec![(&1, &0)]);
    }));
    checks
}

fn occurrence_checks() -> Vec<proc_macro2::TokenStream> {
    let mut checks = Vec::new();
    for reader in [
        quote!(j = let canon probe first [x] l,r;),
        quote!(j = let canon semi eager [x] l,r;),
    ] {
        let source: structures::physical::Program = syn::parse2(quote! { physical {
            relation L(a:i32); relation R(tag:i32,a:i32);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x} constants {tag = -7};
            #reader
            return j as answer(x);
        }})
        .unwrap();
        let selected = cq_structure_selection::compile(&source).unwrap();
        let facts = structures::contract::check(&selected).unwrap();
        assert!(!facts.retained.contains_key("r"));
        assert!(!facts.filtered.contains_key("r"));
        let expression = sequence_rows(&selected).unwrap();
        checks.push(quote!({
            let L: &[(i32,)] = &[(1,), (2,), (3,)];
            let R: &[(i32,i32)] = &[(-7,1),(-7,1),(7,2),(-7,3)];
            let actual: ::std::collections::BTreeSet<_> =
                (#expression).map(|(x,)| *x).collect();
            assert_eq!(actual, [1,3].into_iter().collect());
        }));
    }
    let authored: structures::Program = syn::parse_quote! { physical {
        relation L(a:i32); relation R(tag:i32,a:i32,b:i32,unused:i32);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,b -> y,unused -> z} constants {tag = -7};
        p = pipe probe first [x] l,r;
        j = let canon project p keep {x,y};
        return j as answer(x,y);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; hash_index = ::cq_storage::HashIndex<'_, _, _> { reads [exact, membership], supports [keys], build typed, join_filter, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact hash_finalize stat_rows host optional build single, keys: keys distinct_keys exact hash_finalize stat_keys host optional build single, max_group: max_group max_bucket_occurrences exact hash_finalize stat_max_group host optional build single ] }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        l host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        r = @hash_index as ::cq_storage::HashIndex<'_,_,_> [x];
        j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
    }};
    let expression = sequence_rows(&authored).unwrap();
    checks.push(quote!({
        let L: &[(i32,)] = &[(1,), (2,), (3,)];
        let R: &[(i32,i32,i32,i32)] =
            &[(-7,1,10,100),(-7,1,11,101),(7,2,20,200),(-7,3,30,300)];
        let actual: ::std::collections::BTreeSet<_> =
            (#expression).map(|(x,y)| (*x,*y)).collect();
        assert_eq!(actual, [(1,10),(1,11),(3,30)].into_iter().collect());
    }));
    let source: structures::physical::Program = syn::parse_quote! { physical {
        relation L(a:i32); relation R(tag:i32,a:i32,b:i32);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,b -> y} constants {tag = -7};
        p = pipe probe first [x] l,r;
        q = pipe project p keep {y};
        j = let canon probe first [y] q,r;
        return j as answer(x,y);
    }};
    let selected = cq_structure_selection::compile(&source).unwrap();
    let facts = structures::contract::check(&selected).unwrap();
    assert_eq!(facts.built["r"].len(), 2);
    assert!(facts.retained.contains_key("r"));
    assert!(!facts.filtered.contains_key("r"));
    let expression = sequence_rows(&selected).unwrap();
    checks.push(quote!({
        let L: &[(i32,)] = &[(1,),(2,),(3,)];
        let R: &[(i32,i32,i32)] = &[(-7,1,10),(7,2,20),(-7,3,30)];
        let actual: ::std::collections::BTreeSet<_> =
            (#expression).map(|(x,y)| (*x,*y)).collect();
        assert_eq!(actual, [(1,10),(3,30)].into_iter().collect());
    }));
    let source: structures::physical::Program = syn::parse_quote! { physical {
        relation R(tag:u32,a:i32,b:bool);
        r = let rename R@0 {a -> tag} constants {tag = 1u32, b = true};
        g = let canon guard r if (*tag > 1);
        return g as answer(tag);
    }};
    let selected = cq_structure_selection::compile(&source).unwrap();
    assert!(
        structures::contract::check(&selected)
            .unwrap()
            .retained
            .contains_key("r")
    );
    let expression = sequence_rows(&selected).unwrap();
    checks.push(quote!({
        let R = vec![
            (1_u32,1,true),
            (1_u32,2,true),
            (1_u32,2,true),
            (0_u32,3,true),
            (1_u32,4,false),
        ];
        let actual: Vec<_> = (#expression).map(|(x,)| *x).collect();
        assert_eq!(actual, vec![2]);
    }));
    let source: structures::physical::Program = syn::parse_quote! { physical {
        relation R(a:i32);
        r = exists canon rename R@0 {} constants {a = 7};
        return r as answer();
    }};
    let expression = sequence_rows(&cq_structure_selection::compile(&source).unwrap()).unwrap();
    checks.push(quote!({
        for (R, expected) in [
            (&[(7,), (7,), (8,)][..], vec![()]),
            (&[(8,)][..], vec![]),
            (&[][..], vec![]),
        ] {
            let actual: Vec<()> = (#expression).collect();
            assert_eq!(actual, expected);
        }
    }));
    checks
}

/// `answer(c, a) :- L(a), R(b, a), S(b, c)`: the head reverses the sorted
/// heading `[a, c]`, and `R` declares `b` before `a`.
#[test]
fn complete_scenarios_compile_and_execute_against_authored_answers() {
    let mut checks = Vec::new();
    for case in scenarios::SCENARIOS {
        let source = case.parse_structures().unwrap();
        let expression = sequence_rows(&source).unwrap();
        let data_name = if case.name == "fine_triangle" {
            "triangle"
        } else {
            case.name.trim_end_matches("_reduced")
        };
        let data = cases::CASES.iter().find(|c| c.name == data_name).unwrap();
        let bindings: Vec<_> = source
            .physical
            .inputs()
            .map(|d| {
                let name = &d.name;
                let rows = data.inputs.iter().find(|(n, _)| name == *n).unwrap().1;
                let tuples = rows.iter().map(|r| quote!((#(#r,)*)));
                let types = d.columns.iter().map(|c| &c.ty);
                quote!(let #name: &[(#(#types,)*)] = &[#(#tuples),*];)
            })
            .collect();
        let variables: Vec<_> = (0..source.physical.program.head.args.len())
            .map(|i| quote::format_ident!("v{i}"))
            .collect();
        let copied = variables.iter().map(|v| quote!(*#v));
        let expected = data.expected.iter().map(|r| quote!((#(#r,)*)));
        let label = case.name;
        checks.push(quote!({
            #(#bindings)*
            let actual: ::std::collections::BTreeSet<_> = (#expression).map(|(#(#variables,)*)| (#(#copied,)*)).collect();
            let expected: ::std::collections::BTreeSet<_> = [#(#expected),*].into_iter().collect();
            assert_eq!(actual, expected, #label);
        }));
    }
    checks.extend(mixed_smallest_checks());
    checks.extend(filtered_checks());
    checks.extend(occurrence_checks());
    let source = quote! {
        #![allow(non_snake_case, unused_variables)]
        fn main() { #(#checks)* }
    };
    let runtime = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../runtime/cq-storage/src/lib.rs");
    let runtime = cq_testkit::rustc::Runtime::new(&runtime, false);
    runtime.run(&source.to_string());
    // Selection records ordered fallbacks even when the host declares an
    // ordered read: Sequence's keyed printer consumes the recorded fallback.
    let unordered: structures::Program = syn::parse_quote!(physical {
        relation L(a:i32) host [a] reads [ordered]; relation R(a:i32) host [a] reads [ordered];
        l = pipe rename L@0 {a -> a}; r = pipe rename R@1 {a -> a};
        j = let canon merge[a] l,r; return j as answer(a);
    } structures { providers {  }});
    let complete = cq_structure_selection::compile(&unordered.physical).unwrap();
    let expression = sequence_rows(&complete).unwrap();
    let fallback = quote! {
        #[allow(non_snake_case)] fn main() {
            let L: &[(i32,)] = &[(2,), (1,), (1,)];
            let R: &[(i32,)] = &[(3,), (1,), (2,), (2,)];
            let actual: ::std::collections::BTreeSet<_> =
                (#expression).map(|(a,)| (*a,)).collect();
            let expected: ::std::collections::BTreeSet<_> = [(1,), (2,)].into_iter().collect();
            assert_eq!(actual, expected);
        }
    };
    runtime.run(&fallback.to_string());
}

#[test]
fn point_prints_build_free_dispatch_and_canonical_lets_use_sort() {
    let p: structures::Program = syn::parse_quote!(physical {
        relation R(a:i32,b:i32) host [a] reads [exact]; r = pipe rename R@0 {a -> x,b -> y};
        w = let canon point select r if (x = 2);
        p = pipe project w keep {x,y}; return p as answer(y,x);
    } structures { providers {  }});
    let text = program(&p);
    assert_eq!(text.matches("Lazy :: new").count(), 0);
    // The declared exact read is one direct point call.
    assert!(text.contains("dispatch :: point ("), "{text}");
    assert!(text.contains("cq_storage :: dedup :: Sort > :: sort"));
    assert!(text.contains("(y , x ,)"));
}

#[test]
fn adapter_carriers_drop_columns_after_their_last_reader() {
    let physical = syn::parse_quote!(physical {
        relation R(a:i32,b:i32,dead:i32);
        r = pipe rename R@0 {a -> a,b -> b,dead -> dead};
        s = pipe select r if (a = 7);
        p = let canon project s keep {b};
        return p as answer(b);
    });
    let module = cq_structure_selection::compile(&physical).unwrap();
    let analysis = structures::contract::check(&module).unwrap();
    let context = cq_print::Context::new(&module, &analysis);
    assert_eq!(context.carried("r"), ["a", "b"]);
    assert_eq!(context.carried("s"), ["b"]);
    assert_eq!(context.carried("p"), ["b"]);
    let printed = text(sequence_rows(&module).unwrap());
    assert!(printed.contains("(a , b , _ ,)"), "{printed}");
    assert!(printed.contains("& (a , _ ,)"), "{printed}");
    assert!(!printed.contains("dead"), "{printed}");
}

#[test]
fn host_certificates_only_emit_for_locally_checked_facts() {
    for (fact, certified) in [
        (quote!(host_set R;), true),
        (quote!(host_set R discharged;), false),
    ] {
        let module: structures::Program = syn::parse2(quote! {
            physical {
                relation R(a:i32); #fact
                r = pipe rename R@0 {a -> a};
                return r as answer(a);
            }
            structures { providers {  }}
        })
        .unwrap();
        let printed = program(&module);
        assert_eq!(printed.contains("__cq_host_set_seen"), certified);
    }
}

#[test]
fn canonical_consumer_omits_local_answer_state_for_every_head_shape() {
    for (head, project) in [
        (quote!(a, b), false),
        (quote!(a), true),
        (quote!(*a % 2), false),
    ] {
        let projection = project.then(|| quote!(p = pipe project r keep {a};));
        let returned = if project { quote!(p) } else { quote!(r) };
        let module: structures::Program = syn::parse2(quote! {
            physical {
                relation R(a:i32,b:i32);
                consumer canonical;
                r = pipe rename R@0 {a -> a,b -> b};
                #projection
                return #returned as answer(#head);
            }
            structures { providers {  }}
        })
        .unwrap();
        let printed = program(&module);
        assert!(!printed.contains("__cq_seen"), "{printed}");
        assert!(!printed.contains("HashSet"), "{printed}");
    }
}

#[test]
fn grouped_folds_share_full_named_valuations_and_empty_identities() {
    let prefix = quote! {
        relation rows(k:i32, witness:i32, value:i64, anonymous:i32);
        raw = pipe rename rows@0 {k -> k, witness -> witness, value -> value, anonymous -> anonymous};
        valuations = let canon project raw keep {k, witness, value};
    };
    let aggregates = quote! {
        n: usize = count();
        total: i64 = sum(value);
        low: Option<i64> = min(value);
        high: Option<i64> = max(value);
    };
    let keyed = syn::parse2(quote! { physical {
        #prefix
        grouped = pipe group valuations by [k] { #aggregates };
        return grouped as answer(k,n,total,low,high);
    }})
    .unwrap();
    let global = syn::parse2(quote! { physical {
        #prefix
        grouped = pipe group valuations by [] { #aggregates };
        return grouped as answer(n,total,low,high);
    }})
    .unwrap();
    let keyed = sequence_rows(&cq_structure_selection::compile(&keyed).unwrap()).unwrap();
    let global = sequence_rows(&cq_structure_selection::compile(&global).unwrap()).unwrap();
    let source = quote! {
        fn main() {
            let populated: &[(i32,i32,i64,i32)] = &[
                (1,10,7,100), (1,10,7,100), (1,10,7,200),
                (1,11,7,100), (1,12,-2,100), (2,20,5,100),
            ];
            for rows in [populated, &[][..]] {
                let mut keyed: Vec<(i32,usize,i64,Option<i64>,Option<i64>)> =
                    (#keyed).map(|(k,n,total,low,high)| (*k,n,total,low,high)).collect();
                keyed.sort();
                let global: Vec<(usize,i64,Option<i64>,Option<i64>)> = (#global).collect();
                if rows.is_empty() {
                    assert!(keyed.is_empty());
                    assert_eq!(global, vec![(0,0,None,None)]);
                } else {
                    // Two witnesses with value 7 contribute twice; wildcard
                    // differences and repeated host rows contribute only once.
                    assert_eq!(keyed, vec![(1,3,12,Some(-2),Some(7)), (2,1,5,Some(5),Some(5))]);
                    assert_eq!(global, vec![(4,17,Some(-2),Some(7))]);
                }
            }
        }
    };
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}

#[test]
#[ignore = "run with CQ_EXPANSION_COUNTERS=1 to enable diagnostic translation"]
fn expansion_counters_measure_children_before_closing_predicates() {
    assert!(std::env::var_os("CQ_EXPANSION_COUNTERS").is_some());
    let physical = syn::parse_quote!(physical {
        relation keys(k:i32); relation payload(k:i32,v:i32);
        relation closing(v:i32,w:i32);
        left = pipe rename keys@0 {k -> k};
        right = let rename payload@1 {k -> k,v -> v};
        joined = pipe probe first [k] left,right;
        last = let rename closing@2 {v -> v,w -> w};
        expanded = pipe probe first [v] joined,last;
        closed = pipe guard expanded if (*w >= 30);
        result = let canon project closed keep {k,v,w};
        return result as answer(k,v,w);
    });
    let selected = cq_structure_selection::compile(&physical).unwrap();
    let program = cq_sequence_lowering::compile(&selected).unwrap();
    let analysis = cq_lang_sequence::check_grading(&program, &cq_sequence_pull::machine()).unwrap();
    let expression = cq_sequence_pull::translate_profile(&program, &analysis).unwrap();
    let source = quote! {
        fn main() {
            let keys = &[(1i32,), (2,)];
            let payload = &[(1i32,10i32), (1,20), (2,30), (2,40)];
            let closing = &[(10i32,0i32), (10,1), (20,2), (30,30), (40,40)];
            ::cq_storage::counters::reset();
            let actual = (#expression).into_iter().map(|(k,v,w)| (*k,*v,*w))
                .collect::<::std::collections::BTreeSet<_>>();
            assert_eq!(actual, ::std::collections::BTreeSet::from([(2,30,30),(2,40,40)]));
            let counts = ::cq_storage::counters::expansions();
            let children = counts.iter().filter(|record| record.stage != "output")
                .map(|record| record.children).max().unwrap();
            let output = counts.iter().find(|record| record.stage == "output").unwrap();
            assert_eq!(children, 5, "count before the closing predicate, not after");
            assert_eq!(output.children, 2);
        }
    };
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, true).run(&source.to_string());
}

#[test]
fn blocked_predicates_execute_exact_formulas_and_own_only_demanded_fields() {
    let cases = [
        (quote!(range), quote!(x >= 3, x < 9), quote!(x >= 3 && x < 9)),
        (quote!(point), quote!(x = 5), quote!(x == 5)),
        (quote!(), quote!(x in {1, 5, 9}), quote!([1, 5, 9].contains(&x))),
        (quote!(), quote!((x < y and x < 4) or (x > 7 and y < 3)), quote!((x < y && x < 4) || (x > 7 && y < 3))),
    ];
    let mut checks = Vec::new();
    for family in [quote!(plain), quote!(bytes)] {
        for (operator, condition, oracle) in &cases {
            let physical = syn::parse2(quote!(physical {
                relation R(a: i64 [1], b: i64 [1], unused: NeverMaterialize [2]);
                r = pipe rename R@0 {a -> x, b -> y, unused -> dead};
                p = pipe #operator select r if (#condition);
                q = pipe range select p if (y >= 0);
                out = pipe project q keep {x,y};
                return canon out as answer(x,y);
            })).unwrap();
            let selected = cq_structure_selection::compile_with(&physical, quote!(
                blocks(R { block_rows 1024; summary none; columns [a: #family u64, b: #family u64, unused: plain u64]; })
            )).unwrap();
            let expression = sequence_rows(&selected).unwrap();
            let storage = if family.to_string() == "plain" { quote!(Plain) } else { quote!(Bytes) };
            checks.push(quote! {
                {
                    let R = ScanTable::new((
                        ScanColumn::<i64, #storage>::new(&a, geometry, false),
                        ScanColumn::<i64, #storage>::new(&b, geometry, false),
                        ScanColumn::<NeverMaterialize, Plain>::new(&unused, geometry, false),
                    )).bind_domains(&[domain(3),domain(3),domain(5)]).unwrap();
                    let mut expected: Vec<_> = a.iter().copied().zip(b.iter().copied())
                        .filter(|&(x,y)| (#oracle) && y >= 0).collect();
                    expected.sort(); expected.dedup();
                    let actual: Vec<_> = (#expression).collect();
                    assert_eq!(actual, expected);
                }
            });
        }
    }
    let physical = syn::parse_quote!(physical {
        relation R(a: i64 [1], b: i64 [1], unused: NeverMaterialize [2]);
        r = pipe rename R@0 {a -> x, b -> y, unused -> dead};
        p = pipe range select r if (x >= 3);
        q = pipe range select p if (x < 9);
        g = pipe guard q if (observe(y, &limit));
        v = pipe bind g with value: i64 = (*x + *y);
        out = pipe project v keep {value};
        return canon out as answer(value);
    });
    let selected = cq_structure_selection::compile_with(&physical, quote!(
        blocks(R { block_rows 1024; summary minmax; columns [a: bytes u64,b: bytes u64,unused: plain u64]; })
    )).unwrap();
    let complete = sequence_rows(&selected).unwrap();
    // Exercise the checked streaming prefix before its mandatory canonical
    // answer consumer, rather than weakening the complete query's semantics.
    let sequence = cq_sequence_lowering::compile(&selected).unwrap();
    let facts = structures::contract::check(&selected).unwrap();
    let mut context = cq_print::Context::new(&selected, &facts);
    context.blocked_runs = sequence.representation.blocks.runs.clone();
    for name in ["r", "p", "q", "g", "v"] { context.register(name); }
    let mut emitter = cq_physical_realization::RowEmitter::new(context);
    let mut prefix = emitter.context().host_scan("r");
    for (name, input) in [("p", "r"), ("q", "p"), ("g", "q"), ("v", "g")] {
        prefix = emitter.operator(name, Some((input.into(), prefix))).unwrap();
    }
    let statements = &emitter.context().statements;
    let lazy = quote!({ #(#statements)* #prefix });
    let source = quote! {
        use cq_storage::scan::*;
        use cq_storage::scan::format::{Domain,DomainId};
        use std::sync::atomic::{AtomicUsize,Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn observe(value: &i64, limit: &i64) -> bool {
            CALLS.fetch_add(1,Ordering::Relaxed);
            value <= limit
        }
        #[derive(Clone,Copy,PartialEq,Eq,PartialOrd,Ord)]
        struct NeverMaterialize(u64);
        impl cq_storage::OrderKey for NeverMaterialize {
            type Ordinal = u64;
            fn ordinal(&self) -> u64 { self.0 }
        }
        impl OrdinalDecode for NeverMaterialize {
            const WIDTH: Width = Width::W64;
            fn from_ordinal(_:u64) -> Self { panic!("unused full-row decode") }
        }
        fn domain(namespace:u64) -> Domain {
            Domain { id: DomainId {namespace,version:0}, carrier:Width::W64,equality:true,order:true }
        }
        fn main() {
            let geometry = BlockRows::new(1024).unwrap();
            for len in [0usize,1,1024,1027,2051] {
                let a: Vec<i64> = (0..len).map(|i| (i % 11) as i64).collect();
                let b: Vec<i64> = (0..len).map(|i| (i % 13) as i64).collect();
                let unused = vec![NeverMaterialize(7);len];
                #(#checks)*
                let R = ScanTable::new((
                    ScanColumn::<i64,Bytes>::new(&a,geometry,true),
                    ScanColumn::<i64,Bytes>::new(&b,geometry,true),
                    ScanColumn::<NeverMaterialize,Plain>::new(&unused,geometry,true),
                )).bind_domains(&[domain(3),domain(3),domain(5)]).unwrap();
                let limit = 8i64;
                CALLS.store(0,Ordering::Relaxed);
                let first = (#lazy).map(|(_, value)| (value,)).next();
                let candidates: Vec<_> = a.iter().copied().zip(b.iter().copied()).filter(|&(x,_)| x >= 3 && x < 9).collect();
                let expected = candidates.iter().position(|&(_,y)| y <= limit);
                assert_eq!(first, expected.map(|index| { let (x,y) = candidates[index]; (x + y,) }));
                assert_eq!(CALLS.load(Ordering::Relaxed), expected.map_or(candidates.len(), |index| index+1));
                CALLS.store(0,Ordering::Relaxed);
                let all: Vec<_> = (#complete).collect();
                let mut expected: Vec<_> = candidates.iter().filter(|row| row.1 <= limit).map(|row| (row.0 + row.1,)).collect();
                expected.sort(); expected.dedup();
                assert_eq!(all,expected);
                assert_eq!(CALLS.load(Ordering::Relaxed),candidates.len());
            }
        }
    };
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}

#[test]
fn blocked_external_witnesses_dedup_equal_values_without_inverse_codecs() {
    let mut checks = Vec::new();
    for (family, storage) in [(quote!(plain), quote!(Plain)), (quote!(bytes), quote!(Bytes))] {
        let physical = syn::parse_quote!(physical {
            relation R(key: NeverMaterialize [2], price: i64 [1]);
            capture { let group: u64 = 0; }
            r = pipe rename R@0 {key -> witness, price -> price};
            v = pipe bind r with amount: i64 = (*price * 2);
            out = pipe project v keep {witness,amount};
            return canon out as rows(group,witness,amount);
        });
        let demand = structures::OutputConsumer {
            original_consumer: "outer".into(),
            head: ["group", "witness", "amount"].map(String::from).into(),
            value_reads: ["group", "amount"].map(String::from).into(),
        };
        let selected = cq_structure_selection::compile_with_consumer(&physical, quote!(
            blocks(R { block_rows 1024; summary none; columns [key: #family u64, price: #family u64]; })
        ), Some(&demand)).unwrap();
        let expression = sequence_rows(&selected).unwrap();
        let reads = &selected.encoded.blocks.reads;
        assert!(reads.values().all(|read| read.purpose != structures::encoded::ReadPurpose::FinalHostAnswer));
        assert!(reads.values().any(|read| read.purpose == structures::encoded::ReadPurpose::OpaqueRustRead));
        assert!(reads.values().filter_map(|read| read.fields.get("witness"))
            .all(|field| field.action == structures::blocks::ReadAction::Ordinal));
        checks.push(quote! {
            {
                let R = ScanTable::new((
                    ScanColumn::<NeverMaterialize, #storage>::new(&keys, geometry, false),
                    ScanColumn::<i64, #storage>::new(&prices, geometry, false),
                )).bind_domains(&[domain(5),domain(3)]).unwrap();
                let mut actual: Vec<_> = (#expression).collect();
                actual.sort();
                assert_eq!(actual, vec![(0u64,1u64,14i64),(0,2,16),(0,3,14)]);
            }
        });
    }
    let source = quote! {
        use cq_storage::scan::*;
        use cq_storage::scan::format::{Domain,DomainId};
        #[derive(Clone,Copy,PartialEq,Eq,PartialOrd,Ord)]
        struct NeverMaterialize(u64);
        impl cq_storage::OrderKey for NeverMaterialize {
            type Ordinal = u64;
            fn ordinal(&self) -> u64 { self.0 }
        }
        impl OrdinalDecode for NeverMaterialize {
            const WIDTH: Width = Width::W64;
            fn from_ordinal(_:u64) -> Self { panic!("ignored witness inverse codec") }
        }
        fn domain(namespace:u64) -> Domain {
            Domain { id: DomainId {namespace,version:0}, carrier:Width::W64,equality:true,order:true }
        }
        fn main() {
            let geometry = BlockRows::new(1024).unwrap();
            // Equal values occur at distinct positions and across block frames.
            // Dedup must produce three values, not 1027 occurrence identities.
            let keys: Vec<_> = (0..1027).map(|i| NeverMaterialize(1 + (i % 3) as u64)).collect();
            let prices: Vec<i64> = (0..1027).map(|i| if i % 3 == 1 { 8 } else { 7 }).collect();
            #(#checks)*
        }
    };
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}
