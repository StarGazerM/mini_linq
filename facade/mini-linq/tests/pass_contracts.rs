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
        answer(x,y) :- R(x,y), if * x >= lower, if * x < upper;
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
    let mut unbound = vo.clone();
    unbound.valuation.steps.swap(0, 1);
    assert!(cq_lang_vo::contract::check(&unbound).is_err());
    let ra = cq_elaborate::compile(&vo).unwrap();
    cq_elaborate::check(&vo, &ra).unwrap();
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
    s.condition.constraints[1] = syn::parse_quote!(x <= @upper);
    assert!(cq_elaborate::check(&vo, &changed).is_err());
    let mut lost = ra.clone();
    let ra::Operator::Select(s) = &mut lost.plan.definitions[select].operator else {
        panic!()
    };
    s.condition.constraints.pop();
    assert!(cq_elaborate::check(&vo, &lost).is_err());
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
            quote::quote!(structures { #input = ::std::collections::BTreeMap<_, Vec<_>> [x]; }),
        )
        .unwrap();
        assert!(cq_structure_selection::check(&physical, &changed).is_err());
    }
}

#[test]
fn rust_comparison_is_a_relational_select_with_a_host_bound() {
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32); relation S(a:i32);
        answer(od) :- R(od), S(s), if *od < cutoff;
    };
    assert!(matches!(source.query.body[2], cq::BodyItem::Constraint(_)));
    let normal = cq::normal::normalize(&source).unwrap();
    let vo = cq_order::compile(&normal, &Default::default()).unwrap();
    let ra = cq_elaborate::compile(&vo).unwrap();
    cq_elaborate::check(&vo, &ra).unwrap();
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
fn mixed_conjunction_pushes_the_comparison_and_pins_the_residual_in_written_order() {
    use quote::ToTokens;
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32,b:i32); relation S(a:i32);
        answer(od) :- R(od,x), S(s), if *od < cutoff && f(*x) && g(*x);
    };
    assert_eq!(source.query.body.len(), 4);
    assert!(matches!(source.query.body[2], cq::BodyItem::Constraint(_)));
    let cq::BodyItem::Filter(residual) = &source.query.body[3] else {
        panic!()
    };
    assert_eq!(
        residual.condition.to_token_stream().to_string(),
        "f (* x) && g (* x)"
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
    assert!(
        vo.valuation.steps[..rust_step]
            .iter()
            .any(|s| matches!(s,cq_lang_vo::Step::Atom(a,_) if a.clause()==1))
    );
    let mut moved = vo.clone();
    let guard = moved.valuation.steps.remove(rust_step);
    moved.valuation.steps.insert(1, guard);
    assert!(
        cq_lang_vo::contract::check(&moved)
            .unwrap_err()
            .to_string()
            .contains("Before")
    );
    let mut reversed = vo.clone();
    let cq_lang_vo::Step::Guard(g, _) = &mut reversed.valuation.steps[rust_step] else {
        panic!()
    };
    g.condition = syn::parse_quote!(g(*x) && f(*x));
    assert!(cq_order::check(&normal, &reversed, &options).is_err());
    let ra = cq_elaborate::compile(&vo).unwrap();
    cq_elaborate::check(&vo, &ra).unwrap();
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
        .position(|d| matches!(&d.operator,ra::Operator::Rename(r) if r.relation=="S"))
        .unwrap();
    let guard = ra
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, ra::Operator::Guard(_)))
        .unwrap();
    assert!(select < later && later < guard);
    let mut changed = ra.clone();
    let ra::Operator::Guard(g) = &mut changed.plan.definitions[guard].operator else {
        panic!()
    };
    g.condition = syn::parse_quote!(g(*x) && f(*x));
    assert!(cq_elaborate::check(&vo, &changed).is_err());
}

#[test]
fn mixed_disjunction_negation_and_arithmetic_stay_wholly_opaque() {
    for expression in [
        quote::quote!(f(*od) || *od < cutoff),
        quote::quote!(!(*od < cutoff && f(*od))),
        quote::quote!(*od + 1 < 5),
    ] {
        let source: cq::Program = syn::parse2(quote::quote! {
            struct Q; relation R(a:i32); relation S(a:i32);
            answer(od) :- R(od), S(s), if #expression;
        })
        .unwrap();
        assert_eq!(source.query.body.len(), 3);
        assert!(matches!(source.query.body[2], cq::BodyItem::Filter(_)));
        let vo = cq_order::compile(&source, &Default::default()).unwrap();
        let ra = cq_elaborate::compile(&vo).unwrap();
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
        assert!(cq_lang_vo::contract::check(&moved).is_err());
    }
}

#[test]
fn a_constraint_on_a_let_result_stays_before_rust_even_if_a_later_atom_uses_it() {
    for predicate in [quote::quote!(if m > cutoff), quote::quote!(if m == 2)] {
        let source: cq::Program = syn::parse2(quote::quote! {
            struct Bound; relation R(a:i32); relation S(a:i32,b:i32);
            answer(y) :- R(x), let m:i32 = *x, #predicate, if 10 / *m > 0, S(m,y);
        })
        .unwrap();
        let nf = cq::normal::normalize(&source).unwrap();
        let vo = cq_order::compile(&nf, &Default::default()).unwrap();
        let ra = cq_elaborate::compile(&vo).unwrap();
        cq_elaborate::check(&vo, &ra).unwrap();
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
        assert!(
            cq_elaborate::check(&vo, &bad)
                .unwrap_err()
                .to_string()
                .contains("source-before")
        );
    }
}

#[test]
fn shared_names_survive_order_reduce_and_elaborate_as_natural_join_columns() {
    let source: cq::Program = syn::parse_quote! {
        struct Shared; relation Customer(id:i32,segment:&str);
        relation Orders(id:i32,customer:i32,date:i32);
        answer(o,c,d) :- Customer(c,s), if *s == "BUILDING", Orders(o,c,d);
    };
    let nf = cq::normal::normalize(&source).unwrap();
    let vo = cq_order::compile(&nf, &Default::default()).unwrap();
    let options = cq_reduce::Options {
        guards: cq_reduce::Pushdown::Atoms,
        semijoins: cq_reduce::Reductions::Tree,
    };
    let reduced = cq_reduce::compile(&vo, &options).unwrap();
    cq_reduce::check(&vo, &reduced, &options).unwrap();
    let ra = cq_elaborate::compile(&reduced).unwrap();
    cq_elaborate::check(&reduced, &ra).unwrap();
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
fn reduced_plans_compute_the_authored_answers() {
    for case in cases::CASES {
        let source = cq::normal::normalize(&syn::parse_str(case.cq).unwrap()).unwrap();
        let ordered = cq_order::compile(&source, &cq_order::Options::default()).unwrap();
        let options = cq_reduce::Options {
            guards: cq_reduce::Pushdown::Atoms,
            semijoins: if cq::hypergraph::Analysis::of(&source).acyclic {
                cq_reduce::Reductions::Tree
            } else {
                cq_reduce::Reductions::None
            },
        };
        let reduced = cq_reduce::compile(&ordered, &options).unwrap();
        let target = cq_elaborate::compile(&reduced).unwrap();
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
        l = rename L {a -> a}; r = rename R {a -> a, b -> b}; s = rename S {b -> b, c -> c};
        j0 = natural_join l with r; j1 = natural_join j0 with s;
        x = semijoin j1 with l; p = project x keep {a, c}; output p as answer(a, c);
    } indexes { membership = @l[0] stream; second = @s[0] stream; first = @r[0] stream; }};
    let target = cq_physical_lowering::compile(&source).unwrap();
    ix::contract::check(&source).unwrap();
    let error = cq_index_policy::check(&source.ra, &source)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("j0") && error.contains("first-use"),
        "{error}"
    );
    cq_physical_lowering::check(&source, &target).unwrap();
    for (missing, reader) in [(0, "x"), (1, "j1"), (2, "j0")] {
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
            let error = result.unwrap_err().to_string();
            assert!(error.contains(&format!("`{reader}`")), "{error}");
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
    let ra = cq_elaborate::compile(&reduced).unwrap();
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
        assert!(cq_elaborate::check(&reduced, &t).is_err());
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
        r = rename R {a -> x}; output r as answer(x); } };
        let p: physical::Program = syn::parse_quote! { physical { relation R(a:i32); #output
        r = pipe canon rename R {a -> x}; return r as answer(x); } };
        for result in [
            cq::contract::check(&c),
            ra::contract::check(&r).map(|_| ()),
            physical::contract::check(&p).map(|_| ()),
        ] {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("result declarations")
            );
        }
    }
}
