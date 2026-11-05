//! Cross-pass checks live here so each pass keeps only its two language dependencies.
use cq_lang_cq as cq;
use cq_lang_ix as ix;
use cq_lang_physical as physical;
use cq_lang_ra as ra;
use cq_lang_ra::syntax::{RelationDecl, symbol_name};
use cq_testkit::{cases, semantics as oracle, values};

#[test]
fn range_filters_cross_all_stages_and_new_obligations_reject_mutations() {
    use quote::ToTokens;
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(k:i32,v:i32);
        answer(x,y) :- R(x,y), where x >= lower, where x < upper;
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let vo = cq_order::compile(&normal, &Default::default()).unwrap();
    cq_order::check(&normal, &vo, &Default::default()).unwrap();
    let mut missing = vo.clone();
    missing.valuation.steps.pop();
    assert!(cq_lang_vo::contract::check(&missing).is_err());
    let mut duplicate = vo.clone();
    duplicate
        .valuation
        .steps
        .push(vo.valuation.steps[1].clone());
    assert!(cq_lang_vo::contract::check(&duplicate).is_err());
    // A restriction after the occurrence's read is not a restriction.
    let mut unbound = vo.clone();
    unbound.valuation.steps.swap(1, 2);
    assert!(cq_lang_vo::contract::check(&unbound).is_err());
    let ra = cq_relational_lowering::compile(&vo).unwrap();
    cq_relational_lowering::check(&vo, &ra).unwrap();
    let select = ra
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, ra::Operator::Select(_)))
        .unwrap();
    assert!(
        matches!(&ra.plan.definitions[select].operator, ra::Operator::Select(s) if s.condition.range_bounds().is_some())
    );
    let mut changed = ra.clone();
    let ra::Operator::Select(s) = &mut changed.plan.definitions[select].operator else {
        panic!()
    };
    s.condition.constraints[1] = syn::parse_quote!(x <= @(upper));
    assert!(cq_relational_lowering::check(&vo, &changed).is_err());
    let mut lost = ra.clone();
    let ra::Operator::Select(s) = &mut lost.plan.definitions[select].operator else {
        panic!()
    };
    s.condition.constraints.pop();
    assert!(cq_relational_lowering::check(&vo, &lost).is_err());
    let ix = cq_index_policy::compile(&ra).unwrap();
    cq_index_policy::check(&ra, &ix).unwrap();
    assert_eq!(ix.indexes.len(), 1);
    assert_eq!(
        ix.indexes[0].requirement.supply.kind(),
        ix::SupplyKind::Range
    );
    assert!(ix.to_token_stream().to_string().contains("range"));
    let mut missing = ix.clone();
    missing.indexes.clear();
    ix::contract::check(&missing).unwrap();
    assert!(cq_index_policy::check(&ra, &missing).is_err());
    let mut wrong = ix.clone();
    wrong.indexes[0].requirement.supply = syn::parse_quote!(const);
    assert!(cq_index_policy::check(&ra, &wrong).is_err());
    let mut wrong = ix.clone();
    wrong.indexes[0].requirement.columns = syn::parse_quote!(1);
    assert!(cq_index_policy::check(&ra, &wrong).is_err());
    let physical = cq_physical_lowering::compile(&ix).unwrap();
    cq_physical_lowering::check(&ix, &physical).unwrap();
    assert!(matches!(
        physical.program.definitions[select].operator,
        physical::Operator::Range(..)
    ));
    let mut changed = physical.clone();
    let physical::Operator::Range(_, s) = &changed.program.definitions[select].operator else {
        panic!()
    };
    changed.program.definitions[select].operator = physical::Operator::Select(s.clone());
    cq_physical_lowering::check(&ix, &changed).unwrap();
    assert!(cq_physical_lowering::check(&missing, &physical).is_err());
    for tokens in [
        normal.to_token_stream(),
        vo.to_token_stream(),
        ra.to_token_stream(),
        ix.to_token_stream(),
        physical.to_token_stream(),
    ] {
        assert!(tokens.to_string().contains("lower"));
    }
    assert_eq!(
        syn::parse2::<cq_lang_vo::Program>(vo.to_token_stream()).unwrap(),
        vo
    );
    assert_eq!(
        syn::parse2::<ra::Program>(ra.to_token_stream()).unwrap(),
        ra
    );
    assert_eq!(
        syn::parse2::<ix::Program>(ix.to_token_stream()).unwrap(),
        ix
    );
    assert_eq!(
        syn::parse2::<physical::Program>(physical.to_token_stream()).unwrap(),
        physical
    );
    {
        let structures = cq_structure_selection::compile(&physical).unwrap();
        cq_structure_selection::check(&physical, &structures).unwrap();
        assert!(
            structures
                .structures
                .entries
                .iter()
                .all(|e| !matches!(e.structure, cq_lang_structures::Structure::Built { .. }))
        );
        let mut changed = structures.clone();
        let input = &ra.plan.definitions[0].result;
        changed.structures = syn::parse2(
            quote::quote!(structures { providers { btree_map_grouped = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> { reads [exact, membership, ordered], supports [keys], build typed, shared }; } #input = @btree_map_grouped as ::std::collections::BTreeMap<_, Vec<_>> [x]; }),
        )
        .unwrap();
        assert!(cq_structure_selection::check(&physical, &changed).is_err());
    }
}

#[test]
fn where_comparison_is_a_relational_select_with_a_host_bound() {
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32); relation S(a:i32);
        answer(od) :- R(od), S(s), where od < cutoff;
    };
    assert!(matches!(source.query.body[2], cq::BodyItem::Constraint(_)));
    let normal = cq::normal::normalize(&source).unwrap();
    let vo = cq_order::compile(&normal, &Default::default()).unwrap();
    let ra = cq_relational_lowering::compile(&vo).unwrap();
    cq_relational_lowering::check(&vo, &ra).unwrap();
    assert!(
        ra.plan
            .definitions
            .iter()
            .all(|d| !matches!(d.operator, ra::Operator::Guard(_)))
    );
    let select = ra
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, ra::Operator::Select(_)))
        .unwrap();
    let later = ra
        .plan
        .definitions
        .iter()
        .position(|d| matches!(&d.operator, ra::Operator::Rename(r) if r.relation == "S"))
        .unwrap();
    assert!(select < later);
    let ix = cq_index_policy::compile(&ra).unwrap();
    assert!(
        ix.indexes
            .iter()
            .any(|i| i.requirement.supply.kind() == ix::SupplyKind::Range)
    );
}

#[test]
fn rust_conjunction_stays_whole_and_moves_when_its_variables_are_bound() {
    use quote::ToTokens;
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32,b:i32); relation S(a:i32);
        answer(od) :- R(od,x), S(s), if *od < cutoff && f(*x) && g(*x);
    };
    assert_eq!(source.query.body.len(), 3);
    let cq::BodyItem::Filter(residual) = &source.query.body[2] else {
        panic!()
    };
    assert_eq!(
        residual.condition.to_token_stream().to_string(),
        "* od < cutoff && f (* x) && g (* x)"
    );
    let normal = cq::normal::normalize(&source).unwrap();
    let options = cq_order::Options {
        heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        split: cq_order::Split::Coarse,
    };
    let vo = cq_order::compile(&normal, &options).unwrap();
    let rust_step = vo
        .valuation
        .steps
        .iter()
        .position(|s| matches!(s, cq_lang_vo::Step::Guard(..)))
        .unwrap();
    let mut moved = vo.clone();
    let guard = moved.valuation.steps.remove(rust_step);
    moved.valuation.steps.insert(1, guard);
    cq_lang_vo::contract::check(&moved).unwrap();
    let mut reversed = vo.clone();
    let cq_lang_vo::Step::Guard(g, _) = &mut reversed.valuation.steps[rust_step] else {
        panic!()
    };
    g.condition = syn::parse_quote!(g(*x) && f(*x));
    assert!(cq_order::check(&normal, &reversed, &options).is_err());
    let ra = cq_relational_lowering::compile(&vo).unwrap();
    cq_relational_lowering::check(&vo, &ra).unwrap();
    assert!(
        ra.plan
            .definitions
            .iter()
            .all(|definition| { !matches!(definition.operator, ra::Operator::Select(_)) })
    );
    let guard = ra
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, ra::Operator::Guard(_)))
        .unwrap();
    let mut changed = ra.clone();
    let ra::Operator::Guard(g) = &mut changed.plan.definitions[guard].operator else {
        panic!()
    };
    g.condition = syn::parse_quote!(g(*x) && f(*x));
    assert!(cq_relational_lowering::check(&vo, &changed).is_err());
}

#[test]
fn rust_block_depends_on_free_variables_not_the_written_prefix() {
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32); relation S(b:i32);
        answer(x,z) :- R(x), S(y),
            { let z:i32 = *x + 1 } | { let z:i32 = *x + 2 };
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let options = cq_order::Options {
        heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        split: cq_order::Split::Coarse,
    };
    let original = cq_order::compile(&normal, &options).unwrap();
    let mut moved = original.clone();
    let index = moved
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, cq_lang_vo::Step::Block(..)))
        .unwrap();
    assert_eq!(index, 1, "the connected block precedes the independent S scan");
    // Both legal orders retain the block's x-only free-variable scope.
    let independent = moved.valuation.steps.remove(2);
    moved.valuation.steps.insert(0, independent);
    cq_lang_vo::contract::check(&moved).unwrap();
    let with_prior_scan = cq_relational_lowering::compile(&moved).unwrap();
    cq_relational_lowering::check(&moved, &with_prior_scan).unwrap();
    let with_later_scan = cq_relational_lowering::compile(&original).unwrap();
    ra::check_rewrite(&with_later_scan, &with_prior_scan).unwrap();
    ra::check_rewrite(&with_prior_scan, &with_later_scan).unwrap();
}

#[test]
fn mixed_disjunction_and_negation_stay_wholly_opaque() {
    for expression in [
        quote::quote!(f(*od) || *od < cutoff),
        quote::quote!(!(*od < cutoff && f(*od))),
    ] {
        let source: cq::Program = syn::parse2(quote::quote! {
            struct Q; relation R(a:i32); relation S(a:i32);
            answer(od) :- R(od), S(s), if #expression;
        })
        .unwrap();
        assert_eq!(source.query.body.len(), 3);
        assert!(matches!(source.query.body[2], cq::BodyItem::Filter(_)));
        let vo = cq_order::compile(&source, &Default::default()).unwrap();
        let ra = cq_relational_lowering::compile(&vo).unwrap();
        assert!(
            ra.plan
                .definitions
                .iter()
                .any(|d| matches!(d.operator, ra::Operator::Guard(_)))
        );
        assert!(
            ra.plan
                .definitions
                .iter()
                .all(|d| !matches!(d.operator, ra::Operator::Select(_)))
        );
        let mut moved = vo.clone();
        moved.valuation.steps.swap(1, 2);
        assert!(cq_lang_vo::contract::check(&moved).unwrap_err().to_string()
            .contains("VO-Order I2"));
    }
}

#[test]
fn a_constraint_on_a_let_result_can_cross_total_rust() {
    for predicate in [quote::quote!(where m > cutoff), quote::quote!(where m == 2)] {
        let source: cq::Program = syn::parse2(quote::quote! {
            struct Bound; relation R(a:i32); relation S(a:i32,b:i32);
            answer(y) :- R(x), let m:i32 = *x, #predicate, if 10 / (*m).max(1) > 0, S(m,y);
        })
        .unwrap();
        let nf = cq::normal::normalize(&source).unwrap();
        let vo = cq_order::compile(&nf, &Default::default()).unwrap();
        let ra = cq_relational_lowering::compile(&vo).unwrap();
        cq_relational_lowering::check(&vo, &ra).unwrap();
        let select = ra
            .plan
            .definitions
            .iter()
            .position(|d| matches!(d.operator, ra::Operator::Select(_)))
            .unwrap();
        let guard = ra
            .plan
            .definitions
            .iter()
            .position(|d| matches!(d.operator, ra::Operator::Guard(_)))
            .unwrap();
        assert!(select < guard);
        let mut bad = ra.clone();
        let ra::Operator::Select(mut s) = bad.plan.definitions[select].operator.clone() else {
            panic!()
        };
        let ra::Operator::Guard(mut g) = bad.plan.definitions[guard].operator.clone() else {
            panic!()
        };
        g.input = s.input.clone();
        s.input = bad.plan.definitions[select].result.clone();
        bad.plan.definitions[select].operator = ra::Operator::Guard(g);
        bad.plan.definitions[guard].operator = ra::Operator::Select(s);
        ra::contract::check(&bad).unwrap();
        cq_relational_lowering::check(&vo, &bad).unwrap();
    }
}

#[test]
fn shared_names_survive_order_reduce_and_elaborate_as_natural_join_columns() {
    let source: cq::Program = syn::parse_quote! {
        struct Shared; relation Customer(id:i32,segment:i32);
        relation Orders(id:i32,customer:i32,date:i32);
        answer(o,c,d) :- Customer(c,s), if *s == 1, Orders(o,c,d);
    };
    let nf = cq::normal::normalize(&source).unwrap();
    let vo = cq_order::compile(&nf, &Default::default()).unwrap();
    let options = cq_reduce::Options {
        semijoins: cq_reduce::Reductions::Tree,
    };
    let reduced = cq_reduce::compile(&vo, &options).unwrap();
    cq_reduce::check(&vo, &reduced, &options).unwrap();
    let ra = cq_relational_lowering::compile(&reduced).unwrap();
    cq_relational_lowering::check(&reduced, &ra).unwrap();
    assert!(!ra.plan.definitions.iter().any(|d| {
        match &d.operator {
            ra::Operator::Select(s) => s
                .condition
                .equalities
                .iter()
                .any(|e| matches!(e.right, ra::EqualityValue::Variable(_))),
            _ => false,
        }
    }));
    let facts = ra::contract::check(&ra).unwrap();
    assert!(ra.plan.definitions.iter().any(|d| {
        match &d.operator {
            ra::Operator::NaturalJoin(j) => facts[&symbol_name(&j.left)]
                .heading
                .intersection(&facts[&symbol_name(&j.right)].heading)
                .any(|name| name == "c"),
            _ => false,
        }
    }));
    let mut changed = reduced.clone();
    changed.cq.query.body[2] = syn::parse_quote!(Orders(o, other, d));
    assert!(cq_reduce::check(&vo, &changed, &options).is_err());
}

#[test]
fn hypergraph_reduced_prefix_tuple_survives_elaboration_checks() {
    let source: cq::Program = syn::parse_quote! {
        struct PrefixTuple;
        relation R(x:i32); relation Other(y:i32); relation T(x:i32,y:i32,z:i32);
        answer(z) :- R(x), Other(y), where x > y, if 10 / (*x - *y).max(1) > 0, T(x,y,z),
            where z < 9;
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let order = cq_order::Options {
        heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        split: cq_order::Split::Coarse,
    };
    let written = cq_order::compile(&normal, &order).unwrap();
    cq_order::check(&normal, &written, &order).unwrap();
    let options = cq_reduce::Options {
        semijoins: cq_reduce::Reductions::Hypergraph,
    };
    let reduced = cq_reduce::compile(&written, &options).unwrap();
    cq_reduce::check(&written, &reduced, &options).unwrap();
    assert!(
        reduced
            .valuation
            .steps
            .iter()
            .any(|step| matches!(step, cq_lang_vo::Step::Reduce(..)))
    );
    let target = cq_relational_lowering::compile(&reduced).unwrap();
    cq_relational_lowering::check(&reduced, &target).unwrap();
    let indexed = cq_index_policy::compile(&target).unwrap();
    cq_index_policy::check(&target, &indexed).unwrap();
    let physical = cq_physical_lowering::compile(&indexed).unwrap();
    cq_physical_lowering::check(&indexed, &physical).unwrap();
}

#[test]
fn reduced_plans_compute_the_authored_answers() {
    for case in cases::CASES {
        let source = cq::normal::normalize(&syn::parse_str(case.cq).unwrap()).unwrap();
        let ordered = cq_order::compile(&source, &cq_order::Options::default()).unwrap();
        let options = cq_reduce::Options {
            semijoins: if cq::hypergraph::Analysis::of(&source).acyclic {
                cq_reduce::Reductions::Tree
            } else {
                cq_reduce::Reductions::None
            },
        };
        let reduced = cq_reduce::compile(&ordered, &options).unwrap();
        let target = cq_relational_lowering::compile(&reduced).unwrap();
        let database = case
            .inputs
            .iter()
            .map(|(name, rows)| {
                (
                    (*name).into(),
                    rows.iter().map(|row| row.to_vec()).collect(),
                )
            })
            .collect();
        let expected: values::Rows = case.expected.iter().map(|row| row.to_vec()).collect();
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            expected,
            "{}",
            case.name
        );
    }
}

#[test]
fn lowering_preserves_authored_demands_but_index_policy_requires_first_use_names() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        l = rename L@0 {a -> a}; r = rename R@1 {a -> a, b -> b}; s = rename S@2 {b -> b, c -> c};
        j0 = natural_join l with r; j1 = natural_join j0 with s;
        x = semijoin j1 with l; p = project x keep {a, c}; output p as answer(a, c);
    } indexes { membership = @l[0] stream; second = @s[0] stream; first = @r[0] stream; }};
    let target = cq_physical_lowering::compile(&source).unwrap();
    ix::contract::check(&source).unwrap();
    assert!(cq_index_policy::check(&source.ra, &source).is_err());
    cq_physical_lowering::check(&source, &target).unwrap();
    for missing in 0..3 {
        let mut changed = source.clone();
        changed.indexes = source
            .indexes
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != missing)
            .map(|(_, demand)| demand.clone())
            .collect();
        changed.indexes.push_punct(Default::default());
        ix::contract::check(&changed).unwrap();
        assert!(cq_index_policy::check(&source.ra, &changed).is_err());
        for result in [
            cq_physical_lowering::check(&changed, &target),
            cq_physical_lowering::compile(&changed).map(|_| ()),
        ] {
            assert!(result.is_err());
        }
    }
}

#[test]
fn complete_program_parsers_reject_wrong_stage_payloads() {
    let scan = cq_testkit::SCENARIOS
        .iter()
        .find(|scenario| scenario.name == "scan")
        .unwrap();
    let cq = scan.cq;
    let ra = scan.ra;
    let ix = format!("{ra} indexes {{}}");
    assert!(syn::parse_str::<ix::Program>(cq).is_err());
    assert!(syn::parse_str::<ix::Program>(ra).is_err());
    assert!(syn::parse_str::<cq::Program>(&ix).is_err());
    assert!(syn::parse_str::<ra::Program>(&ix).is_err());
}

#[test]
fn declaration_layout_and_leaf_expressions_are_preserved_at_every_edge() {
    use quote::ToTokens;
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32) = (host_expression(config));
        answer(x) :- R(x);
    };
    let nf = cq::normal::normalize(&source).unwrap();
    let order = cq_order::Options::default();
    let vo = cq_order::compile(&nf, &order).unwrap();
    let reduction = cq_reduce::Options::default();
    let reduced = cq_reduce::compile(&vo, &reduction).unwrap();
    let ra = cq_relational_lowering::compile(&reduced).unwrap();
    let ix = cq_index_policy::compile(&ra).unwrap();
    let physical = cq_physical_lowering::compile(&ix).unwrap();
    let structures = cq_structure_selection::compile(&physical).unwrap();
    assert_eq!(source.declarations, nf.declarations);
    assert_eq!(source.declarations, vo.cq.declarations);
    assert_eq!(source.declarations, reduced.cq.declarations);
    assert_eq!(source.declarations, ra.plan.declarations);
    assert_eq!(source.declarations, ix.ra.plan.declarations);
    assert_eq!(source.declarations, physical.program.declarations);
    assert_eq!(
        source.declarations,
        structures.physical.program.declarations
    );
    assert_eq!(
        syn::parse2::<cq_lang_structures::Program>(structures.to_token_stream()).unwrap(),
        structures
    );
    let replacements: [RelationDecl; 2] = [
        syn::parse_quote!(relation R(a:u64) = (host_expression(config));),
        syn::parse_quote!(relation R(a:i32) = (other_expression(config));),
    ];
    for replacement in replacements {
        let mut t = vo.clone();
        t.cq.declarations[0] = replacement.clone();
        assert!(cq_order::check(&nf, &t, &order).is_err());
        let mut t = reduced.clone();
        t.cq.declarations[0] = replacement.clone();
        assert!(cq_reduce::check(&vo, &t, &reduction).is_err());
        let mut t = ra.clone();
        t.plan.declarations[0] = replacement.clone();
        assert!(cq_relational_lowering::check(&reduced, &t).is_err());
        let mut t = ix.clone();
        t.ra.plan.declarations[0] = replacement.clone();
        assert!(cq_index_policy::check(&ra, &t).is_err());
        let mut t = physical.clone();
        t.program.declarations[0] = replacement.clone();
        assert!(cq_physical_lowering::check(&ix, &t).is_err());
        let mut t = structures.clone();
        t.physical.program.declarations[0] = replacement;
        assert!(cq_structure_selection::check(&physical, &t).is_err());
    }
}

#[test]
fn output_schemas_cannot_silently_discard_leaf_values() {
    for output in [syn::parse_quote!(relation answer(x:i32) = (side_effect());)] {
        let output: RelationDecl = output;
        let c: cq::Program =
            syn::parse_quote! { struct Q; relation R(a:i32); #output answer(x) :- R(x); };
        let r: ra::Program = syn::parse_quote! { relational { relation R(a:i32); #output
        r = rename R@0 {a -> x}; output r as answer(x); } };
        let p: physical::Program = syn::parse_quote! { physical { relation R(a:i32); #output
        r = let canon rename R@0 {a -> x}; return r as answer(x); } };
        for result in [
            cq::contract::check(&c),
            ra::contract::check(&r).map(|_| ()),
            physical::contract::check(&p).map(|_| ()),
        ] {
            assert!(result.is_err());
        }
    }
}

#[test]
fn fine_tree_reduced_filtered_split_read_passes_its_own_elaboration_check() {
    // Test-layer finding F1 (stage differential seeds 157/397/701/957).
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation r0(c0: i32, c1: i32); relation r2(c0: i32, c1: i32);
        relation answer(c0: i32);
        answer(v3) :- r2(v0, v1), r0(v3, v1), where v0 >= 3;
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let order = cq_order::Options {
        heuristic: cq_order::Heuristic::Elimination,
        split: cq_order::Split::Fine,
    };
    let ordered = cq_order::compile(&normal, &order).unwrap();
    cq_order::check(&normal, &ordered, &order).unwrap();
    let options = cq_reduce::Options {
        semijoins: cq_reduce::Reductions::Tree,
    };
    let reduced = cq_reduce::compile(&ordered, &options).unwrap();
    cq_reduce::check(&ordered, &reduced, &options).unwrap();
    let target = cq_relational_lowering::compile(&reduced).unwrap();
    cq_relational_lowering::check(&reduced, &target).unwrap();
}

#[test]
fn masks_reduces_only_by_supports_its_restrictions_filter() {
    // `where x < 5` restricts both covering atoms, B@1 written before it and
    // C@3 written after it, so every Masks support is actually filtered.
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation A(a: i32, b: i32); relation B(b: i32, x: i32);
        relation C(x: i32, d: i32); relation answer(a: i32, d: i32);
        answer(a, d) :- A(a, b), B(b, x), where x < 5, C(x, d);
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let ordered = cq_order::compile(&normal, &Default::default()).unwrap();
    let options = cq_reduce::Options {
        semijoins: cq_reduce::Reductions::Masks,
    };
    let reduced = cq_reduce::compile(&ordered, &options).unwrap();
    cq_reduce::check(&ordered, &reduced, &options).unwrap();
    let mut restricted = std::collections::BTreeSet::new();
    let mut supports = Vec::new();
    for step in &reduced.valuation.steps {
        match step {
            cq_lang_vo::Step::Restrict(restrict, _) => {
                restricted.insert(restrict.clause());
            }
            cq_lang_vo::Step::Reduce(reduce, _) => {
                let cq_lang_vo::Reduction::With(_, support) = &reduce.reduction;
                supports.push(support.clause());
            }
            _ => (),
        }
    }
    assert_eq!(restricted, [1, 3].into());
    assert!(!supports.is_empty());
    assert!(supports.iter().all(|support| restricted.contains(support)));
    let target = cq_relational_lowering::compile(&reduced).unwrap();
    cq_relational_lowering::check(&reduced, &target).unwrap();
    // Ranks read the recorded restrictions: the written reference keeps the
    // predicate as a prefix step, so no support ranks as filtered.
    let written = cq_order::compile_written(&normal).unwrap();
    let unrestricted = cq_reduce::compile(&written, &options).unwrap();
    cq_reduce::check(&written, &unrestricted, &options).unwrap();
    assert!(
        !unrestricted
            .valuation
            .steps
            .iter()
            .any(|step| matches!(step, cq_lang_vo::Step::Reduce(..)))
    );
}

#[test]
fn restrictions_do_not_depend_on_the_predicate_written_position() {
    // The same query with `where x < 5` written after or before `S(x, b)`
    // restricts the same occurrences.
    for body in [
        quote::quote!(R(a, x), S(x, b), where x < 5, T(b)),
        quote::quote!(R(a, x), where x < 5, S(x, b), T(b)),
    ] {
        let source: cq::Program = syn::parse_quote! {
            struct Q; relation R(a: i32, x: i32); relation S(x: i32, b: i32);
            relation T(b: i32); relation answer(a: i32, b: i32);
            answer(a, b) :- #body;
        };
        let normal = cq::normal::normalize(&source).unwrap();
        let ordered = cq_order::compile(&normal, &Default::default()).unwrap();
        let restricted: Vec<_> = ordered
            .valuation
            .steps
            .iter()
            .filter_map(|step| match step {
                cq_lang_vo::Step::Restrict(restrict, _) => Some(restrict.relation.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(restricted, ["R", "S"]);
        assert!(
            !ordered
                .valuation
                .steps
                .iter()
                .any(|step| matches!(step, cq_lang_vo::Step::Constraint(..)))
        );
        let target = cq_relational_lowering::compile(&ordered).unwrap();
        cq_relational_lowering::check(&ordered, &target).unwrap();
    }
}

#[test]
fn a_hoisted_block_domain_is_read_only_after_vo_binds_its_free_columns() {
    use cq_lang_vo::Step;
    // The arm `where x == 1` generates no `x`: the block takes its finite
    // domain from the prefix, so neither the block nor a reduction reading
    // it may precede the read binding `x`.
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(x: i32); relation S(x: i32); relation T(x: i32);
        answer(x) :- R(x), {where x == 1} | {S(x)}, T(x);
    };
    let normal = cq::normal::normalize(&source).unwrap();
    for semijoins in [
        cq_reduce::Reductions::Masks,
        cq_reduce::Reductions::Hypergraph,
    ] {
        let ordered = cq_order::compile(&normal, &Default::default()).unwrap();
        let options = cq_reduce::Options { semijoins };
        let reduced = cq_reduce::compile(&ordered, &options).unwrap();
        cq_reduce::check(&ordered, &reduced, &options).unwrap();
        let steps = &reduced.valuation.steps;
        let read = steps
            .iter()
            .position(|step| matches!(step, Step::Atom(atom, _) if atom.clause() == 0))
            .unwrap();
        let reads_block = |step: &Step| match step {
            Step::Block(..) => true,
            Step::Reduce(reduce, _) => {
                let cq_lang_vo::Reduction::With(_, support) = &reduce.reduction;
                reduce.clause() == 1 || support.clause() == 1
            }
            _ => false,
        };
        assert!(!steps[..read].iter().any(reads_block), "{semijoins:?}");
        let target = cq_relational_lowering::compile(&reduced).unwrap();
        cq_relational_lowering::check(&reduced, &target).unwrap();
        for seed in 0..16 {
            let database = cq_testkit::databases::binary_database(&source, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                cq_testkit::cq_semantics::evaluate_cq(&source, &database),
                "{semijoins:?}/{seed}"
            );
        }
        // WF-BlockPlacement and the reduction premises reject moving the
        // block, or a reduction reading it, before the read binding `x`.
        let block = steps
            .iter()
            .position(|step| matches!(step, Step::Block(..)))
            .unwrap();
        let reduction = steps
            .iter()
            .position(|step| matches!(step, Step::Reduce(..)) && reads_block(step))
            .unwrap();
        for moved in [block, reduction] {
            let mut early = reduced.clone();
            let step = early.valuation.steps.remove(moved);
            let head = early
                .valuation
                .steps
                .iter()
                .take_while(|step| matches!(step, Step::Restrict(..)))
                .count();
            early.valuation.steps.insert(head, step);
            assert!(cq_lang_vo::contract::check(&early).is_err(), "{semijoins:?}");
        }
    }
}

#[test]
fn a_nested_domain_block_in_an_unseeded_hoisted_arm_follows_its_generator() {
    // DriftWatch5 #1. The outer arm `{ S(x), ... }` is hoisted without a
    // finite domain, so `x` is visible in its scope but no seed provides it.
    // The nested block's arm `where x == 1` takes its domain from the arm's
    // own prefix, which must therefore read `S(x)` first, never Unit.
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(x: i32); relation S(x: i32); relation T(x: i32);
        relation U(x: i32); relation answer(x: i32);
        answer(x) :- R(x), { S(x), { where x == 1 } | { T(x) } } | { U(x) };
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let written = cq_order::compile_written(&normal).unwrap();
    let reference = cq_relational_lowering::compile(&written).unwrap();
    for heuristic in [
        cq_order::Heuristic::Filtered,
        cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        cq_order::Heuristic::Elimination,
        cq_order::Heuristic::Written,
    ] {
        for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
            let order = cq_order::Options { heuristic, split };
            let ordered = cq_order::compile(&normal, &order)
                .unwrap_or_else(|error| panic!("{heuristic:?}/{split:?}: {error}"));
            cq_order::check(&normal, &ordered, &order).unwrap();
            for semijoins in [cq_reduce::Reductions::None, cq_reduce::Reductions::Masks] {
                let options = cq_reduce::Options { semijoins };
                let reduced = cq_reduce::compile(&ordered, &options).unwrap();
                cq_reduce::check(&ordered, &reduced, &options).unwrap();
                let target = cq_relational_lowering::compile(&reduced).unwrap_or_else(|error| {
                    panic!("{heuristic:?}/{split:?}/{semijoins:?}: {error}")
                });
                cq_relational_lowering::check(&reduced, &target).unwrap();
                for seed in 0..16 {
                    let database = cq_testkit::databases::binary_database(&source, seed);
                    assert_eq!(
                        oracle::evaluate_ra(&target, &database).output,
                        oracle::evaluate_ra(&reference, &database).output,
                        "{heuristic:?}/{split:?}/{semijoins:?}/{seed}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_nested_domain_block_waits_for_both_unseeded_columns() {
    // Fine splitting must not emit the nested domain block after only one
    // of its two prefix suppliers has provided a column.
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(x: i32, y: i32); relation S(x: i32);
        relation V(y: i32); relation T(x: i32, y: i32);
        relation U(x: i32, y: i32); relation answer(x: i32, y: i32);
        answer(x,y) :- R(x,y), { S(x), V(y), { where x == 1, where y == 2 } | { T(x,y) } } | { U(x,y) };
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let written = cq_order::compile_written(&normal).unwrap();
    let reference = cq_relational_lowering::compile(&written).unwrap();
    for heuristic in [
        cq_order::Heuristic::Filtered,
        cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        cq_order::Heuristic::Elimination,
        cq_order::Heuristic::Written,
    ] {
        for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
            let order = cq_order::Options { heuristic, split };
            let ordered = cq_order::compile(&normal, &order)
                .unwrap_or_else(|error| panic!("{heuristic:?}/{split:?}: {error}"));
            cq_order::check(&normal, &ordered, &order).unwrap();
            for semijoins in [cq_reduce::Reductions::None, cq_reduce::Reductions::Masks] {
                let options = cq_reduce::Options { semijoins };
                let reduced = cq_reduce::compile(&ordered, &options).unwrap();
                cq_reduce::check(&ordered, &reduced, &options).unwrap();
                let target = cq_relational_lowering::compile(&reduced).unwrap_or_else(|error| {
                    panic!("{heuristic:?}/{split:?}/{semijoins:?}: {error}")
                });
                cq_relational_lowering::check(&reduced, &target).unwrap();
                for seed in 0..16 {
                    let database = cq_testkit::databases::binary_database(&source, seed);
                    assert_eq!(
                        oracle::evaluate_ra(&target, &database).output,
                        oracle::evaluate_ra(&reference, &database).output,
                        "{heuristic:?}/{split:?}/{semijoins:?}/{seed}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_predicate_after_a_domain_needing_block_restricts_earlier_atoms() {
    use cq_lang_vo::Step;
    // `where x < 5` is written after a hoisted block whose arm `where x == 1`
    // generates no `x`; it still restricts `R@0`, written before the block,
    // and the block's domain is then the restricted prefix.
    for choice in ["|", "else"] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(x: i32); relation S(x: i32); relation T(x: i32);
             answer(x) :- R(x), {{ where x == 1 }} {choice} {{ S(x) }}, where x < 5, T(x);"
        ))
        .unwrap();
        let planned = cq_planning::prefix_valuation(&source, &Default::default()).unwrap();
        let restricted: Vec<_> = planned
            .valuation
            .steps
            .iter()
            .filter_map(|step| match step {
                Step::Restrict(restrict, _) => Some(restrict.clause()),
                _ => None,
            })
            .collect();
        assert_eq!(restricted, [0, 3], "{choice}");
        let target = cq_relational_lowering::compile(&planned).unwrap();
        cq_relational_lowering::check(&planned, &target).unwrap();
        let normal = cq::normal::normalize(&source).unwrap();
        let written = cq_order::compile_written(&normal).unwrap();
        let reference = cq_relational_lowering::compile(&written).unwrap();
        for seed in 0..32 {
            let database = cq_testkit::databases::binary_database(&source, seed);
            assert_eq!(
                oracle::evaluate_ra(&target, &database).output,
                oracle::evaluate_ra(&reference, &database).output,
                "{choice}/{seed}"
            );
        }
    }
}

#[test]
fn fine_tree_repeated_occurrences_pass_their_own_elaboration_check() {
    // Test-layer finding F6 (stage differential seeds 557/877/941, minimized).
    let elimination = cq_order::Options {
        heuristic: cq_order::Heuristic::Elimination,
        split: cq_order::Split::Fine,
    };
    let connected = cq_order::Options {
        heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        split: cq_order::Split::Fine,
    };
    let cases: [(cq::Program, cq_order::Options); 3] = [
        (
            syn::parse_quote! {
                struct Q; relation R(a:i32,b:i32); relation answer(a:i32);
                answer(a) :- R(a,b), R(b,b), where b <= b;
            },
            elimination.clone(),
        ),
        (
            syn::parse_quote! {
                struct Q; relation R(a:i32,b:i32,c:i32); relation S(a:i32,b:i32);
                relation answer(a:i32);
                answer(a) :- R(a,b,a), R(a,d,_), S(c,d);
            },
            elimination,
        ),
        (
            syn::parse_quote! {
                struct Q; relation R(a:i32,b:i32,c:i32); relation S(a:i32);
                relation answer(a:i32);
                answer(z) :- R(a,b,a), S(a), R(c,b,d), R(c,b,z);
            },
            connected,
        ),
    ];
    let options = cq_reduce::Options {
        semijoins: cq_reduce::Reductions::Tree,
    };
    for (source, order) in cases {
        let normal = cq::normal::normalize(&source).unwrap();
        let ordered = cq_order::compile(&normal, &order).unwrap();
        cq_order::check(&normal, &ordered, &order).unwrap();
        let reduced = cq_reduce::compile(&ordered, &options).unwrap();
        cq_reduce::check(&ordered, &reduced, &options).unwrap();
        let target = cq_relational_lowering::compile(&reduced).unwrap();
        cq_relational_lowering::check(&reduced, &target).unwrap_or_else(|error| {
            panic!(
                "{error}\n{}\n{}",
                quote::ToTokens::to_token_stream(&reduced),
                quote::ToTokens::to_token_stream(&target)
            )
        });
    }
}
