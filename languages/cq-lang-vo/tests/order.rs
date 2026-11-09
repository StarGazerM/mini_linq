use cq_lang_vo::{Program, Step, contract, order};
use quote::quote;

fn rejection(cq: &cq_lang_vo::cq::Program, program: &Program, invariant: &str) {
    let error = contract::check(cq, program).unwrap_err().to_string();
    assert!(error.contains(invariant), "{error}");
}

#[test]
fn filters_cannot_cross_atoms_lets_or_generators() {
    for (body, introduction) in [
        (quote!(S(y)), quote!(S@1(y);)),
        (quote!(let z = x), quote!(let z = x;)),
        (quote!(for z in 0..x), quote!(for z in 0..x;)),
    ] {
        let (cq, program) = parse(quote! {
            struct Q; relation R(x:i32); relation S(y:i32);
            answer(x) :- R(x), #body, if opaque(x);
            valuation { R@0(x); #introduction if opaque(x); }
        }).unwrap();
        rejection(&cq, &program, "VO-Order I2");
        let mut legal = program;
        legal.valuation.steps.swap(1, 2);
        contract::check(&cq, &legal).unwrap();
    }
}

#[test]
fn simultaneously_ready_filters_are_unordered_and_fully_bound_atoms_are_filters() {
    let (cq, mut program) = parse(quote! {
        struct Q; relation R(x:i32); relation S(y:i32); relation K(x:i32);
        answer(x,y) :- R(x), S(y), K(x), if opaque(x), let z = x;
        valuation { R@0(x); K@2(x); if opaque(x); S@1(y); let z = x; }
    }).unwrap();
    contract::check(&cq, &program).unwrap();
    program.valuation.steps.swap(1, 2);
    contract::check(&cq, &program).unwrap();
    program.valuation.steps.swap(2, 3);
    rejection(&cq, &program, "VO-Order I2");
}

#[test]
fn introducing_lets_are_not_filters_and_readiness_limits_connectivity() {
    let (cq, program) = parse(quote! {
        struct Q; relation R(x:i32); relation S(y:i32); relation T(x:i32,z:i32);
        answer(x,y,z) :- R(x), S(y), let z:i32 = x, T(x,z);
        valuation { R@0(x); S@1(y); let z:i32 = x; T@3(x,z); }
    }).unwrap();
    contract::check(&cq, &program).unwrap();
}

#[test]
fn disconnected_atom_cannot_cross_a_ready_connected_atom() {
    let (cq, mut program) = parse(quote! {
        struct Q; relation R(x:i32); relation S(y:i32); relation T(x:i32,z:i32);
        answer(x,y,z) :- R(x), S(y), T(x,z);
        valuation { R@0(x); S@1(y); T@2(x,z); }
    }).unwrap();
    rejection(&cq, &program, "VO-Order I3");
    program.valuation.steps.swap(1, 2);
    contract::check(&cq, &program).unwrap();
}

#[test]
fn redundant_empty_checks_are_rejected_even_at_the_root() {
    let (cq, mut program) = parse(quote! {
        struct Q; relation R(x:i32); relation S(y:i32);
        answer(x,y) :- R(x), S(y);
        valuation { R@0(x); S@1(_); S@1(y); }
    }).unwrap();
    rejection(&cq, &program, "VO-Order I2");
    program.valuation.steps.swap(0, 1);
    rejection(&cq, &program, "redundant zero-column check");
    program.valuation.steps.remove(0);
    contract::check(&cq, &program).unwrap();
    assert_eq!(round_trip(&cq, &program), program);
}

#[test]
fn recursive_scopes_check_i2_and_i3() {
    let (cq, mut program) = parse(quote! {
        struct Q; relation R(a:i32); relation S(a:i32,b:i32); relation T(c:i32);
        relation K(b:i32,d:i32);
        answer(a,b,c,d) :- R(a),
            { S(a,b), if opaque(b), T(c), K(b,d) }
            | { S(a,b), if opaque(b), T(c), K(b,d) };
        valuation {
            R@0(a);
            block@1(a,b,c,d) correlated {
                S@1.0.0(a,b); if opaque(b); K@1.0.3(b,d); T@1.0.2(c);
            } | {
                S@1.1.0(a,b); if opaque(b); K@1.1.3(b,d); T@1.1.2(c);
            };
        }
    }).unwrap();
    contract::check(&cq, &program).unwrap();
    let Step::Block(block, _) = &mut program.valuation.steps[1] else { unreachable!() };
    block.branches[1].steps.swap(1, 2);
    rejection(&cq, &program, "VO-Order I2");
    let Step::Block(block, _) = &mut program.valuation.steps[1] else { unreachable!() };
    block.branches[1].steps.swap(1, 2);
    block.branches[1].steps.swap(2, 3);
    rejection(&cq, &program, "VO-Order I3");
}

#[test]
fn hoisted_visible_names_are_not_supplied_bindings() {
    let (cq, mut program) = parse(quote! {
        struct Q; relation R(a:i32); relation S(a:i32,b:i32);
        answer(a,b) :- R(a), { where a > 0, S(a,b) } | { where a > 0, S(a,b) };
        valuation {
            R@0(a);
            block@1(a,b) hoisted {
                where a > 0; S@1.0.1(a,b);
            } | {
                S@1.1.1(a,b); where a > 0;
            };
        }
    }).unwrap();
    let analysis = order::analyze(&cq, &program).unwrap();
    assert!(analysis.branches[&1][0].provided_before[0].is_empty());
    rejection(&cq, &program, "VO-Order I1");
    let Step::Block(block, _) = &mut program.valuation.steps[1] else { unreachable!() };
    block.branches[0].steps.swap(0, 1);
    contract::check(&cq, &program).unwrap();
}

#[test]
fn recorded_ordering_inputs_round_trip_and_are_preserved() {
    for (driver, delta, seed) in [(quote!(WrittenOrder), quote!(none), true),
                                (quote!(DeltaFirst), quote!(1), false)] {
        for demand in [quote!(none), quote!([R(0)])] {
            let facts = quote!(ordering(seed = #seed, delta = #delta, driver = #driver,
                permutation = [1, 0], shared_demand = #demand));
            let (cq, mut program) = parse(quote! {
                struct Q;
                relation R(x:i32); relation S(x:i32);
                answer(x) :- R(x), S(x);
                valuation #facts { R@0(x); S@1(x); }
            }).unwrap();
            contract::check(&cq, &program).unwrap();
            assert_eq!(round_trip(&cq, &program), program);
            let source = program.clone();
            program.valuation.ordering = None;
            contract::check(&cq, &program).unwrap();
            assert!(cq_lang_vo::check_rewrite(&cq, &source, &cq, &program).is_err());
        }
    }
}

#[test]
fn ordering_inputs_reject_non_bijections_and_seed_delta_disagreement() {
    let (cq, program) = parse(quote! {
        struct Q;
        relation R(x:i32); relation S(x:i32);
        answer(x) :- R(x), S(x);
        valuation ordering(seed = false, delta = 1, driver = DeltaFirst,
            permutation = [1, 0], shared_demand = none) { R@0(x); S@1(x); }
    }).unwrap();
    contract::check(&cq, &program).unwrap();
    let changes: [fn(&mut cq_lang_vo::ordering::OrderingFacts); 4] = [
        |facts| facts.permutation = vec![0, 0],
        |facts| facts.permutation = vec![0, 2],
        |facts| facts.seed = true,
        |facts| facts.delta = Some(2),
    ];
    for change in changes {
        let mut invalid = program.clone();
        change(invalid.valuation.ordering.as_mut().unwrap());
        rejection(&cq, &invalid, "VO-OrderingInputs");
    }
}

#[test]
fn zero_binding_lets_and_constraints_are_earliest_filters() {
    let (cq, mut program) = parse(quote! {
        struct Q; relation R(x:i32); relation S(y:i32);
        answer(x,y) :- R(x), S(y), let _ = opaque(x), where x > 0;
        valuation { R@0(x); S@1(y); let _ = opaque(x); where x > 0; }
    }).unwrap();
    rejection(&cq, &program, "VO-Order I2");
    let expansion = program.valuation.steps.remove(1);
    program.valuation.steps.push(expansion);
    contract::check(&cq, &program).unwrap();
    program.valuation.steps.swap(1, 2);
    contract::check(&cq, &program).unwrap();
}

#[test]
fn occurrence_reference_is_not_a_recorded_order_exemption() {
    let (cq, program) = parse(quote! {
        struct Q; relation R(x:i32); relation S(y:i32);
        answer(x,y) :- R(x), S(y), if opaque(x);
        valuation { R@0(x); S@1(y); if opaque(x); }
    }).unwrap();
    order::analyze_reference(&cq, &program.valuation.steps).unwrap();
    rejection(&cq, &program, "VO-Order I2");
}

#[test]
fn membership_is_derived_from_step_introductions() {
    let (cq, mut program) = parse(quote! {
        struct Q; relation R(x:i32); relation S(x:i32);
        answer(x) :- R(x), S(x);
        valuation { R@0(x); S@1(x); }
    }).unwrap();
    assert!(contract::check(&cq, &program).unwrap().introduces[1].is_empty());
    assert_eq!(round_trip(&cq, &program), program);
    program.valuation.steps.swap(0, 1);
    let facts = contract::check(&cq, &program).unwrap();
    assert_eq!(facts.introduces[0], ["x"]);
    assert!(facts.introduces[1].is_empty());
}

fn parse(tokens: proc_macro2::TokenStream) -> syn::Result<(cq_lang_vo::cq::Program, cq_lang_vo::Program)> {
    syn::parse::Parser::parse2(|input: syn::parse::ParseStream<'_>| {
        let cq = input.parse()?;
        let vo = cq_lang_vo::Program::parse(input, &cq)?;
        Ok((cq, vo))
    }, tokens)
}

fn round_trip(cq: &cq_lang_vo::cq::Program, vo: &cq_lang_vo::Program) -> cq_lang_vo::Program {
    syn::parse::Parser::parse2(|input: syn::parse::ParseStream<'_>| {
        cq_lang_vo::Program::parse(input, cq)
    }, quote::ToTokens::to_token_stream(vo)).unwrap()
}
