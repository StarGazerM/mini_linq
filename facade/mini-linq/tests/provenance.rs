use cq_testkit::rewrites as authored;
// Leibniz identity and functional extensionality permit canonical projection.
// Compile and execute both optimized queries and the written-order reference.
use proc_macro2::TokenStream;
use quote::quote;

fn expression(source: &str, options: cq_order::Options, decisions: &str) -> TokenStream {
    elaborate_expression(source, options, decisions, false)
}

fn written_expression(source: &str, decisions: &str) -> TokenStream {
    elaborate_expression(source, cq_order::Options::default(), decisions, true)
}

fn elaborate_expression(
    source: &str,
    options: cq_order::Options,
    decisions: &str,
    written: bool,
) -> TokenStream {
    let source = cq_lang_cq::normal::normalize(&syn::parse_str(source).unwrap()).unwrap();
    let vo = cq_order::compile(&source, &options).unwrap();
    cq_order::check(&source, &vo, &options).unwrap();
    let ra = if written {
        cq_elaborate::compile_written(&vo).unwrap()
    } else {
        cq_elaborate::compile(&vo).unwrap()
    };
    cq_elaborate::check(&vo, &ra).unwrap();
    realize(&ra, decisions)
}

fn realize(ra: &cq_lang_ra::Program, decisions: &str) -> TokenStream {
    let ix = cq_index_policy::compile(ra).unwrap();
    cq_index_policy::check(ra, &ix).unwrap();
    let mut physical = cq_physical_lowering::compile(&ix).unwrap();
    if decisions.contains("merge") {
        physical = authored::merged(&physical).unwrap();
    }
    if decisions.contains("canon") {
        physical = authored::canonical_projects(&physical).unwrap();
    }
    cq_physical_lowering::check(&ix, &physical).unwrap();
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let structures = if decisions.contains("ordered_dedup") {
        authored::ordered_dedup(&structures).unwrap()
    } else {
        structures
    };
    cq_structure_selection::check(&physical, &structures).unwrap();
    let expression = cq_physical_realization::compile(&structures).unwrap();
    quote!(#expression)
}

#[test]
fn admissible_values_and_literal_reads_refine_the_written_order() {
    let mut cases = Vec::new();
    for heuristic in [
        cq_order::Heuristic::Written,
        cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
        cq_order::Heuristic::Elimination,
    ] {
        for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
            for decisions in ["probe keep", "probe canon", "merge keep", "merge canon"] {
                let label = format!("{heuristic:?}/{split:?}/{decisions}");
                // A slice has no native OrderedRead. Merge fixtures group on
                // the declared leading key and retain all payload rows.
                let (r_host, s_host) = if decisions.contains("merge") {
                    (
                        quote!(let R: ::std::collections::BTreeMap<(Key,),Vec<(i32,)>> = rows_r.iter().cloned().fold(::std::collections::BTreeMap::new(), |mut map,(key,row)| { map.entry((key,)).or_default().push((row,)); map });),
                        quote!(let S: ::std::collections::BTreeMap<(Key,),Vec<()>> = rows_s.iter().cloned().map(|row| (row,vec![()])).collect();),
                    )
                } else {
                    (
                        quote!(let R = rows_r.as_slice();),
                        quote!(let S = rows_s.as_slice();),
                    )
                };
                let source = "struct Q; relation R(k:Key,r:i32); relation S(k:Key); \
                              answer(y) :- R(x,z), S(x), let y:bool = x.flag;";
                let written = written_expression(source, decisions);
                let query = expression(source, cq_order::Options { heuristic, split }, decisions);
                cases.push(quote!({
                    let rows_r = [(Key { id: 1, flag: false }, 0)];
                    let rows_s = [(Key { id: 1, flag: false },), (Key { id: 1, flag: true },)];
                    #r_host #s_host
                    let expected: Vec<_> = (#written).collect();
                    let actual: Vec<_> = (#query).collect();
                    assert_eq!(actual, expected, "Leibniz join: {}", #label);
                }));
                let source = "struct Q; relation R(k:Key,r:i32); answer(flag,z) :- R(x,z), let flag:bool=x.flag;";
                let written = written_expression(source, decisions);
                let correlated =
                    expression(source, cq_order::Options { heuristic, split }, decisions);
                cases.push(quote!({
                    let rows_r = [(Key {id:1,flag:false},0),(Key {id:1,flag:true},1)];
                    #r_host
                    let mut written: Vec<_> = (#written).collect();
                    let mut actual: Vec<_> = (#correlated).collect();
                    written.sort(); actual.sort();
                    assert_eq!(actual,written,"full-row correlation: {}",#label);
                }));
                let source = "struct Q; relation R(k:Key,r:i32); answer(y) :- R(x,z), let y:i32=if x.flag {10 / *z} else {0};";
                let written = written_expression(source, decisions);
                let partial = expression(source, cq_order::Options { heuristic, split }, decisions);
                cases.push(quote!({
                    let rows_r = [(Key {id:1,flag:true},1),(Key {id:1,flag:false},0)];
                    #r_host
                    let mut written: Vec<_> = (#written).collect();
                    let mut actual: Vec<_> = (#partial).collect();
                    written.sort(); actual.sort();
                    assert_eq!(actual,written,"correlation cannot introduce a panic: {}",#label);
                }));
            }
        }
    }
    let source = "struct Q; relation R(k:i32,p:i32); \
                  answer(y) :- R(x,z), if * x == 5, let y:i32 = *x + 1;";
    let written = written_expression(source, "decisions {}");
    let point = expression(source, cq_order::Options::default(), "decisions {}");
    cases.push(quote!({
        let rows = [(5_i32,7_i32), (5,8), (6,9)];
        let R = rows.as_slice();
        let expected: Vec<_> = (#written).collect();
        let actual: Vec<_> = (#point).collect();
        assert_eq!(actual, expected, "slice constant read retains the key value");
    }));
    cases.push(quote!({
        let R = ::std::collections::HashMap::from([((5_i32,),vec![(7_i32,), (8,)]), ((6,),vec![(9,)])]);
        let expected: Vec<_> = (#written).collect();
        let actual: Vec<_> = (#point).collect();
        assert_eq!(actual, expected, "native constant read retains the key value");
    }));
    run_cases(&cases);
}

#[test]
fn canonical_projection_before_an_opaque_guard_enumerates_fan_out_linearly() {
    let source = "struct Q; relation R(k:i32,z:i32); relation S(k:i32,w:i32); \
                  answer(y) :- R(x,z), S(x,w), if observe(x,visits), let y:i32=10 / *x;";
    let normalized = cq_lang_cq::normal::normalize(&syn::parse_str(source).unwrap()).unwrap();
    let vo = cq_order::compile(&normalized, &cq_order::Options::default()).unwrap();
    let ra = syn::parse_quote!(relational {
        relation R(k:i32,z:i32); relation S(k:i32,w:i32);
        r = rename R@0 {k -> x, z -> z}; rp = project r keep {x};
        s = rename S@1 {k -> x, w -> w}; sp = project s keep {x};
        j = natural_join rp with sp;
        g = guard j if (observe(x,visits)); b = bind g with y:i32 = (10 / *x);
        p = project b keep {y}; output p as answer(y);
    });
    cq_elaborate::check(&vo, &ra).unwrap();
    let optimized = realize(&ra, "decisions {project=canon;}");
    let written = written_expression(source, "decisions {}");
    run_cases(&[quote!({
        fn observe(x: &i32, visits: &::std::cell::Cell<usize>) -> bool {
            assert_ne!(*x, 0, "both full written atoms must constrain the guard");
            visits.set(visits.get()+1);
            *x > 1
        }
        let mut previous_rows = None;
        for scale in [24_i32,48,96] {
            let mut rows_r: Vec<_> = (0..scale).map(|z| (2,z)).collect();
            rows_r.push((0,0)); // S has no 0: the written reference never panics.
            let rows_s: Vec<_> = (0..scale).map(|w| (2,w)).collect();
            let R = rows_r.as_slice(); let S = rows_s.as_slice();
            let written_visits = ::std::cell::Cell::new(0);
            let visits = &written_visits;
            let expected: ::std::collections::BTreeSet<_> = (#written).collect();
            assert_eq!(written_visits.get(), (scale*scale) as usize);
            let optimized_visits = ::std::cell::Cell::new(0);
            let visits = &optimized_visits;
            ::cq_storage::counters::reset();
            let actual: ::std::collections::BTreeSet<_> = (#optimized).collect();
            let counts = ::cq_storage::counters::snapshot();
            assert_eq!(actual, expected);
            assert_eq!(actual, ::std::collections::BTreeSet::from([(5,)]));
            assert_eq!(optimized_visits.get(), 1);
            assert!(counts.rows_enumerated > 0);
            if let Some(previous) = previous_rows {
                assert!(counts.rows_enumerated*2 < previous*5, "fan-out must enumerate linearly: {previous} -> {counts:?}");
            }
            previous_rows = Some(counts.rows_enumerated);
            println!("fan scale={scale} guard_visits={} counts={counts:?}", optimized_visits.get());
        }
        let R: &[(i32,i32)] = &[(0,0)];
        let S: &[(i32,i32)] = &[];
        let visits = &::std::cell::Cell::new(0);
        assert_eq!((#written).count(), 0);
        assert_eq!((#optimized).count(), 0);
        assert_eq!(visits.get(), 0);
    })]);
}

fn hypergraph_programs(
    normal: &cq_lang_cq::Program,
) -> (
    cq_lang_vo::Program,
    cq_lang_ra::Program,
    cq_lang_ra::Program,
) {
    let ordering = cq_order::Options {
        heuristic: cq_order::Heuristic::Written,
        split: cq_order::Split::Coarse,
    };
    let written_vo = cq_order::compile(normal, &ordering).unwrap();
    cq_order::check(normal, &written_vo, &ordering).unwrap();
    let reference = cq_elaborate::compile_written(&written_vo).unwrap();
    cq_elaborate::check(&written_vo, &reference).unwrap();
    let reductions = cq_reduce::Options {
        semijoins: cq_reduce::Reductions::Tree,
    };
    let reduced = cq_reduce::compile(&written_vo, &reductions).unwrap();
    cq_reduce::check(&written_vo, &reduced, &reductions).unwrap();
    let target = cq_elaborate::compile(&reduced).unwrap();
    cq_elaborate::check(&reduced, &target).unwrap();
    cq_lang_ra::check_rewrite(&reference, &target).unwrap();
    (written_vo, reference, target)
}

fn seed_997_normal() -> cq_lang_cq::Program {
    cq_lang_cq::normal::normalize(
        &syn::parse_str(include_str!(
            "../../../passes/cq-elaborate/tests/fixtures/fuzz_997.cq"
        ))
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn seed_997_hypergraph_executes_against_the_written_reference() {
    let (_, reference, target) = hypergraph_programs(&seed_997_normal());
    let written = realize(&reference, "ordered_dedup");
    let hypergraph = realize(&target, "ordered_dedup");
    // Saved data.txt is `0 2 3 0 2 0`: both declared inputs are empty.
    let generated = cq_testkit::fuzz::generate(997);
    let inputs = [
        [Vec::new(), Vec::new()],
        [generated.data[0].clone(), generated.data[1].clone()],
        [
            vec![
                vec![-3, 1, -3],
                vec![0, 1, -3],
                vec![2, 1, -3],
                vec![4, 0, -3],
                vec![-4, 1, 0],
                vec![-3, 1, -3],
            ],
            vec![
                vec![1, 3],
                vec![-2, 3],
                vec![2, 3],
                vec![7, -3],
                vec![8, 0],
                vec![9, 2],
                vec![1, 3],
                vec![4, 3],
            ],
        ],
    ];
    let mut cases = Vec::new();
    for (i, inputs) in inputs.iter().enumerate() {
        let r0_rows = inputs[0].iter().map(|row| {
            let (a, b, c) = (row[0], row[1], row[2]);
            quote!((#a, #b, #c))
        });
        let r1_rows = inputs[1].iter().map(|row| {
            let (a, b) = (row[0], row[1]);
            quote!((#a, #b))
        });
        let rows = quote! {
            let rows0: &[(i32,i32,i32)] = &[#(#r0_rows),*];
            let rows1: &[(i32,i32)] = &[#(#r1_rows),*];
        };
        for host in [
            quote!(let r0 = rows0; let r1 = rows1;),
            // The failing configuration uses native HashMap hosts.
            quote! {
                let mut map0 = ::std::collections::HashMap::<(i32,),Vec<(i32,i32)>>::new();
                for &(a,b,c) in rows0 { map0.entry((a,)).or_default().push((b,c)); }
                let mut map1 = ::std::collections::HashMap::<(i32,),Vec<(i32,)>>::new();
                for &(a,b) in rows1 { map1.entry((a,)).or_default().push((b,)); }
                let r0 = &map0; let r1 = &map1;
            },
        ] {
            cases.push(quote!({
                #rows #host
                let expected: ::std::collections::BTreeSet<_> = (#written).map(|(a,b)| (a,*b)).collect();
                let actual: ::std::collections::BTreeSet<_> = (#hypergraph).map(|(a,b)| (a,*b)).collect();
                assert_eq!(actual, expected, "seed 997 input {}", #i);
                if #i == 2 {
                    assert_eq!(actual, ::std::collections::BTreeSet::from([(0,-3),(0,0)]));
                }
            }));
        }
    }
    run_cases(&cases);
}

#[test]
fn seed_997_literal_occurrences_are_preserved_before_rust() {
    let normal = seed_997_normal();
    let (written_vo, reference, target) = hypergraph_programs(&normal);
    let mut changed = reference.clone();
    let constants = changed
        .plan
        .definitions
        .iter_mut()
        .find_map(|definition| {
            let cq_lang_ra::Operator::Rename(rename) = &mut definition.operator else {
                return None;
            };
            rename.constants.as_mut()
        })
        .unwrap();
    constants.equalities[0].right = cq_lang_ra::EqualityValue::Literal(syn::parse_quote!(99));
    cq_lang_ra::contract::check(&changed).unwrap();
    assert!(cq_elaborate::check(&written_vo, &changed).is_err());
    assert!(cq_lang_ra::check_rewrite(&reference, &changed).is_err());
    let written = realize(&reference, "ordered_dedup");
    let hypergraph = realize(&target, "ordered_dedup");
    run_cases(&[quote!({
        let r0: &[(i32,i32,i32)] = &[(-3,1,-3)];
        let r1: &[(i32,i32)] = &[(1,0),(7,-3)];
        let expected: ::std::collections::BTreeSet<_> = (#written).collect();
        let actual: ::std::collections::BTreeSet<_> = (#hypergraph).collect();
        assert!(expected.is_empty()); assert_eq!(actual, expected);
    })]);
}

#[test]
fn seed_997_equality_class_reductions_can_suppress_a_written_panic() {
    let mut normal = seed_997_normal();
    let first_guard = normal
        .query
        .body
        .iter_mut()
        .find_map(|item| match item {
            cq_lang_cq::BodyItem::Filter(g) => Some(g),
            _ => None,
        })
        .unwrap();
    let condition = &first_guard.condition;
    first_guard.condition = syn::parse_quote!({ assert_ne!(*v3, 0); #condition });
    let (_, reference, target) = hypergraph_programs(&normal);
    let mut unmatched = target.clone();
    for definition in &mut unmatched.plan.definitions {
        if matches!(definition.operator, cq_lang_ra::Operator::Guard(_)) {
            break;
        }
        if let cq_lang_ra::Operator::Select(select) = &mut definition.operator {
            for equality in &mut select.condition.equalities {
                if matches!(equality.right, cq_lang_ra::EqualityValue::Variable(_)) {
                    equality.right = cq_lang_ra::EqualityValue::Variable(equality.left.clone());
                }
            }
        }
    }
    cq_lang_ra::contract::check(&unmatched).unwrap();
    assert!(
        cq_lang_ra::check_rewrite(&target, &unmatched)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
    let written = realize(&reference, "ordered_dedup");
    let hypergraph = realize(&target, "ordered_dedup");
    let unmatched = realize(&unmatched, "ordered_dedup");
    run_cases(&[quote!({
        let r0: &[(i32, i32, i32)] = &[(-3, 1, -3)];
        let r1: &[(i32, i32)] = &[(1, 3), (7, 0)];
        assert!(::std::panic::catch_unwind(|| (#written).collect::<Vec<_>>()).is_err());
        // Earlier relational restriction may lawfully turn bottom into a Set.
        assert!((#hypergraph).collect::<Vec<_>>().is_empty());
        assert!(::std::panic::catch_unwind(|| (#unmatched).collect::<Vec<_>>()).is_err());
    })]);
}

#[test]
fn partial_generators_refine_written_expression_and_iteration_panics() {
    let mut cases = Vec::new();
    for source in [
        "struct Q; relation R(x:i32); relation S(x:i32); relation answer(value:i32); \
         answer(value) :- R(x), S(x), for value in 0..(12 / *x);",
        "struct Q; relation R(x:i32); relation S(x:i32); relation answer(value:i32); \
         answer(value) :- R(x), S(x), for value in partial_items(x);",
        "struct Q; relation R(x:i32); relation S(x:i32); relation answer(value:i32); \
         answer(value) :- R(x), if std::convert::identity(*x) != 0, \
         for value in partial_items(x), S(x);",
    ] {
        let written = written_expression(source, "decisions {}");
        for heuristic in [
            cq_order::Heuristic::Written,
            cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
            cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
            cq_order::Heuristic::Elimination,
        ] {
            for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
                let optimized = expression(
                    source,
                    cq_order::Options { heuristic, split },
                    "decisions {}",
                );
                cases.push(quote!({
                    fn partial_items(x: &i32) -> impl Iterator<Item = i32> {
                        let denominator = *x;
                        (0..2).map(move |value| {
                            if value == 0 { 0 } else { 12 / denominator }
                        })
                    }
                    for (rows_r, rows_s) in [
                        (vec![(0,), (2,)], vec![(2,)]),
                        (vec![(0,)], vec![]),
                        (vec![(0,)], vec![(0,)]),
                        (vec![(2,), (2,)], vec![(2,), (2,)]),
                    ] {
                        let R = rows_r.as_slice();
                        let S = rows_s.as_slice();
                        let expected = ::std::panic::catch_unwind(|| {
                            (#written).collect::<::std::collections::BTreeSet<_>>()
                        });
                        let actual = ::std::panic::catch_unwind(|| {
                            (#optimized).collect::<::std::collections::BTreeSet<_>>()
                        });
                        // Defined written answers must remain defined and identical;
                        // written bottom may refine to either bottom or a set.
                        if let Ok(expected) = expected {
                            assert_eq!(actual.unwrap(), expected);
                        }
                    }
                }));
            }
        }
    }
    run_cases(&cases);
}

fn run_cases(cases: &[TokenStream]) {
    let program = quote! {
        #![allow(non_snake_case, unused_variables, unused_macros)]
        #[derive(Clone,Debug,PartialEq,Eq,Hash,PartialOrd,Ord)]
        struct Key { id:i32, flag:bool }
        fn main() { #(#cases)* }
    };
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    let runtime = cq_testkit::rustc::Runtime::new(&source, true);
    let run = runtime.run(&program.to_string());
    print!("{}", String::from_utf8_lossy(&run.stdout));
}
