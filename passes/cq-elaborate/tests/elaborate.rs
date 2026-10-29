//! RA prime correspondence: fixed occurrences and relational constraints,
//! with unchanged opaque, total Rust expressions and free-variable dependencies.
use cq_testkit::{corpus as scenarios, cq_semantics as cq_oracle, databases, semantics as oracle};

use cq_elaborate::check;
use cq_lang_ra as ra;
use cq_lang_vo as valuation;
use cq_lang_vo::cq;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn compile(source: &valuation::Program) -> syn::Result<ra::Program> {
    let target = cq_elaborate::compile(source)?;
    check(source, &target)?;
    Ok(target)
}

fn cq(tokens: TokenStream) -> cq::Program {
    syn::parse2(tokens).unwrap()
}
fn vo(source: &cq::Program, steps: TokenStream) -> valuation::Program {
    syn::parse2(quote! { #source valuation { #steps } }).unwrap()
}
fn coarse(source: &cq::Program) -> valuation::Program {
    let steps = source
        .query
        .body
        .iter()
        .enumerate()
        .map(|(clause, item)| match item {
            cq::BodyItem::Positive { atom } => {
                let relation = &atom.relation;
                let clause = proc_macro2::Literal::usize_unsuffixed(clause);
                let columns = atom.variables();
                quote!(#relation @ #clause (#(#columns),*);)
            }
            cq::BodyItem::Filter(filter) => quote!(#filter;),
            cq::BodyItem::Let(binding) => quote!(#binding;),
            cq::BodyItem::For(binding) => quote!(#binding;),
            cq::BodyItem::Equality(equality) => quote!(#equality;),
            cq::BodyItem::Constraint(c) => quote!(where #c;),
            cq::BodyItem::Negation(n) => quote!(#n;),
            _ => unreachable!("normal form"),
        });
    vo(source, quote!(#(#steps)*))
}

// The VO contract admits equivalent equality nodes constructed through either IR variant.
fn equality_forms(source: &valuation::Program) -> Vec<valuation::Program> {
    let mut forms = Vec::new();
    for source_constraints in [false, true] {
        for step_constraints in [false, true] {
            let mut form = source.clone();
            if source_constraints {
                for item in &mut form.cq.query.body {
                    if let cq::BodyItem::Equality(equality) = item {
                        *item = cq::BodyItem::Constraint(cq::Constraint::from_equality(equality));
                    }
                }
            }
            if step_constraints {
                for step in &mut form.valuation.steps {
                    if let valuation::Step::Equality(equality, semi) = step {
                        *step = valuation::Step::Constraint(
                            cq::Constraint::from_equality(equality),
                            *semi,
                        );
                    }
                }
            }
            forms.push(form);
        }
    }
    forms
}
fn assert_plan(target: &ra::Program, plan: TokenStream) {
    let declarations = &target.plan.declarations;
    let expected: ra::Program =
        syn::parse2(quote! { relational { #(#declarations)* #plan } }).unwrap();
    assert_eq!(target, &expected);
}
fn chain() -> cq::Program {
    cq(quote! {
        struct Chain; relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(a, b), S(b, c);
    })
}

#[test]
fn generated_results_do_not_shadow_declared_relations() {
    let source = cq(quote! {
        struct Collision; relation r0(x: i32); relation r1(x: i32); relation r2(x: i32);
        r2(x) :- r0(x);
    });
    let target = compile(&coarse(&source)).unwrap();
    assert_plan(
        &target,
        quote! {
            r3 = rename r0@0 { x -> x }; r4 = project r3 keep { x }; output r4 as r2(x);
        },
    );
}

#[test]
fn coarse_plans_elaborate_relation_at_a_time() {
    let target = compile(&coarse(&chain())).unwrap();
    assert_plan(
        &target,
        quote! {
            r0 = rename L@0 { a -> a }; r1 = rename R@1 { a -> a, b -> b };
            r2 = natural_join r0 with r1; r3 = rename S@2 { b -> b, c -> c };
            r4 = natural_join r2 with r3; r5 = project r4 keep { a, c };
            output r5 as answer(a, c);
        },
    );
}

#[test]
fn literals_are_relational_reads_and_repeats_are_relational_equalities() {
    let source = cq(quote! {
        struct Q; relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        answer(x, y) :- R(x, 5), S(x, x), if { *x > 0 }, let y: i32 = *x + 1;
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let target = compile(&coarse(&normal)).unwrap();
    let selections: Vec<_> = target
        .plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Select(select) => Some(select),
            _ => None,
        })
        .collect();
    assert!(
        !selections
            .iter()
            .any(|s| s.condition.constant_keys().is_some())
    );
    let rename = target
        .plan
        .definitions
        .iter()
        .find_map(|d| match &d.operator {
            ra::Operator::Rename(rename) if rename.relation == "R" => Some(rename),
            _ => None,
        })
        .unwrap();
    assert_eq!(rename.mapping.len(), 1);
    assert_eq!(rename.mapping[0].source, "a");
    assert_eq!(
        rename.constants.as_ref().unwrap().equalities,
        syn::parse_quote!(b = 5)
    );
    assert!(selections.iter().any(|s| {
        s.condition
            .equalities
            .iter()
            .any(|e| matches!(e.right, ra::EqualityValue::Variable(_)))
    }));
    assert!(
        !target
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Unit(_)))
    );
    assert!(
        target
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Bind(_)))
    );
    assert!(
        target
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Guard(_)))
    );
}

#[test]
fn a_computed_head_is_a_final_bind_with_variable_output() {
    let source = cq(quote! {
        struct Sum; relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        relation answer(value: i32); answer(*a + *c) :- R(a, b), S(b, c);
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let target = compile(&coarse(&normal)).unwrap();
    assert_eq!(target.plan.output.head.variables().count(), 1);
    let final_bind = target
        .plan
        .definitions
        .iter()
        .rposition(|d| matches!(d.operator, ra::Operator::Bind(_)))
        .unwrap();
    assert_eq!(final_bind + 2, target.plan.definitions.len());
}

#[test]
fn normalized_cross_atom_equalities_become_shared_join_headings() {
    let normal = cq::normal::normalize(&chain()).unwrap();
    let target = compile(&coarse(&normal)).unwrap();
    let analysis = ra::contract::check(&target).unwrap();
    for definition in &target.plan.definitions {
        if let ra::Operator::NaturalJoin(join) = &definition.operator {
            assert!(
                !analysis[&join.left.to_string()]
                    .heading
                    .is_disjoint(&analysis[&join.right.to_string()].heading)
            );
        }
    }
}

#[test]
fn fine_plans_keep_existential_projection_and_semijoin_meaning() {
    let source = cq(quote! {
        struct Q; relation R(a: i32, b: i32); relation S(b: i32, c: i32); relation T(a: i32, c: i32);
        answer(a, c) :- R(a, b), S(b, c), T(a, c);
    });
    let plans = [
        quote! { R@0(a, _); T@2(a, _); R@0(a, b); S@1(b, _); S@1(b, c); T@2(a, c); },
        quote! { S@1(b, c); R@0(a, b); T@2(a, c); },
        quote! { T@2(a, c); R@0(a, _); R@0(a, b); S@1(b, c); },
    ];
    for steps in plans {
        let target = compile(&vo(&source, steps)).unwrap();
        for seed in 0..64 {
            let database = databases::binary_database(&source, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                cq_oracle::evaluate_cq(&source, &database),
                "seed {seed}: {}",
                target.to_token_stream()
            );
        }
    }
}

#[test]
fn semijoin_reductions_restrict_clause_relations() {
    let source = chain();
    let target = compile(&vo(
        &source,
        quote! {
            reduce R@1 with L@0(a); reduce S@2 with R@1(_, b);
            reduce R@1 with S@2(b, _); reduce L@0 with R@1(a, _);
            L@0(a); R@1(a, b); S@2(b, c);
        },
    ))
    .unwrap();
    for seed in 0..64 {
        let database = databases::binary_database(&source, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            cq_oracle::evaluate_cq(&source, &database)
        );
    }
}

#[test]
fn reduction_absorption_restores_native_inputs_and_removes_dead_support() {
    let query = cq(quote! {
        struct Q; relation l(k:i32); relation r(k:i32,v:i32);
        answer(k,v) :- l(k), r(k,v);
    });
    let reduced = vo(
        &query,
        quote! {
            reduce r@1 with l@0(k); reduce l@0 with r@1(k,_);
            l@0(k); r@1(k,v);
        },
    );
    let source: ra::Program = syn::parse_quote! { relational {
        relation l(k:i32); relation r(k:i32,v:i32);
        r0 = rename r@1 {k -> k,v -> v};
        r1 = rename l@0 {k -> k};
        r2 = semijoin r0 with r1;
        r3 = semijoin r1 with r2;
        r4 = natural_join r3 with r2;
        output r4 as answer(k,v);
    }};
    check(&reduced, &source).unwrap();
    let target = cq_elaborate::simplify_reductions(&source).unwrap();
    let reference = cq_elaborate::compile_written(&coarse(&query)).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&reference, &target).unwrap();
    // Absorption is an RA rewrite, not permission to omit a named VO reduction.
    assert!(check(&reduced, &target).is_err());
    let repeated = cq_elaborate::simplify_reductions(&target).unwrap();
    ra::check_rewrite(&target, &repeated).unwrap();
    for seed in 0..64 {
        let database = databases::binary_database(&query, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            cq_oracle::evaluate_cq(&query, &database),
            "seed {seed}"
        );
        assert_eq!(
            oracle::evaluate_ra(&repeated, &database).output,
            cq_oracle::evaluate_cq(&query, &database),
            "repeated pass, seed {seed}"
        );
    }
}

#[test]
fn ternary_guard_absorption_preserves_the_shared_restricted_witness() {
    let query = scenarios::SCENARIOS
        .iter()
        .find(|scenario| scenario.name == "ternary_guard")
        .unwrap()
        .parse_cq()
        .unwrap();
    let written = coarse(&query);
    let reduced = vo(
        &query,
        quote! {
            reduce Stock@3 with C@2(d,t);
            reduce Stock@3 with B@1(i,t);
            reduce Stock@3 with A@0(d,i);
            reduce A@0 with Stock@3(d,i,_);
            reduce B@1 with Stock@3(_,i,t);
            reduce C@2 with Stock@3(d,_,t);
            A@0(d,i); B@1(i,t); C@2(d,t); Stock@3(d,i,t);
        },
    );
    let source = compile(&reduced).unwrap();
    let target = cq_elaborate::simplify_reductions(&source).unwrap();
    // This is the final external-planner contract, not just the local
    // source-to-target absorption certificate.
    check(&written, &target).unwrap();
    let reference = cq_elaborate::compile_written(&written).unwrap();
    ra::check_rewrite(&reference, &target).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    let full_input = |plan: &ra::Program| {
        plan.plan
            .definitions
            .iter()
            .rev()
            .find_map(|definition| {
                if let ra::Operator::NaturalJoin(join) = &definition.operator {
                    Some(join.right.clone())
                } else {
                    None
                }
            })
            .unwrap()
    };
    assert_eq!(full_input(&target), full_input(&source));
    let full_stock = full_input(&source);
    let older_stock = source
        .plan
        .definitions
        .iter()
        .find_map(|definition| {
            if let ra::Operator::Semijoin(join) = &definition.operator
                && definition.result == full_stock
            {
                Some(join.left.clone())
            } else {
                None
            }
        })
        .unwrap();
    // Partially bypassing the shared restriction is algebraically exact, but
    // splits independently projected witnesses under the structural contract.
    let mut split_witness = target.clone();
    let first_support = split_witness
        .plan
        .definitions
        .iter_mut()
        .find_map(|definition| {
            if let ra::Operator::Semijoin(join) = &mut definition.operator
                && join.right == full_stock
            {
                Some(join)
            } else {
                None
            }
        })
        .unwrap();
    first_support.right = older_stock;
    ra::contract::check(&split_witness).unwrap();
    assert!(check(&written, &split_witness).is_err());
    assert!(ra::check_rewrite(&source, &split_witness).is_err());
    for seed in 0..64 {
        let database = databases::binary_database(&query, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            cq_oracle::evaluate_cq(&query, &database),
            "seed {seed}"
        );
    }
}

#[test]
fn reduction_absorption_certificate_rejects_changed_support_predicates() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation l(k:i32); relation r(k:i32,v:i32);
        r0 = rename r@1 {k -> k,v -> v};
        r1 = rename l@0 {k -> k};
        support = select r1 if (k > 0);
        r2 = semijoin r0 with support;
        r3 = semijoin support with r2;
        r4 = natural_join r3 with r2;
        output r4 as answer(k,v);
    }};
    let target = cq_elaborate::simplify_reductions(&source).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    let mut changed = target;
    changed.plan.definitions[2].operator = syn::parse_quote!(select r1 if (k >= 0));
    ra::contract::check(&changed).unwrap();
    assert!(ra::check_rewrite(&source, &changed).is_err());
}

#[test]
fn reduction_absorption_retains_support_for_an_uncontained_consumer() {
    let query = cq(quote! {
        struct Q; relation R(x:i32,y:i32); relation T(y:i32); relation U(x:i32);
        answer(x,y) :- R(x,y), T(y), U(x);
    });
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(x:i32,y:i32); relation T(y:i32); relation U(x:i32);
        r = rename R@0 {x -> x,y -> y};
        t = rename T@1 {y -> y};
        u = rename U@2 {x -> x};
        reduced = semijoin r with t;
        result = natural_join u with reduced;
        output result as answer(x,y);
    }};
    let target = cq_elaborate::simplify_reductions(&source).unwrap();
    assert_eq!(target, source);
    let reference = cq_elaborate::compile_written(&coarse(&query)).unwrap();
    ra::check_rewrite(&reference, &target).unwrap();
    let mut bypassed = source.clone();
    bypassed.plan.definitions[4].operator = syn::parse_quote!(natural_join u with r);
    assert!(ra::check_rewrite(&source, &bypassed).is_err());
    let mut database = databases::binary_database(&query, 0);
    database.insert("R".into(), [vec![0, 1]].into());
    database.insert("T".into(), [vec![0]].into());
    database.insert("U".into(), [vec![0]].into());
    let expected = cq_oracle::evaluate_cq(&query, &database);
    assert_eq!(oracle::evaluate_ra(&target, &database).output, expected);
    assert_ne!(oracle::evaluate_ra(&bypassed, &database).output, expected);
}

#[test]
fn reduction_absorption_does_not_confuse_projected_aliases_with_full_key_support() {
    // Sharing T's x projection is insufficient: the reducer also matches y.
    // The aliases deliberately share a base occurrence but not its full heading.
    for (consumer, bypass, head) in [
        (
            quote!(natural_join alias with reduced),
            quote!(natural_join alias with r),
            quote!(answer(x, y)),
        ),
        (
            quote!(semijoin alias with reduced),
            quote!(semijoin alias with r),
            quote!(answer(x)),
        ),
        (
            quote!(natural_join reduced with alias),
            quote!(natural_join r with alias),
            quote!(answer(x, y)),
        ),
    ] {
        let query = cq(quote! {
            struct Q; relation R(x:i32,y:i32); relation T(x:i32,y:i32);
            #head :- R(x,y), T(x,y);
        });
        let source: ra::Program = syn::parse_quote! { relational {
            relation R(x:i32,y:i32); relation T(x:i32,y:i32);
            r = rename R@0 {x -> x,y -> y};
            t = rename T@1 {x -> x,y -> y};
            reduced = semijoin r with t;
            projected = project t keep {x};
            alias = project projected keep {x};
            result = #consumer;
            output result as #head;
        }};
        let target = cq_elaborate::simplify_reductions(&source).unwrap();
        assert_eq!(target, source);
        ra::check_rewrite(&source, &target).unwrap();
        let mut bypassed = source.clone();
        bypassed.plan.definitions[5].operator = syn::parse2(bypass).unwrap();
        assert!(ra::check_rewrite(&source, &bypassed).is_err());
        let mut database = databases::binary_database(&query, 0);
        database.insert("R".into(), [vec![0, 1]].into());
        database.insert("T".into(), [vec![0, 0]].into());
        let expected = cq_oracle::evaluate_cq(&query, &database);
        assert_eq!(oracle::evaluate_ra(&target, &database).output, expected);
        assert_ne!(oracle::evaluate_ra(&bypassed, &database).output, expected);
    }
}

#[test]
fn two_sided_absorption_keeps_the_restriction_justifying_the_other_operand() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(x:i32); relation S(x:i32); relation T(x:i32);
        r = rename R@0 {x -> x};
        s = rename S@1 {x -> x};
        t = rename T@2 {x -> x};
        left = semijoin r with s;
        right = semijoin t with left;
        result = natural_join left with right;
        output result as answer(x);
    }};
    let target = cq_elaborate::simplify_reductions(&source).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    let invalid: ra::Program = syn::parse_quote! { relational {
        relation R(x:i32); relation S(x:i32); relation T(x:i32);
        r = rename R@0 {x -> x};
        s = rename S@1 {x -> x};
        t = rename T@2 {x -> x};
        result = natural_join r with t;
        output result as answer(x);
    }};
    assert!(ra::check_rewrite(&source, &invalid).is_err());
    for support in [vec![], vec![vec![1]]] {
        let database = [
            ("R".into(), [vec![1], vec![2]].into()),
            ("S".into(), support.into_iter().collect()),
            ("T".into(), [vec![1]].into()),
        ]
        .into();
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            oracle::evaluate_ra(&source, &database).output
        );
    }
}

fn dead_nation_expansion(region_consumer: TokenStream, tail: TokenStream) -> ra::Program {
    syn::parse2(quote! { relational {
        relation Left(nation_key:i32); relation Nation(nation_key:i32,region_key:i32);
        relation Region(region_key:i32);
        left = rename Left@0 {nation_key -> nation_key};
        nation = rename Nation@1 {nation_key -> nation_key,region_key -> region_key};
        region = rename Region@2 {region_key -> region_key};
        reduced_nation = semijoin nation with region;
        witnessed = semijoin left with reduced_nation;
        expanded = natural_join witnessed with reduced_nation;
        supported = #region_consumer;
        #tail
    } })
    .unwrap()
}

#[test]
fn dead_expansion_eliminates_the_canonical_nation_region_chain() {
    for consumer in [
        quote!(semijoin expanded with region),
        quote!(natural_join expanded with region),
    ] {
        let source = dead_nation_expansion(
            consumer,
            quote! {
                result = project supported keep {nation_key};
                output result as answer(nation_key);
            },
        );
        let target = cq_elaborate::simplify_expansions(&source).unwrap();
        ra::contract::check(&target).unwrap();
        ra::check_rewrite(&source, &target).unwrap();
        assert!(
            !target
                .plan
                .definitions
                .iter()
                .any(|d| matches!(d.operator, ra::Operator::NaturalJoin(_))),
            "unobservable Nation/Region expansion survives: {}",
            target.to_token_stream()
        );
        assert_eq!(
            target
                .plan
                .definitions
                .iter()
                .filter(|d| matches!(d.operator, ra::Operator::Semijoin(_)))
                .count(),
            2,
            "only Nation's Region restriction and Left's Nation witness are needed"
        );
        assert_eq!(cq_elaborate::simplify_expansions(&target).unwrap(), target);
        let query = cq(quote! {
            struct Q; relation Left(nation_key:i32);
            relation Nation(nation_key:i32,region_key:i32); relation Region(region_key:i32);
            answer(nation_key) :- Left(nation_key), Nation(nation_key,region_key), Region(region_key);
        });
        for seed in 0..64 {
            let database = databases::binary_database(&query, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                oracle::evaluate_ra(&source, &database).output,
                "seed {seed}"
            );
        }
    }
}

#[test]
fn dead_expansion_retains_observable_region_columns() {
    let source = dead_nation_expansion(
        quote!(semijoin expanded with region),
        quote! {
            result = project supported keep {nation_key,region_key};
            output result as answer(nation_key,region_key);
        },
    );
    let target = cq_elaborate::simplify_expansions(&source).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    assert!(
        target
            .plan
            .definitions
            .iter()
            .any(|d| d.result == "expanded" && matches!(d.operator, ra::Operator::NaturalJoin(_)))
    );
    let mut forged = source.clone();
    forged
        .plan
        .definitions
        .iter_mut()
        .find(|d| d.result == "expanded")
        .unwrap()
        .operator = syn::parse_quote!(semijoin witnessed with reduced_nation);
    assert!(ra::contract::check(&forged).is_err());
    assert!(ra::check_rewrite(&source, &forged).is_err());
}

#[test]
fn dead_expansion_does_not_erase_an_opaque_region_observation() {
    let source = dead_nation_expansion(
        quote!(semijoin expanded with region),
        quote! {
            observed = guard supported if (10 / *region_key > 0);
            result = project observed keep {nation_key};
            output result as answer(nation_key);
        },
    );
    let target = cq_elaborate::simplify_expansions(&source).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    assert!(
        target
            .plan
            .definitions
            .iter()
            .any(|d| d.result == "expanded" && matches!(d.operator, ra::Operator::NaturalJoin(_)))
    );
    assert!(
        target
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Guard(_)))
    );
}

#[test]
fn dead_expansion_does_not_confuse_distinct_named_region_support() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation Left(nation_key:i32); relation Nation(nation_key:i32,region_key:i32);
        relation Region(region_key:i32);
        left = rename Left@0 {nation_key -> nation_key};
        nation = rename Nation@1 {nation_key -> nation_key,region_key -> region_key};
        region = rename Region@2 {region_key -> region_key};
        restricted_region = select region if (region_key > 0);
        reduced_nation = semijoin nation with region;
        witnessed = semijoin left with reduced_nation;
        expanded = natural_join witnessed with reduced_nation;
        supported = semijoin expanded with restricted_region;
        result = project supported keep {nation_key};
        output result as answer(nation_key);
    }};
    let target = cq_elaborate::simplify_expansions(&source).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    let database: oracle::Database = [
        ("Left".into(), [vec![0]].into()),
        ("Nation".into(), [vec![0, 0]].into()),
        ("Region".into(), [vec![0], vec![1]].into()),
    ]
    .into();
    assert!(oracle::evaluate_ra(&target, &database).output.is_empty());
    let mut forged = source.clone();
    forged
        .plan
        .definitions
        .iter_mut()
        .find(|d| d.result == "supported")
        .unwrap()
        .operator = syn::parse_quote!(project expanded keep {nation_key,region_key});
    forged
        .plan
        .definitions
        .retain(|d| d.result != "restricted_region");
    ra::contract::check(&forged).unwrap();
    assert!(ra::check_rewrite(&source, &forged).is_err());
    assert!(!oracle::evaluate_ra(&forged, &database).output.is_empty());
}

#[test]
fn dead_expansion_duplicate_rich_bags_agree_only_after_canon() {
    use std::collections::{BTreeMap, BTreeSet};
    type Row = BTreeMap<String, i32>;
    // Deliberately retain duplicates at every operator: the shared set oracle
    // cannot expose the multiplicity change which this optimization permits.
    fn bag(program: &ra::Program, database: &BTreeMap<String, Vec<Vec<i32>>>) -> Vec<Vec<i32>> {
        let mut values = BTreeMap::<String, Vec<Row>>::new();
        for definition in &program.plan.definitions {
            let rows = match &definition.operator {
                ra::Operator::Rename(rename) => {
                    let declaration = program
                        .plan
                        .declarations
                        .iter()
                        .find(|d| d.name == rename.relation)
                        .unwrap();
                    database[&rename.relation.to_string()]
                        .iter()
                        .map(|tuple| {
                            rename
                                .mapping
                                .iter()
                                .map(|m| {
                                    let column = declaration
                                        .columns
                                        .iter()
                                        .position(|c| c.name == m.source)
                                        .unwrap();
                                    (m.target.to_string(), tuple[column])
                                })
                                .collect()
                        })
                        .collect()
                }
                ra::Operator::NaturalJoin(join) => {
                    let mut rows = Vec::new();
                    for left in &values[&join.left.to_string()] {
                        for right in &values[&join.right.to_string()] {
                            if right
                                .iter()
                                .all(|(k, v)| left.get(k).is_none_or(|w| w == v))
                            {
                                let mut row = left.clone();
                                row.extend(right.clone());
                                rows.push(row);
                            }
                        }
                    }
                    rows
                }
                ra::Operator::Semijoin(join) => values[&join.left.to_string()]
                    .iter()
                    .filter(|left| {
                        values[&join.right.to_string()].iter().any(|right| {
                            right
                                .iter()
                                .all(|(k, v)| left.get(k).is_none_or(|w| w == v))
                        })
                    })
                    .cloned()
                    .collect(),
                ra::Operator::Project(project) => values[&project.input.to_string()]
                    .iter()
                    .map(|row| {
                        project
                            .attributes
                            .iter()
                            .map(|k| (k.to_string(), row[&k.to_string()]))
                            .collect()
                    })
                    .collect(),
                _ => panic!("unexpected operator in dead-expansion bag fixture"),
            };
            values.insert(definition.result.to_string(), rows);
        }
        values[&program.plan.output.input.to_string()]
            .iter()
            .map(|row| {
                program
                    .plan
                    .output
                    .head
                    .variables()
                    .map(|k| row[&k.to_string()])
                    .collect()
            })
            .collect()
    }
    let database = [
        ("Left".into(), vec![vec![0], vec![0], vec![1], vec![2]]),
        (
            "Nation".into(),
            vec![vec![0, 7], vec![0, 7], vec![0, 8], vec![1, 9]],
        ),
        ("Region".into(), vec![vec![7], vec![7], vec![8], vec![8]]),
    ]
    .into();
    for consumer in [
        quote!(semijoin expanded with region),
        quote!(natural_join expanded with region),
    ] {
        let source = dead_nation_expansion(
            consumer,
            quote! {
                result = project supported keep {nation_key};
                output result as answer(nation_key);
            },
        );
        let target = cq_elaborate::simplify_expansions(&source).unwrap();
        ra::check_rewrite(&source, &target).unwrap();
        let before = bag(&source, &database);
        let after = bag(&target, &database);
        assert!(
            before.len() > after.len(),
            "dead expansion must stop multiplying witnesses"
        );
        assert_eq!(after, vec![vec![0], vec![0]]);
        assert_eq!(
            before.into_iter().collect::<BTreeSet<_>>(),
            after.into_iter().collect::<BTreeSet<_>>()
        );
    }
}

#[test]
fn reduced_prefix_guards_keep_independent_support_witnesses() {
    let source = cq(quote! {
        struct PrefixTuple;
        relation R(x:i32); relation Other(y:i32); relation T(x:i32,y:i32,z:i32);
        answer(z) :- R(x), Other(y), where x > y, if 10 / (*x - *y) > 0, T(x,y,z);
    });
    let written = coarse(&source);
    let steps = &written.valuation.steps;
    let reduced = vo(
        &source,
        quote! {
            reduce T@4 with Other@1(y);
            reduce R@0 with T@4(x,_,_);
            reduce T@4 with R@0(x);
            reduce Other@1 with T@4(_,y,_);
            #(#steps)*
        },
    );
    let target = compile(&reduced).unwrap();
    assert_plan(
        &target,
        quote! {
            r0 = rename T@4 { x -> x, y -> y, z -> z };
            r1 = rename Other@1 { y -> y };
            r2 = semijoin r0 with r1;
            r3 = rename R@0 { x -> x };
            r4 = semijoin r3 with r2;
            r5 = semijoin r2 with r4;
            r6 = semijoin r1 with r5;
            r7 = natural_join r4 with r6;
            r8 = select r7 if (x > y);
            r9 = guard r8 if (10 / (*x - *y) > 0);
            r10 = natural_join r9 with r5;
            r11 = project r10 keep { z };
            output r11 as answer(z);
        },
    );
}

#[test]
fn seed_997_reductions_match_equality_classes_before_rust() {
    let surface = syn::parse_str(include_str!("fixtures/fuzz_997.cq")).unwrap();
    let normal = cq::normal::normalize(&surface).unwrap();
    let written = coarse(&normal);
    let steps = &written.valuation.steps;
    // The full upward/downward tree schedule from fuzz seed 997. Both v3
    // and v4 occur in opaque Rust, so alias solving must preserve their names.
    let reduced = vo(
        &normal,
        quote! {
            reduce r1@1 with r1@0(_);
            reduce r0@2 with r1@1(_, v3);
            reduce r1@1 with r0@2(v4);
            reduce r1@0 with r1@1(_, _);
            #(#steps)*
        },
    );
    let target = compile(&reduced).unwrap();
    let reference = cq_elaborate::compile_written(&written).unwrap();
    ra::check_rewrite(&reference, &target).unwrap();
    let first_rust = target
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, ra::Operator::Guard(_)))
        .unwrap();
    let mut mutations = 0;
    for (i, definition) in target.plan.definitions[..first_rust].iter().enumerate() {
        let ra::Operator::Select(select) = &definition.operator else {
            continue;
        };
        for (j, equality) in select.condition.equalities.iter().enumerate() {
            let ra::EqualityValue::Variable(right) = &equality.right else {
                continue;
            };
            if equality.left == *right {
                continue;
            }
            let mut changed = target.clone();
            let ra::Operator::Select(select) = &mut changed.plan.definitions[i].operator else {
                unreachable!()
            };
            // Output equality alone cannot replace a reduction's own support
            // column match.
            select.condition.equalities[j].right =
                ra::EqualityValue::Variable(equality.left.clone());
            ra::contract::check(&changed).unwrap();
            let error = check(&reduced, &changed).unwrap_err();
            assert!(
                error.to_string().contains("support column matches"),
                "{error}"
            );
            mutations += 1;
        }
    }
    assert_eq!(mutations, 2, "each direction must actually match v3 and v4");
}

#[test]
fn reductions_match_every_protected_column_in_an_equality_class() {
    let source = cq(quote! {
        struct Q; relation R(a:i32,b:i32); relation S(c:i32);
        answer(x,z,y) :- R(x,z), S(y), if accept(x,z,y),
            where x == y, where z == y;
    });
    let plan = vo(
        &source,
        quote! {
            reduce R@0 with S@1(y); reduce S@1 with R@0(x,z);
            R@0(x,z); S@1(y); if accept(x,z,y); where x = y; where z = y;
        },
    );
    assert_eq!(
        plan.valuation
            .steps
            .iter()
            .filter(|step| { matches!(step, valuation::Step::Equality(..)) })
            .count(),
        2
    );
    let target = compile(&plan).unwrap();
    for form in equality_forms(&plan) {
        assert_eq!(compile(&form).unwrap(), target);
    }
    let reference = cq_elaborate::compile_written(&coarse(&source)).unwrap();
    ra::check_rewrite(&reference, &target).unwrap();
}

#[test]
fn independent_relations_commute_across_rust() {
    let source = cq(quote! {
        struct Q; relation R(a: i32); relation S(b: i32);
        answer(x, w) :- R(x), if 10 / *x > 0, S(w);
    });
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R@0 {a -> x}; s = rename S@2 {b -> w};
        j = natural_join r with s; g = guard j if (10 / *x > 0);
        output g as answer(x,w);
    }};
    check(&coarse(&source), &target).unwrap();
    let later_source = cq(quote! {
        struct Q; relation R(a: i32); relation S(b: i32);
        answer(x, w) :- R(x), S(w), if 10 / *x > 0;
    });
    let late: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R@0 {a -> x}; g = guard r if (10 / *x > 0);
        s = rename S@1 {b -> w}; j = natural_join g with s;
        output j as answer(x,w);
    }};
    ra::contract::check(&late).unwrap();
    check(&coarse(&later_source), &late).unwrap();
}

#[test]
fn rust_reordering_is_accepted_but_payload_mutations_are_rejected() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); answer(x,y) :- R(x), if { *x != 0 }, let y:i32 = 10 / *x;
    });
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R@0 {a -> x};
        b = bind r with y:i32 = (10 / *x); g = guard b if ({ *x != 0 });
        output g as answer(x,y);
    }};
    ra::contract::check(&target).unwrap();
    check(&coarse(&source), &target).unwrap();
    let mut changed = compile(&coarse(&source)).unwrap();
    let node = changed
        .plan
        .definitions
        .iter_mut()
        .find(|d| matches!(d.operator, ra::Operator::Guard(_)))
        .unwrap();
    let ra::Operator::Guard(guard) = &mut node.operator else {
        unreachable!()
    };
    guard.condition = syn::parse_quote!(*x != 1);
    assert!(check(&coarse(&source), &changed).is_err());
}

#[test]
fn elaboration_retains_the_recorded_rust_placement() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation S(b:i32);
        answer(x,y) :- R(x), S(w), if { *x != 0 && *w > 1 }, let y:i32 = 10 / *x;
    });
    let target = compile(&coarse(&source)).unwrap();
    let chain: Vec<_> = target
        .plan
        .definitions
        .iter()
        .filter(|d| matches!(d.operator, ra::Operator::Guard(_) | ra::Operator::Bind(_)))
        .collect();
    assert!(matches!(chain[0].operator, ra::Operator::Guard(_)));
    let ra::Operator::Bind(bind) = &chain[1].operator else {
        unreachable!()
    };
    assert_eq!(bind.input, chain[0].result);
}

#[test]
fn projection_cannot_discard_a_join_correlation() {
    let source = cq(quote! {
        struct Q; relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        answer(x,z) :- R(x,y), S(y,z);
    });
    let broken: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        r = rename R@0 {a -> x,b -> y}; p = project r keep {x};
        s = rename S@1 {b -> y,c -> z}; j = natural_join p with s;
        o = project j keep {x,z}; output o as answer(x,z);
    }};
    ra::contract::check(&broken).unwrap();
    assert!(
        check(&coarse(&source), &broken)
            .unwrap_err()
            .to_string()
            .contains("equality closure")
    );
}

#[test]
fn relational_equality_commutes_with_total_rust() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); answer(x) :- R(x), where x == 1, if 10 / *x > 0;
    });
    let broken: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R@0 {a -> x};
        g = guard r if (10 / *x > 0); s = select g if (x = 1);
        output s as answer(x);
    }};
    ra::contract::check(&broken).unwrap();
    check(&coarse(&source), &broken).unwrap();
}

#[test]
fn differing_literal_spellings_are_not_assumed_unequal() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); answer(x) :- R(x), where x == 1, where x == 0x1;
    });
    let target = compile(&coarse(&source)).unwrap();
    let mut changed = target.clone();
    let ra::Operator::Select(select) = &mut changed.plan.definitions[1].operator else {
        panic!()
    };
    select.condition.equalities[1].right = ra::EqualityValue::Literal(syn::parse_quote!(2));
    assert!(check(&coarse(&source), &changed).is_err());
}

#[test]
fn shared_scenarios_and_mutated_output_correspondence() {
    for case in scenarios::SCENARIOS {
        let source: valuation::Program = syn::parse_str(case.vo).unwrap();
        let target = compile(&source).unwrap();
        assert_eq!(
            target,
            syn::parse_str::<ra::Program>(case.ra).unwrap(),
            "{}",
            case.name
        );
        check(&source, &target).unwrap();
        for seed in 0..4 {
            let database = databases::binary_database(&source.cq, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                cq_oracle::evaluate_cq(&source.cq, &database),
                "{}",
                case.name
            );
        }
        let mut changed = target.clone();
        changed.plan.output.head.relation = syn::parse_quote!(other);
        assert!(check(&source, &changed).is_err());
        let mut changed = target;
        changed.plan.declarations[0].columns[0].ty = syn::parse_quote!(u32);
        assert!(check(&source, &changed).is_err());
    }
}

#[test]
fn written_reference_filters_literal_columns_before_dependent_expressions() {
    // The literal belongs to the occurrence: a mismatching row never reaches
    // the dependent binding, in either the written or optimized plan.
    let surface = cq(quote! {
        struct Q; relation R(x:i32, value:i32, tag:i32);
        answer(x) :- R(x, 10 / *x, 5);
    });
    let normal = cq::normal::normalize(&surface).unwrap();
    let source = coarse(&normal);
    let written = cq_elaborate::compile_written(&source).unwrap();
    check(&source, &written).unwrap();
    for target in [written, compile(&source).unwrap()] {
        let bind = target
            .plan
            .definitions
            .iter()
            .position(|d| matches!(d.operator, ra::Operator::Bind(_)))
            .unwrap();
        let literal = target
            .plan
            .definitions
            .iter()
            .position(|d| matches!(&d.operator, ra::Operator::Rename(r) if r.constants.is_some()))
            .unwrap();
        assert!(literal < bind);
        assert!(!target.plan.definitions.iter().any(|d|
            matches!(&d.operator, ra::Operator::Select(s) if s.condition.constant_keys().is_some())));
    }
}

#[test]
fn written_reference_rejects_a_reordered_valuation() {
    let source = cq(quote! {
        struct Q; relation R(x:i32); relation S(y:i32);
        answer(x,y) :- R(x), S(y);
    });
    let reordered = vo(&source, quote! { S@1(y); R@0(x); });
    valuation::contract::check(&reordered).unwrap();
    assert!(
        cq_elaborate::compile_written(&reordered)
            .unwrap_err()
            .to_string()
            .contains("written reference")
    );
}

#[test]
fn correspondence_accepts_independent_relational_placement() {
    let source = cq(quote! {
        struct Q; relation R(x:i32); relation S(w:i32);
        answer(x,w) :- R(x), if 10 / *x > 1, S(w);
    });
    let written = coarse(&source);
    let hoisted = vo(&source, quote! { R@0(x); S@2(w); if 10 / *x > 1; });
    valuation::contract::check(&hoisted).unwrap();
    let later = cq_elaborate::compile_written(&written).unwrap();
    // Total Rust has no written relational predecessor obligations.
    check(&written, &later).unwrap();
    check(&hoisted, &later).unwrap();
    check(&written, &compile(&hoisted).unwrap()).unwrap();
}

#[test]
fn leibniz_equality_class_can_use_either_generator_binding() {
    // Key is admissible: equality includes every field Rust can read. Retain
    // the expression's free-variable name, then solve its relational aliases.
    let surface = cq(quote! {
        struct Q; relation R(key:Key, marker:i32); relation S(key:Key);
        answer(y) :- R(x,z), S(x), let y:bool = x.flag;
    });
    let source = cq::normal::normalize(&surface).unwrap();
    let source = coarse(&source);
    let target = compile(&source).unwrap();
    let first = &target.plan.definitions[0];
    let ra::Operator::Rename(rename) = &first.operator else {
        panic!()
    };
    assert_eq!(rename.relation, "R");
    let explicit = target.plan.definitions.iter().any(|d| matches!(&d.operator, ra::Operator::Select(select) if select.condition.equalities.iter().any(|e| matches!(e.right, ra::EqualityValue::Variable(_)))));
    assert!(!explicit, "the shared join heading decides identity");
    let replacement: ra::Program = syn::parse_quote! { relational {
        relation R(key:Key, marker:i32); relation S(key:Key);
        s = rename S@1 {key -> x}; r = rename R@0 {key -> x, marker -> z};
        j = natural_join s with r; b = bind j with y:bool = (x.flag);
        p = project b keep {y}; output p as answer(y);
    }};
    ra::contract::check(&replacement).unwrap();
    check(&source, &replacement).unwrap();
}

#[test]
fn projection_before_rust_can_discard_variables_outside_its_free_variables() {
    let source = cq(quote! {
        struct Q; relation R(key:Key, marker:i32);
        answer(y) :- R(x,z), let y:bool = x.flag;
    });
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(key:Key, marker:i32);
        r = rename R@0 {key -> x, marker -> z}; p = project r keep {x};
        b = bind p with y:bool = (x.flag); out = project b keep {y};
        output out as answer(y);
    }};
    ra::contract::check(&target).unwrap();
    check(&coarse(&source), &target).unwrap();
}

#[test]
fn named_vo_reductions_cannot_be_erased_by_the_final_join() {
    let source = chain();
    let reduced = vo(
        &source,
        quote! {
            reduce R@1 with L@0(a); L@0(a); R@1(a,b); S@2(b,c);
        },
    );
    let mut target = compile(&reduced).unwrap();
    let analysis = ra::contract::check(&target).unwrap();
    let definition = target
        .plan
        .definitions
        .iter_mut()
        .find(|d| matches!(d.operator, ra::Operator::Semijoin(_)))
        .unwrap();
    let ra::Operator::Semijoin(join) = &definition.operator else {
        unreachable!()
    };
    let input = &join.left;
    let attributes = analysis[&input.to_string()].bindings.values();
    definition.operator = syn::parse_quote!(project #input keep { #(#attributes),* });
    ra::contract::check(&target).unwrap();
    // All atoms still reach the complete final join. That does not supply the
    // named reducer requested by this VO schedule.
    let error = check(&reduced, &target).unwrap_err();
    assert!(error.to_string().contains("semijoin reduction"), "{error}");
}

#[test]
fn vo_clause_identity_is_checked_but_redundant_partial_columns_can_change() {
    let source = cq(quote! {
        struct Q; relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        answer(a,c) :- L(a), if opaque(a), R(a,b), S(b,c);
    });
    let valuation = vo(
        &source,
        quote! { R@2(a,_); L@0(a); if opaque(a); R@2(a,b); S@3(b,c); },
    );
    let target = compile(&valuation).unwrap();
    for index in [0, 1] {
        let mut changed = target.clone();
        let rename = changed
            .plan
            .definitions
            .iter_mut()
            .filter_map(|d| match &mut d.operator {
                ra::Operator::Rename(r) => Some(r),
                _ => None,
            })
            .nth(index)
            .unwrap();
        rename.occurrence = syn::parse_quote!(50);
        ra::contract::check(&changed).unwrap();
        assert!(
            check(&valuation, &changed)
                .unwrap_err()
                .to_string()
                .contains("occurrence")
        );
    }
    let mut changed = target;
    let projection = changed
        .plan
        .definitions
        .iter_mut()
        .find_map(|d| match &mut d.operator {
            ra::Operator::Project(p) => Some(p),
            _ => None,
        })
        .unwrap();
    projection.attributes[0] = syn::parse_quote!(b);
    ra::contract::check(&changed).unwrap();
    // The later full R witness supplies both columns. The earlier projection
    // only restricts a domain, and opaque total Rust does not observe it.
    check(&valuation, &changed).unwrap();
}

#[test]
fn relational_conjuncts_keep_their_independent_selection_domains() {
    let source = cq(quote! {
        struct Q; relation R(a:i32,b:i32,c:i32);
        answer(x,y,z) :- R(x,y,z), where x < y && y < z;
    });
    let vo = coarse(&source);
    let target = compile(&vo).unwrap();
    let conditions: Vec<_> = target
        .plan
        .definitions
        .iter()
        .filter_map(|definition| match &definition.operator {
            ra::Operator::Select(selection) => Some(&selection.condition),
            _ => None,
        })
        .collect();
    assert_eq!(conditions.len(), 2);
    assert_eq!(conditions[0].constraints.len(), 1);
    assert_eq!(conditions[1].constraints.len(), 1);
    assert_eq!(conditions[0].constraints[0], syn::parse_quote!(x < y));
    assert_eq!(conditions[1].constraints[0], syn::parse_quote!(y < z));
    check(&vo, &target).unwrap();
}

#[test]
fn literal_points_precede_membership_filters_without_type_classification() {
    for ty in ["String", "StringAlias"] {
        let ty: syn::Type = syn::parse_str(ty).unwrap();
        let source = cq(quote! {
            struct Q; relation Lineitem(mode:#ty,instruction:#ty);
            answer(mode,instruction) :- Lineitem(mode,instruction),
                where (mode == "AIR" || mode == "AIR REG")
                    && instruction == "DELIVER IN PERSON";
        });
        let vo = coarse(&source);
        let target = compile(&vo).unwrap();
        let selections: Vec<_> = target
            .plan
            .definitions
            .iter()
            .filter_map(|definition| match &definition.operator {
                ra::Operator::Select(selection) => Some(selection),
                _ => None,
            })
            .collect();
        assert_eq!(selections.len(), 2);
        let point = selections[0].condition.constant_keys().unwrap();
        assert_eq!(point.len(), 1);
        assert_eq!(point["instruction"], syn::parse_quote!("DELIVER IN PERSON"));
        assert!(target.plan.definitions.iter().any(|definition| {
            definition.result == selections[0].input
                && matches!(definition.operator, ra::Operator::Rename(_))
        }));
        assert!(matches!(
            selections[1].condition.constraints[0],
            cq::Constraint::In { .. }
        ));
        check(&vo, &target).unwrap();
    }
}

#[test]
fn derived_domains_filter_binders_but_preserve_the_exact_disjunction() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation S(b:i32);
        answer(x,y) :- R(x), S(y), where (x == 1 && y >= 2) || (x == 3 && y <= 4);
    });
    let vo = coarse(&source);
    let target = compile(&vo).unwrap();
    let selections: Vec<_> = target
        .plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Select(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(selections.len(), 3);
    assert_eq!(
        selections
            .iter()
            .filter(|s| target
                .plan
                .definitions
                .iter()
                .any(|d| d.result == s.input && matches!(d.operator, ra::Operator::Rename(_))))
            .count(),
        2
    );
    for seed in 0..16 {
        let database = databases::binary_database(&source, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            cq_oracle::evaluate_cq(&source, &database)
        );
    }
    let mut changed = target.clone();
    let ra::Operator::Select(selection) = &mut changed
        .plan
        .definitions
        .iter_mut()
        .find(|d| matches!(d.operator, ra::Operator::Select(_)))
        .unwrap()
        .operator
    else {
        panic!()
    };
    selection.condition.constraints[0] = syn::parse_quote!(x > 99);
    assert!(check(&vo, &changed).is_err());
    let mut changed = target;
    let last = changed
        .plan
        .definitions
        .iter_mut()
        .rev()
        .find(|d| matches!(d.operator, ra::Operator::Select(_)))
        .unwrap();
    let ra::Operator::Select(selection) = &last.operator else {
        panic!()
    };
    let input = &selection.input;
    last.operator = syn::parse_quote!(project #input keep {x,y});
    assert!(check(&vo, &changed).is_err());
}

#[test]
fn opaque_method_and_function_guards_keep_their_written_prefix() {
    for guard in ["if name.contains(\"green\")", "if custom(name)"] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:i32); relation P(k:i32,n:String); answer(x,name) :- R(x), P(x,name), {guard};"
        )).unwrap();
        let vo = coarse(&source);
        let target = compile(&vo).unwrap();
        let guard = target
            .plan
            .definitions
            .iter()
            .find_map(|d| match &d.operator {
                ra::Operator::Guard(g) => Some(g),
                _ => None,
            })
            .unwrap();
        assert!(
            target
                .plan
                .definitions
                .iter()
                .any(|d| d.result == guard.input
                    && matches!(d.operator, ra::Operator::NaturalJoin(_)))
        );
        assert!(
            !target
                .plan
                .definitions
                .iter()
                .any(|d| matches!(d.operator, ra::Operator::Select(_)))
        );
        assert_eq!(
            syn::parse2::<ra::Program>(target.to_token_stream()).unwrap(),
            target
        );
    }
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation S(b:i32);
        answer(x,y) :- R(x), S(y), if 10 / *x > 1;
    });
    let target = compile(&coarse(&source)).unwrap();
    let guard = target
        .plan
        .definitions
        .iter()
        .find_map(|d| match &d.operator {
            ra::Operator::Guard(g) => Some(g),
            _ => None,
        })
        .unwrap();
    assert!(
        target
            .plan
            .definitions
            .iter()
            .any(|d| d.result == guard.input && matches!(d.operator, ra::Operator::NaturalJoin(_)))
    );
    let mut forged = target.clone();
    let step = forged
        .plan
        .definitions
        .iter_mut()
        .find(|d| matches!(d.operator, ra::Operator::Guard(_)))
        .unwrap();
    let input = &guard.input;
    step.operator = syn::parse_quote!(select #input if (x > 1));
    ra::contract::check(&forged).unwrap();
    assert!(
        check(&coarse(&source), &forged).is_err(),
        "a target cannot replace an opaque Rust guard with a relational predicate"
    );
}

#[test]
fn opaque_multi_atom_guard_runs_once_its_free_variables_are_available() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation S(b:i32); relation T(c:i32);
        answer(x,y,z) :- R(x), S(y), T(z), if compatible(x,y);
    });
    let planned = cq_order::compile(
        &source,
        &cq_order::Options {
            heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
            split: cq_order::Split::Coarse,
        },
    )
    .unwrap();
    let position = planned
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, valuation::Step::Guard(..)))
        .unwrap();
    assert_eq!(position, 2);
    assert!(
        matches!(&planned.valuation.steps[3], valuation::Step::Atom(atom, _) if atom.relation == "T")
    );
    let target = compile(&planned).unwrap();
    let select = target
        .plan
        .definitions
        .iter()
        .find_map(|d| match &d.operator {
            ra::Operator::Guard(s) => Some(s),
            _ => None,
        })
        .unwrap();
    assert!(target.plan.definitions.iter().any(|d| d.result == select.input && matches!(d.operator, ra::Operator::NaturalJoin(_))));
}

#[test]
fn independent_branch_is_exact_projected_and_checked() {
    let source = cq(quote! {
        struct Q;
        relation L(o:i32,p:i32,s:i32); relation S(s:i32,n:i32);
        relation N(n:i32,label:i32);
        answer(o,p,s) :- L(o,p,s), S(s,n), N(n,label), where label == 7;
    });
    let grouped: valuation::Program = syn::parse2(quote! {
        #source valuation branches { branch [1,2] (s); } {
            L@0(o,p,s); S@1(s,n); N@2(n,label); where label = 7;
        }
    })
    .unwrap();
    assert!(matches!(
        grouped.valuation.steps.last(),
        Some(valuation::Step::Equality(..))
    ));
    let target = compile(&grouped).unwrap();
    for form in equality_forms(&grouped) {
        assert_eq!(compile(&form).unwrap(), target);
    }
    assert!(target.plan.definitions.iter().any(|d| matches!(&d.operator, ra::Operator::Project(p) if p.attributes.iter().map(ToString::to_string).collect::<Vec<_>>() == ["s"])));
    let flat = compile(&coarse(&source)).unwrap();
    assert!(check(&grouped, &flat).is_err());
    assert!(cq_elaborate::compile_written(&grouped).is_err());
    for seed in 0..32 {
        let database = databases::binary_database(&source, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            oracle::evaluate_ra(&flat, &database).output
        );
    }
}

#[test]
fn grouped_branch_preserves_multi_atom_predicates_and_duplicate_witnesses() {
    let source = cq(quote! {
        struct Q;
        relation L(o:i32,p:i32,s:i32); relation S(s:i32,n:i32);
        relation N(n:i32,k:i32);
        answer(o,p,s) :- L(o,p,s), S(s,n), N(n,k), where n < k;
    });
    let grouped: valuation::Program = syn::parse2(quote! {
        #source valuation branches { branch [1,2] (s); } {
            L@0(o,p,s); S@1(s,n); N@2(n,k); where n < k;
        }
    })
    .unwrap();
    let target = compile(&grouped).unwrap();
    let flat = compile(&coarse(&source)).unwrap();
    for seed in 0..32 {
        let database = databases::binary_database(&source, seed);
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            oracle::evaluate_ra(&flat, &database).output
        );
    }
}

#[test]
fn dimension_subtrees_build_before_their_parent_without_cartesian_children() {
    let source = cq(quote! {
        struct Q;
        relation L(o:i32,p:i32,s:i32); relation PS(p:i32,s:i32);
        relation P(p:i32); relation S(s:i32,n:i32); relation N(n:i32); relation O(o:i32);
        answer(o,p,s) :- L(o,p,s), PS(p,s), P(p), S(s,n), N(n), O(o);
    });
    let grouped: valuation::Program = syn::parse2(quote! {
        #source valuation branches { branch [1,2,3,4] (p,s); } {
            L@0(o,p,s); PS@1(p,s); P@2(p); S@3(s,n); N@4(n); O@5(o);
        }
    })
    .unwrap();
    let target = compile(&grouped).unwrap();
    let facts = ra::contract::check(&target).unwrap();
    let supplier = target
        .plan
        .definitions
        .iter()
        .find(|d| matches!(&d.operator, ra::Operator::Rename(r) if r.relation == "S"))
        .unwrap();
    let nation = target
        .plan
        .definitions
        .iter()
        .find(|d| matches!(&d.operator, ra::Operator::Rename(r) if r.relation == "N"))
        .unwrap();
    assert!(target.plan.definitions.iter().any(|d| matches!(&d.operator, ra::Operator::NaturalJoin(j) if j.left == supplier.result && j.right == nation.result)));
    for definition in &target.plan.definitions {
        if let ra::Operator::NaturalJoin(join) = &definition.operator {
            assert!(
                facts[&join.left.to_string()]
                    .bindings
                    .keys()
                    .any(|v| facts[&join.right.to_string()].bindings.contains_key(v))
            );
        }
    }
}

#[test]
fn generator_elaboration_round_trips_and_certifies_set_valued_binding() {
    let source = coarse(&cq(quote! {
        struct Q; relation R(a: i32); relation answer(b: i32);
        answer(b) :- R(a), for b in 0..a, let c = b;
    }));
    let target = compile(&source).unwrap();
    cq_elaborate::compile_written(&source).unwrap();
    assert_eq!(
        syn::parse2::<ra::Program>(target.to_token_stream()).unwrap(),
        target
    );
    let generator = target
        .plan
        .definitions
        .iter()
        .position(|d| matches!(&d.operator, ra::Operator::Bind(b) if b.for_token.is_some()))
        .unwrap();
    let mut changed = target.clone();
    let ra::Operator::Bind(binding) = &mut changed.plan.definitions[generator].operator else {
        unreachable!()
    };
    binding.for_token = None;
    assert!(check(&source, &changed).is_err());
    let mut changed = target.clone();
    let ra::Operator::Bind(binding) = &mut changed.plan.definitions[generator].operator else {
        unreachable!()
    };
    binding.expression = syn::parse_quote!(0..100);
    assert!(check(&source, &changed).is_err());
}

#[test]
fn literal_occurrences_drop_exact_source_positions_and_reject_changed_values() {
    for body in [
        quote!(R(7, x, -2)),
        quote!(R(x, 7, -2)),
        quote!(R(7, -2, x)),
    ] {
        let source = cq(quote! {
            struct Q; relation R(a:i32,b:i32,c:i32);
            answer(x) :- #body;
        });
        let normal = cq::normal::normalize(&source).unwrap();
        assert_eq!(normal, source);
        let planned = coarse(&normal);
        let target = compile(&planned).unwrap();
        let rename = target
            .plan
            .definitions
            .iter()
            .find_map(|d| match &d.operator {
                ra::Operator::Rename(r) => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(rename.mapping.len(), 1);
        assert_eq!(rename.constants.as_ref().unwrap().equalities.len(), 2);
        assert!(
            !target
                .plan
                .definitions
                .iter()
                .any(|d| matches!(d.operator, ra::Operator::Select(_) | ra::Operator::Guard(_)))
        );
        assert_eq!(
            syn::parse2::<ra::Program>(target.to_token_stream()).unwrap(),
            target
        );
        let mut changed = target.clone();
        let rename = changed
            .plan
            .definitions
            .iter_mut()
            .find_map(|d| match &mut d.operator {
                ra::Operator::Rename(r) => Some(r),
                _ => None,
            })
            .unwrap();
        rename.constants.as_mut().unwrap().equalities[0].right =
            ra::EqualityValue::Literal(syn::parse_quote!(99));
        ra::contract::check(&changed).unwrap();
        assert!(check(&planned, &changed).is_err());
    }
    let source = cq(quote! {
        struct Q; relation R(a:i32,b:bool,c:&str); relation answer(value:i32);
        answer(1) :- R(-7,true,"fixed");
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let target = compile(&coarse(&normal)).unwrap();
    let rows = ra::contract::check(&target).unwrap();
    let occurrence = target
        .plan
        .definitions
        .iter()
        .find(|definition| matches!(definition.operator, ra::Operator::Rename(_)))
        .unwrap();
    assert!(rows[&occurrence.result.to_string()].heading.is_empty());
}

#[test]
fn negation_elaborates_direct_host_keys_without_redundant_projection() {
    let source = cq(quote! {
        struct Negative; relation R(a:i32,b:i32); relation K(a:i32,b:i32);
        answer(x,y) :- R(x,y), !K(x,y);
    });
    let target = compile(&coarse(&source)).unwrap();
    assert_plan(
        &target,
        quote! {
            r0 = rename R@0 { a -> x, b -> y };
            r1 = rename K@1 { a -> x, b -> y };
            r2 = anti [x,y] r0, r1;
            r3 = project r2 keep { x,y };
            output r3 as answer(x,y);
        },
    );
}

#[test]
fn negative_constants_repeats_and_wildcards_match_reference_semantics() {
    for args in ["x,x", "x,1", "1,x", "x,_", "_,_", "1,_"] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct Negative; relation R(a:i32); relation K(a:i32,b:i32);
             answer(x) :- R(x), !K({args});"
        ))
        .unwrap();
        let written = coarse(&source);
        let target = compile(&written).unwrap();
        let reference = cq_elaborate::compile_written(&written).unwrap();
        let anti = target
            .plan
            .definitions
            .iter()
            .find_map(|d| match &d.operator {
                ra::Operator::Anti(a) => Some(a),
                _ => None,
            })
            .unwrap();
        assert_eq!(anti.columns.len(), usize::from(args.contains('x')));
        let right = &target
            .plan
            .definitions
            .iter()
            .find(|d| d.result == anti.right)
            .unwrap()
            .operator;
        if matches!(args, "x,_" | "_,_") {
            assert!(matches!(right, ra::Operator::Rename(_)), "{args}");
        } else {
            assert!(matches!(right, ra::Operator::Project(_)), "{args}");
        }
        for seed in 0..32 {
            let database = databases::binary_database(&source, seed);
            let expected = cq_oracle::evaluate_cq(&source, &database);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                expected,
                "{args}, {seed}"
            );
            assert_eq!(
                oracle::evaluate_ra(&reference, &database).output,
                expected,
                "{args}, {seed}"
            );
        }
        let mut changed = target.clone();
        let step = changed
            .plan
            .definitions
            .iter_mut()
            .find(|d| matches!(d.operator, ra::Operator::Anti(_)))
            .unwrap();
        let ra::Operator::Anti(anti) = &step.operator else {
            unreachable!()
        };
        let input = &anti.left;
        step.operator = syn::parse_quote!(project #input keep {x});
        assert!(check(&written, &changed).is_err(), "{args}");
    }
}

#[test]
fn raw_negation_names_match_reference_semantics() {
    for relation in ["K", "r#K"] {
        for args in ["r#x,x", "x,r#x", "r#x,1", "1,r#x", "r#x,_", "_,r#x"] {
            for body in [
                format!("R(x), !{relation}({args})"),
                format!("R(x), {{ !{relation}({args}) }} | {{ !{relation}({args}) }}"),
            ] {
                let source: cq::Program = syn::parse_str(&format!(
                    "struct Raw; relation r#R(a:i32); relation r#K(a:i32,b:i32);
                     answer(x) :- {body};"
                ))
                .unwrap();
                let normal = cq::normal::normalize(&source).unwrap();
                let written = cq_order::compile(
                    &normal,
                    &cq_order::Options {
                        heuristic: cq_order::Heuristic::Written,
                        split: cq_order::Split::Coarse,
                    },
                )
                .unwrap();
                let target = compile(&written).unwrap();
                let reference_vo = cq_order::compile_written(&normal).unwrap();
                let reference = cq_elaborate::compile_written(&reference_vo).unwrap();
                check(&reference_vo, &reference).unwrap();
                for seed in 0..32 {
                    let database = databases::binary_database(&source, seed);
                    let expected = cq_oracle::evaluate_cq(&source, &database);
                    assert_eq!(
                        oracle::evaluate_ra(&target, &database).output,
                        expected,
                        "{body}, {seed}"
                    );
                    assert_eq!(
                        oracle::evaluate_ra(&reference, &database).output,
                        expected,
                        "{body}, {seed}"
                    );
                }
            }
        }
    }
}

#[test]
fn parameter_negation_selects_a_capture_without_binding_it() {
    let source = cq(quote! {
        struct Negative; relation R(a:i32); relation K(a:i32,b:i32);
        answer(x) :- R(x), !K(x, cutoff);
    });
    let target = compile(&coarse(&source)).unwrap();
    let selection = target
        .plan
        .definitions
        .iter()
        .find_map(|d| match &d.operator {
            ra::Operator::Select(s) => Some(s),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        selection.condition.constraints[0],
        syn::parse_quote!(__cq_neg_1 = @(cutoff))
    );
}

#[test]
fn all_wildcard_negation_can_seed_an_empty_heading() {
    let source = cq(quote! {
        struct Empty; relation K(a:i32,b:i32); relation Seed(x:i32);
        answer(x) :- !K(_,_), Seed(x);
    });
    let target = compile(&coarse(&source)).unwrap();
    for rows in [std::collections::BTreeSet::new(), [vec![1, 2]].into()] {
        let database: oracle::Database =
            [("K".into(), rows), ("Seed".into(), [vec![1]].into())].into();
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            cq_oracle::evaluate_cq(&source, &database)
        );
    }
}

#[test]
fn native_anti_obligations_reject_changed_keys_inputs_filters_and_polarity() {
    let source = cq(quote! {
        struct Negative; relation R(a:i32); relation K(a:i32,b:i32); relation Other(a:i32,b:i32);
        answer(x) :- R(x), !K(x,1);
    });
    let written = coarse(&source);
    let target = compile(&written).unwrap();
    for mutation in 0..5 {
        let mut changed = target.clone();
        match mutation {
            0 => {
                let anti = changed
                    .plan
                    .definitions
                    .iter_mut()
                    .find_map(|d| match &mut d.operator {
                        ra::Operator::Anti(a) => Some(a),
                        _ => None,
                    })
                    .unwrap();
                anti.columns.clear();
            }
            1 => {
                let rename = changed
                    .plan
                    .definitions
                    .iter_mut()
                    .find_map(|d| match &mut d.operator {
                        ra::Operator::Rename(r) if r.relation == "K" => Some(r),
                        _ => None,
                    })
                    .unwrap();
                rename.relation = syn::parse_quote!(Other);
            }
            2 => {
                let selection = changed
                    .plan
                    .definitions
                    .iter_mut()
                    .find_map(|d| match &mut d.operator {
                        ra::Operator::Select(s) => Some(s),
                        _ => None,
                    })
                    .unwrap();
                selection.condition.equalities[0].right =
                    ra::EqualityValue::Literal(syn::parse_quote!(2));
            }
            _ => {
                let step = changed
                    .plan
                    .definitions
                    .iter_mut()
                    .find(|d| matches!(d.operator, ra::Operator::Anti(_)))
                    .unwrap();
                let ra::Operator::Anti(anti) = &step.operator else {
                    unreachable!()
                };
                let (left, right) = (&anti.left, &anti.right);
                step.operator = if mutation == 3 {
                    syn::parse_quote!(semijoin #left with #right)
                } else {
                    syn::parse_quote!(project #left keep {x})
                };
            }
        }
        if mutation == 4 {
            let mut live =
                std::collections::BTreeSet::from([changed.plan.output.input.to_string()]);
            for definition in changed.plan.definitions.iter().rev() {
                if live.contains(&definition.result.to_string()) {
                    match &definition.operator {
                        ra::Operator::Project(project) => {
                            live.insert(project.input.to_string());
                        }
                        ra::Operator::Rename(_) => {}
                        _ => unreachable!("removing the anti leaves only its positive input"),
                    }
                }
            }
            changed
                .plan
                .definitions
                .retain(|definition| live.contains(&definition.result.to_string()));
        }
        ra::contract::check(&changed).unwrap();
        assert!(check(&written, &changed).is_err(), "mutation {mutation}");
    }
}

#[test]
fn native_anti_proof_allows_positive_join_and_total_rust_reordering() {
    let source = cq(quote! {
        struct Negative; relation R(a:i32); relation S(b:i32); relation K(a:i32);
        answer(x,y) :- R(x), S(y), !K(x), if opaque(x);
    });
    let written = coarse(&source);
    let reordered = vo(
        &source,
        quote! {
            S@1(y); R@0(x); !K(x); if opaque(x);
        },
    );
    let target = compile(&reordered).unwrap();
    check(&written, &target).unwrap();
    let mut delayed = target.clone();
    let guard_position = delayed
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, ra::Operator::Guard(_)))
        .unwrap();
    let anti_position = delayed
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, ra::Operator::Anti(_)))
        .unwrap();
    let ra::Operator::Anti(anti) = &delayed.plan.definitions[anti_position].operator else {
        unreachable!()
    };
    let (left, right, keys) = (anti.left.clone(), anti.right.clone(), anti.columns.clone());
    let ra::Operator::Guard(guard) = &delayed.plan.definitions[guard_position].operator else {
        unreachable!()
    };
    let mut guard = guard.clone();
    guard.input = left;
    let earlier = delayed.plan.definitions[anti_position].result.clone();
    delayed.plan.definitions[anti_position].operator = ra::Operator::Guard(guard);
    delayed.plan.definitions[guard_position].operator =
        syn::parse_quote!(anti [#keys] #earlier, #right);
    ra::contract::check(&delayed).unwrap();
    check(&written, &delayed).unwrap();
}
