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
fn zero_heading_exists_can_be_scanned_and_probed_against_itself() {
    let physical = syn::parse_quote!(physical {
        relation rows(a:i32);
        r = pipe rename rows@0 {a -> a};
        p = exists project r keep {};
        out = pipe canon semi eager [] p,p;
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
        out = pipe canon project observed keep {b};
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
fn worker_guards_use_serial_or_explicit_target_worker_counts() {
    use cq_lang_sequence::Term;
    let physical = syn::parse_quote!(physical {
        relation input(a:i32);
        r = pipe canon rename input@0 {a -> a};
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
                    out = pipe canon project expanded keep {a,b,c};
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
                        keyed.ty = syn::parse_quote!(::cq_storage::SortedRun<_, _>);
                    }
                }
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
    use syn::visit_mut::VisitMut;
    struct ReplaceSets(syn::Type);
    impl VisitMut for ReplaceSets {
        fn visit_type_mut(&mut self, ty: &mut syn::Type) {
            if *ty == syn::parse_quote!(::cq_storage::HashSet<_>) {
                *ty = self.0.clone();
            } else {
                syn::visit_mut::visit_type_mut(self, ty);
            }
        }
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
            let mut replace = ReplaceSets(ty.clone());
            for entry in &mut recorded.structures.entries {
                let provider = match &mut entry.structure {
                    Structure::Built { keyed, .. } | Structure::Host { keyed, .. } => &mut keyed.ty,
                    Structure::Rows { ty, .. }
                    | Structure::Dedup { ty, .. }
                    | Structure::HeadDedup { ty, .. }
                    | Structure::Group { ty, .. }
                    | Structure::Selection { ty, .. }
                    | Structure::KeySet { ty, .. } => ty,
                    Structure::Read { .. }
                    | Structure::JoinFilter { .. }
                    | Structure::Repr { .. }
                    | Structure::Stage { .. }
                    | Structure::Share { .. } => continue,
                };
                replace.visit_type_mut(provider);
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
                checked = pipe canon semi eager [a,c] candidate,s;
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
            checked = pipe canon semi eager [a,c] candidate,s;
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
                    out = pipe canon project shared keep {{{kept}}}; return out as answer(a,k,v);
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
                                keyed.ty = syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>);
                            }
                        }
                    }
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
            out = pipe canon semi eager [{key}] j,f; return out as answer(a,k);
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
                out = pipe canon project j keep {a,k}; return out as answer(a,k);
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
        relation L(k:i32); relation R(tag:String,k:i32);
        l = pipe rename L@0 {k -> k}; r = pipe rename R@1 {tag -> tag,k -> k};
        f = let point select r if (tag = "a");
        j = pipe canon probe first [k] l,f; return j as answer(k,tag);
    });
    let expression = sequence_rows(&cq_structure_selection::compile(&source).unwrap()).unwrap();
    checks.push(quote!({
        let L: &[(i32,)] = &[(1,), (2,)];
        let R = vec![(String::from("a"),1), (String::from("b"),2)];
        let actual: Vec<(&i32,&String)> = (#expression).collect();
        assert_eq!(actual, vec![(&1, &String::from("a"))]);
    }));
    for ty in ["&'a str", "&'a String"] {
        let source: structures::physical::Program = syn::parse_str(&format!(
            "physical {{
            relation L(k:i32); relation R(tag:{ty},k:i32);
            l = pipe rename L@0 {{k -> k}}; r = pipe rename R@1 {{tag -> tag,k -> k}};
            f = let point select r if (tag = \"a\");
            j = pipe canon probe first [k] l,f; return j as answer(k,tag);
        }}"
        ))
        .unwrap();
        let expression = sequence_rows(&cq_structure_selection::compile(&source).unwrap()).unwrap();
        let ty: syn::Type = syn::parse_str(ty).unwrap();
        let input = if text(&ty).contains("String") {
            quote!(let a = String::from("a"); let b = String::from("b"); let R = [(&a,1),(&b,2)];)
        } else {
            quote!(let R = [("a",1),("b",2)];)
        };
        checks.push(quote!({
            fn run<'a>(L: &'a [(i32,)], R: &'a [(#ty,i32)]) -> Vec<(&'a i32,&'a #ty)> {
                (#expression).collect()
            }
            let L = [(1,), (2,)];
            #input
            assert_eq!(run(&L,&R), vec![(&L[0].0,&R[0].0)]);
        }));
    }
    checks
}

fn occurrence_checks() -> Vec<proc_macro2::TokenStream> {
    let mut checks = Vec::new();
    for reader in [
        quote!(j = pipe canon probe first [x] l,r;),
        quote!(j = pipe canon semi eager [x] l,r;),
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
        j = pipe canon project p keep {x,y};
        return j as answer(x,y);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r = ::cq_storage::HashIndex<'_,_,_> [x];
        j canon = ::cq_storage::HashSet<_>;
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
        j = pipe canon probe first [y] q,r;
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
        relation R(tag:String,a:i32,b:bool);
        r = let rename R@0 {a -> tag} constants {tag = "yes", b = true};
        g = pipe canon guard r if (*tag > 1);
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
            (String::from("yes"),1,true),
            (String::from("yes"),2,true),
            (String::from("yes"),2,true),
            (String::from("no"),3,true),
            (String::from("yes"),4,false),
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
    // Direct merge premises are deliberately left to rustc, with the trait diagnostic.
    let unordered: structures::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32);
        l = pipe rename L@0 {a -> a}; r = pipe rename R@1 {a -> a};
        j = pipe canon merge[a] l,r; return j as answer(a);
    } structures {});
    let complete = cq_structure_selection::compile(&unordered.physical).unwrap();
    let expression = sequence_rows(&complete).unwrap();
    let invalid = quote!(#source
        #[allow(non_snake_case)] fn unordered_merge() {
            let L: &[(i32,)] = &[(1,)]; let R: &[(i32,)] = &[(1,)];
            let _: Vec<_> = (#expression).collect();
        }
    );
    let rejected = runtime.compile(&invalid.to_string());
    assert!(!rejected.status.success());
    let diagnostic = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        diagnostic.contains("this read requires ordered inputs"),
        "{diagnostic}"
    );
}

#[test]
fn point_prints_build_free_dispatch_and_canonical_lets_use_the_distinct_provider() {
    let p: structures::Program = syn::parse_quote!(physical {
        relation R(a:i32,b:i32); r = pipe rename R@0 {a -> x,b -> y};
        w = let canon point select r if (x = 2);
        p = pipe project w keep {x,y}; return p as answer(y,x);
    } structures {});
    let text = program(&p);
    assert_eq!(text.matches("Lazy :: new").count(), 0);
    assert!(text.contains("dispatch :: PointHost"));
    assert!(text.contains(". point ("));
    assert!(text.contains("HashSet < _ > > :: new"));
    assert!(text.contains("(y , x ,)"));
}

#[test]
fn adapter_carriers_drop_columns_after_their_last_reader() {
    let physical = syn::parse_quote!(physical {
        relation R(a:i32,b:i32,dead:i32);
        r = pipe rename R@0 {a -> a,b -> b,dead -> dead};
        s = pipe select r if (a = 7);
        p = pipe canon project s keep {b};
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
            structures {}
        })
        .unwrap();
        let printed = program(&module);
        assert_eq!(printed.contains("__cq_host_set_seen"), certified);
        assert_eq!(printed.contains("certificate failed"), certified);
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
            structures {}
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
        valuations = pipe canon project raw keep {k, witness, value};
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
