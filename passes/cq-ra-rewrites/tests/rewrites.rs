//! Checked RA′ rewrites: redundant-support absorption and dead-expansion
//! elimination preserve the lowered program's answers under canon.
use cq_testkit::{corpus as scenarios, cq_semantics as cq_oracle, databases, semantics as oracle};

use cq_lang_ra as ra;
use cq_lang_vo as valuation;
use cq_lang_vo::cq;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn compile(source: &valuation::Program) -> syn::Result<ra::Program> {
    let (target, steps) = cq_relational_lowering::compile(source)?;
    cq_relational_lowering::check(source, &target, &steps)?;
    Ok(target)
}

fn check(source: &valuation::Program, target: &ra::Program) -> syn::Result<()> {
    let (_, steps) = cq_relational_lowering::compile(source)?;
    cq_relational_lowering::check(source, target, &steps)
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
        r0 = rename r {k -> k,v -> v};
        r1 = rename l {k -> k};
        r2 = semijoin r0 with r1;
        r3 = semijoin r1 with r2;
        r4 = natural_join r3 with r2;
        output r4 as answer(k,v);
    }};
    check(&reduced, &source).unwrap();
    let target = cq_ra_rewrites::simplify_reductions(&source).unwrap();
    let expected: ra::Program = syn::parse_quote! { relational {
        relation l(k:i32); relation r(k:i32,v:i32);
        r0 = rename r {k -> k,v -> v};
        r1 = rename l {k -> k};
        r4 = natural_join r1 with r0;
        output r4 as answer(k,v);
    }};
    assert_eq!(target.to_token_stream().to_string(), expected.to_token_stream().to_string());
    let (reference, _) = cq_relational_lowering::compile(&coarse(&query)).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&reference, &target).unwrap();
    // Absorption is an RA rewrite, not permission to omit a named VO reduction.
    assert!(check(&reduced, &target).is_err());
    let repeated = cq_ra_rewrites::simplify_reductions(&target).unwrap();
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
    let target = cq_ra_rewrites::simplify_reductions(&source).unwrap();
    // This is the final external-planner contract, not just the local
    // source-to-target absorption certificate.
    let analysis = valuation::contract::check(&written).unwrap();
    cq_relational_lowering::check_reference(&written.cq, &written.valuation.steps, &analysis, &target).unwrap();
    let (reference, _) = cq_relational_lowering::compile(&written).unwrap();
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
    let expected: ra::Program = syn::parse_quote! { relational {
        relation A(first:i32 [0],second:i32 [0]);
        relation B(first:i32 [0],second:i32 [0]);
        relation C(first:i32 [0],second:i32 [0]);
        relation Stock(depot:i32 [0],item:i32 [0],day:i32 [0]);
        r0 = rename Stock {depot -> d,item -> i,day -> t};
        r1 = rename C {first -> d,second -> t};
        r2 = semijoin r0 with r1;
        r3 = rename B {first -> i,second -> t};
        r4 = semijoin r2 with r3;
        r5 = rename A {first -> d,second -> i};
        r6 = semijoin r4 with r5;
        r7 = semijoin r5 with r6;
        r8 = semijoin r3 with r6;
        r9 = semijoin r1 with r6;
        r10 = natural_join r7 with r8;
        r11 = natural_join r10 with r9;
        r12 = natural_join r11 with r6;
        r13 = project r12 keep {d,i,t};
        output r13 as answer(d,i,t);
    }};
    assert_eq!(target.to_token_stream().to_string(), expected.to_token_stream().to_string());
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
    let mut split_witness = source.clone();
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
        r0 = rename r {k -> k,v -> v};
        r1 = rename l {k -> k};
        support = select r1 if (k > 0);
        r2 = semijoin r0 with support;
        r3 = semijoin support with r2;
        r4 = natural_join r3 with r2;
        output r4 as answer(k,v);
    }};
    let target = cq_ra_rewrites::simplify_reductions(&source).unwrap();
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
        r = rename R {x -> x,y -> y};
        t = rename T {y -> y};
        u = rename U {x -> x};
        reduced = semijoin r with t;
        result = natural_join u with reduced;
        output result as answer(x,y);
    }};
    let target = cq_ra_rewrites::simplify_reductions(&source).unwrap();
    assert_eq!(target, source);
    let (reference, _) = cq_relational_lowering::compile(&coarse(&query)).unwrap();
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
            r = rename R {x -> x,y -> y};
            t = rename T {x -> x,y -> y};
            reduced = semijoin r with t;
            projected = project t keep {x};
            alias = project projected keep {x};
            result = #consumer;
            output result as #head;
        }};
        let target = cq_ra_rewrites::simplify_reductions(&source).unwrap();
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
        r = rename R {x -> x};
        s = rename S {x -> x};
        t = rename T {x -> x};
        left = semijoin r with s;
        right = semijoin t with left;
        result = natural_join left with right;
        output result as answer(x);
    }};
    let target = cq_ra_rewrites::simplify_reductions(&source).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    let invalid: ra::Program = syn::parse_quote! { relational {
        relation R(x:i32); relation S(x:i32); relation T(x:i32);
        r = rename R {x -> x};
        s = rename S {x -> x};
        t = rename T {x -> x};
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

#[test]
fn absorption_checker_accepts_linear_pipes_but_policy_requires_a_direct_consumer() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation P(k:i32,v:i32); relation L(k:i32); relation Q(v:i32,w:i32);
        p = rename P {k -> k,v -> v};
        l = rename L {k -> k};
        q = rename Q {v -> v,w -> w};
        d = semijoin p with l;
        filtered = select d if (v > 0);
        extended = natural_join q with filtered;
        checked = semijoin extended with q;
        result = natural_join checked with l;
        output result as answer(k,v,w);
    }};
    let expected: ra::Program = syn::parse_quote! { relational {
        relation P(k:i32,v:i32); relation L(k:i32); relation Q(v:i32,w:i32);
        p = rename P {k -> k,v -> v};
        l = rename L {k -> k};
        q = rename Q {v -> v,w -> w};
        filtered = select p if (v > 0);
        extended = natural_join q with filtered;
        checked = semijoin extended with q;
        result = natural_join checked with l;
        output result as answer(k,v,w);
    }};
    let actual = cq_ra_rewrites::simplify_reductions(&source).unwrap();
    assert_eq!(actual.to_token_stream().to_string(), source.to_token_stream().to_string());
    ra::check_rewrite(&source, &expected).unwrap();
    let mut reordered = expected;
    reordered.plan.definitions.swap(0, 2);
    ra::check_rewrite(&source, &reordered).unwrap();
    assert_eq!(cq_ra_rewrites::simplify_reductions(&actual).unwrap(), actual);
}

#[test]
fn direct_absorption_policy_does_not_cross_even_nonmultiplying_operators() {
    for (extra, pipe, bypassed_pipe) in [
        (quote!(), quote!(select d if (v > 0)), quote!(select p if (v > 0))),
        (
            quote!(q = rename Q {v -> v};),
            quote!(semijoin d with q),
            quote!(semijoin p with q),
        ),
    ] {
        let source: ra::Program = syn::parse2(quote! { relational {
            relation P(k:i32,v:i32); relation L(k:i32); relation Q(v:i32);
            p = rename P {k -> k,v -> v};
            l = rename L {k -> k};
            #extra
            d = semijoin p with l;
            pipe = #pipe;
            result = natural_join pipe with l;
            output result as answer(k,v);
        }}).unwrap();
        let legal: ra::Program = syn::parse2(quote! { relational {
            relation P(k:i32,v:i32); relation L(k:i32); relation Q(v:i32);
            p = rename P {k -> k,v -> v};
            l = rename L {k -> k};
            #extra
            pipe = #bypassed_pipe;
            result = natural_join pipe with l;
            output result as answer(k,v);
        }}).unwrap();
        let actual = cq_ra_rewrites::simplify_reductions(&source).unwrap();
        assert_eq!(actual.to_token_stream().to_string(), source.to_token_stream().to_string());
        ra::check_rewrite(&source, &legal).unwrap();
    }
}

#[test]
fn direct_absorption_bypasses_all_shared_consumers_together() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation P(k:i32,v:i32); relation L(k:i32);
        p = rename P {k -> k,v -> v};
        l = rename L {k -> k};
        d = semijoin p with l;
        a = natural_join d with l;
        b = natural_join l with d;
        result = natural_join a with b;
        output result as answer(k,v);
    }};
    let expected: ra::Program = syn::parse_quote! { relational {
        relation P(k:i32,v:i32); relation L(k:i32);
        p = rename P {k -> k,v -> v};
        l = rename L {k -> k};
        a = natural_join p with l;
        b = natural_join l with p;
        result = natural_join a with b;
        output result as answer(k,v);
    }};
    let actual = cq_ra_rewrites::simplify_reductions(&source).unwrap();
    assert_eq!(actual.to_token_stream().to_string(), expected.to_token_stream().to_string());
    ra::check_rewrite(&source, &expected).unwrap();

    let blocked: ra::Program = syn::parse_quote! { relational {
        relation P(k:i32,v:i32); relation L(k:i32);
        p = rename P {k -> k,v -> v};
        l = rename L {k -> k};
        d = semijoin p with l;
        a = natural_join d with l;
        b = select d if (v > 0);
        result = natural_join a with b;
        output result as answer(k,v);
    }};
    assert_eq!(cq_ra_rewrites::simplify_reductions(&blocked).unwrap(), blocked);
}

#[test]
fn absorption_stops_at_canonicalization_and_shared_pipes() {
    for pipe in [
        quote!(pipe = project d keep {k,v};),
        quote!(pipe = natural_join d with d;),
    ] {
        let source: ra::Program = syn::parse2(quote! { relational {
            relation P(k:i32,v:i32); relation L(k:i32);
            p = rename P {k -> k,v -> v};
            l = rename L {k -> k};
            d = semijoin p with l;
            #pipe
            result = natural_join pipe with l;
            output result as answer(k,v);
        }}).unwrap();
        assert_eq!(
            cq_ra_rewrites::simplify_reductions(&source).unwrap().to_token_stream().to_string(),
            source.to_token_stream().to_string()
        );
    }
}

#[test]
fn absorption_requires_every_constrained_column_at_the_later_witness() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation P(k:i32,v:i32); relation L(k:i32,v:i32);
        p = rename P {k -> k,v -> v};
        l = rename L {k -> k,v -> v};
        d = semijoin p with l;
        pipe = project d keep {k};
        result = natural_join pipe with l;
        output result as answer(k,v);
    }};
    assert_eq!(cq_ra_rewrites::simplify_reductions(&source).unwrap(), source);
    let invalid: ra::Program = syn::parse_quote! { relational {
        relation P(k:i32,v:i32); relation L(k:i32,v:i32);
        p = rename P {k -> k,v -> v};
        l = rename L {k -> k,v -> v};
        pipe = project p keep {k};
        result = natural_join pipe with l;
        output result as answer(k,v);
    }};
    assert!(ra::check_rewrite(&source, &invalid).is_err());
}

#[test]
fn absorption_does_not_cross_opaque_or_negative_consumers() {
    for consumer in [
        quote!(pipe = guard d if (*v > 0);),
        quote!(pipe = anti [k] d, l;),
    ] {
        let source: ra::Program = syn::parse2(quote! { relational {
            relation P(k:i32,v:i32); relation L(k:i32);
            p = rename P {k -> k,v -> v};
            l = rename L {k -> k};
            d = semijoin p with l;
            #consumer
            result = natural_join pipe with l;
            output result as answer(k,v);
        }}).unwrap();
        assert_eq!(cq_ra_rewrites::simplify_reductions(&source).unwrap(), source);
    }
}

fn dead_nation_expansion(region_consumer: TokenStream, tail: TokenStream) -> ra::Program {
    syn::parse2(quote! { relational {
        relation Left(nation_key:i32); relation Nation(nation_key:i32,region_key:i32);
        relation Region(region_key:i32);
        left = rename Left {nation_key -> nation_key};
        nation = rename Nation {nation_key -> nation_key,region_key -> region_key};
        region = rename Region {region_key -> region_key};
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
        let target = cq_ra_rewrites::simplify_expansions(&source).unwrap();
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
        assert_eq!(
            cq_ra_rewrites::simplify_expansions(&target).unwrap(),
            target
        );
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
    let target = cq_ra_rewrites::simplify_expansions(&source).unwrap();
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
    let target = cq_ra_rewrites::simplify_expansions(&source).unwrap();
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
        left = rename Left {nation_key -> nation_key};
        nation = rename Nation {nation_key -> nation_key,region_key -> region_key};
        region = rename Region {region_key -> region_key};
        restricted_region = select region if (region_key > 0);
        reduced_nation = semijoin nation with region;
        witnessed = semijoin left with reduced_nation;
        expanded = natural_join witnessed with reduced_nation;
        supported = semijoin expanded with restricted_region;
        result = project supported keep {nation_key};
        output result as answer(nation_key);
    }};
    let target = cq_ra_rewrites::simplify_expansions(&source).unwrap();
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
        let target = cq_ra_rewrites::simplify_expansions(&source).unwrap();
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
