//! Source-aware acceptance checks, separate from data-dependent answer checks.
use super::planned_ra::{self, Policy, ProjectionPolicy, Witness};
use super::{case, database, evaluate_cq, evaluate_ra, parse};
use cq_ir::{cq, relational_plan};

// Authored occurrence identities, tree witnesses, and peak logical arities.
// These expectations are not inferred from a plan or a compiler result.
fn reference(name: &str) -> (Witness, usize) {
    let (occurrences, tree, peak): (&[&str], Option<&[Option<usize>]>, usize) = match name {
        "scan" => (&["r"], None, 1),
        "projection" => (&["r"], None, 3),
        "self_chain" => (&["first", "second"], None, 3),
        "return_trip" => (&["outward", "inward"], None, 2),
        "intersection" => (&["r", "s"], None, 2),
        "unary_guard" => (&["r", "approved"], None, 2),
        "chain_projection" => (&["r", "s", "t"], None, 3),
        "claw" => (&["a0", "b0", "c0"], None, 4),
        "fact_star" => (&["f", "a0", "b0", "c0"], None, 4),
        "triangle" => (&["r", "s", "t"], None, 3),
        "cycle4" => (&["r", "s", "t", "u"], None, 3),
        "cartesian" => (&["r", "s"], None, 2),
        "existential" => (&["r", "s"], None, 2),
        "ternary_guard" => (&["a0", "b0", "c0", "stock"], None, 3),
        "composite" => (&["r", "s"], None, 4),
        "bushy" => (&["r", "s", "t", "u"], None, 3),
        "ya_delivery" => (&["d0", "s0", "n0"], Some(&[Some(1), None, Some(1)]), 3),
        "oriented_chain" => (&["first", "second", "third"], None, 3),
        "spade" => (&["r", "s", "t", "h"], None, 4),
        "barbell" => (&["r", "s", "t", "bridge", "u0", "v0", "w0"], None, 6),
        "loomis_whitney_4" => (&["abc", "bcd", "acd", "abd"], None, 4),
        "ya_deep" => (
            &["r0", "s0", "t0", "u0"],
            Some(&[None, Some(0), Some(1), Some(2)]),
            3,
        ),
        "ya_composite" => (&["r0", "s0", "t0"], Some(&[Some(1), None, Some(1)]), 4),
        "ya_disconnected" => (&["r0", "s0"], Some(&[None, Some(0)]), 3),
        "cartesian_first" => (&["r", "s", "t"], None, 4),
        _ => panic!("missing authored contract for {name}"),
    };
    (
        Witness {
            occurrences: occurrences.iter().map(|s| (*s).to_owned()).collect(),
            reduction_tree: tree.map(<[_]>::to_vec),
        },
        peak,
    )
}

fn source(plan: &relational_plan::Module) -> cq::Module {
    cq::Module {
        program: plan.program.clone(),
    }
}

fn policy(projection: ProjectionPolicy) -> Policy {
    Policy {
        projection,
        max_arity: None,
    }
}

pub(super) fn assert_verified_reference(name: &str, plan: &relational_plan::Module) {
    let (witness, peak) = reference(name);
    let verified = planned_ra::verify(
        &source(plan),
        plan.clone(),
        &witness,
        Policy {
            projection: ProjectionPolicy::Eager,
            max_arity: Some(peak),
        },
    )
    .unwrap_or_else(|e| panic!("{name}: administrative contract: {e}"));
    assert_eq!(
        verified.report().peak_arity,
        peak,
        "{name}: authored peak arity"
    );
    // Include raw joins before projection, not just normalized boundaries.
    let interpreted = evaluate_ra(plan, &database(case(name)));
    for (result, attributes) in &verified.report().headings {
        assert_eq!(
            *attributes, interpreted.definitions[result].heading,
            "{name}: {result}"
        );
    }
    assert_eq!(verified.plan(), plan);
}

fn reject(name: &str, text: String, message: &str) {
    let original = parse(case(name).plan);
    let wrong = parse(&text); // Local RA validity is deliberately insufficient.
    let (witness, _) = reference(name);
    let error = planned_ra::verify(
        &source(&original),
        wrong,
        &witness,
        policy(ProjectionPolicy::Eager),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains(message),
        "expected {message:?}, got {error}"
    );
}

pub(super) fn assert_rejected_reference(name: &str, plan: &relational_plan::Module) {
    let original = parse(case(name).plan);
    let (witness, _) = reference(name);
    assert!(
        planned_ra::verify(
            &source(&original),
            plan.clone(),
            &witness,
            policy(ProjectionPolicy::Eager)
        )
        .is_err(),
        "{name}: incorrect RA passed the administrative contract"
    );
}

#[test]
fn equal_answers_do_not_satisfy_eager_pruning() {
    let fixture = case("chain_projection");
    let known = parse(fixture.plan);
    let source = source(&known);
    let r1 = cq_query_lowering::composition::compile_cq(&source).unwrap();
    let db = database(fixture);
    assert_eq!(
        evaluate_ra(&r1, &db).output,
        evaluate_cq(&source.program, &db)
    );
    let witness = Witness {
        occurrences: vec!["r0".into(), "r1".into(), "r3".into()],
        reduction_tree: None,
    };
    let safe = planned_ra::verify(
        &source,
        r1.clone(),
        &witness,
        policy(ProjectionPolicy::Safe),
    )
    .unwrap();
    assert_eq!(safe.report().peak_arity, 4);
    let error = planned_ra::verify(
        &source,
        r1.clone(),
        &witness,
        policy(ProjectionPolicy::Eager),
    )
    .unwrap_err();
    assert!(error.to_string().contains("eager pruning"), "{error}");
    // A width bound is independently enforced even under the safe-only policy.
    let error = planned_ra::verify(
        &source,
        r1,
        &witness,
        Policy {
            projection: ProjectionPolicy::Safe,
            max_arity: Some(3),
        },
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("arity 4 exceeds limit 3"),
        "{error}"
    );
}

#[test]
fn missing_early_projection_is_rejected_even_with_identical_answers() {
    reject(
        "chain_projection",
        case("chain_projection").plan.replace(
            "live = project rs keep {a, c};",
            "live = project rs keep {a, b, c};",
        ),
        "eager pruning",
    );
}

#[test]
fn pruning_a_future_join_variable_is_rejected_without_data() {
    reject(
        "chain_projection",
        case("chain_projection").plan.replace(
            "live = project rs keep {a, c};",
            "live = project rs keep {a};",
        ),
        "unsafe projection",
    );
}

#[test]
fn safety_is_enforced_even_when_eager_pruning_is_disabled() {
    let original = parse(case("chain_projection").plan);
    let wrong = parse(&case("chain_projection").plan.replace(
        "live = project rs keep {a, c};",
        "live = project rs keep {a};",
    ));
    let error = planned_ra::verify(
        &source(&original),
        wrong,
        &reference("chain_projection").0,
        policy(ProjectionPolicy::Safe),
    )
    .unwrap_err();
    assert!(error.to_string().contains("unsafe projection"), "{error}");
}

#[test]
fn pruning_an_output_variable_is_rejected_before_it_is_reintroduced() {
    // Reintroducing a head variable from another occurrence cannot repair a
    // missed agreement. The locally valid plan still has the correct output.
    reject(
        "cycle4",
        case("cycle4").plan.replace(
            "left_side = project rs keep {a, c};",
            "left_side = project rs keep {c};",
        ),
        "unsafe projection",
    );
}

#[test]
fn occurrence_mapping_cannot_be_forged_by_equal_hyperedges() {
    reject(
        "return_trip",
        case("return_trip").plan.replace(
            "inward = rename Road {src -> b, dst -> a};",
            "inward = rename Road {src -> a, dst -> b};",
        ),
        "occurrence mapping",
    );
}

#[test]
fn source_program_cannot_be_changed_to_justify_the_plan() {
    reject(
        "self_chain",
        case("self_chain")
            .plan
            .replace("Road(a, b), Road(b, c)", "Road(a, b), Road(c, b)"),
        "source program",
    );
}

#[test]
fn disconnected_existence_cannot_be_erased() {
    let text = case("existential")
        .plan
        .replace("    s = rename S {first -> y, second -> z};\n", "")
        .replace("    exists = project s keep {};\n", "")
        .replace("    result = natural_join r with exists;\n", "")
        .replace("output result", "output r");
    reject("existential", text, "occurrence coverage");
}

#[test]
fn an_assembly_operand_cannot_be_consumed_twice() {
    // Set idempotence preserves answers, but this violates occurrence ownership.
    reject(
        "scan",
        case("scan").plan.replace(
            "output r",
            "again = natural_join r with r;\n    output again",
        ),
        "assembly frontier",
    );
}

#[test]
fn duplicate_written_occurrences_have_distinct_identities() {
    let plan = parse(
        "struct P; relation R(x: i32); answer(x) :- R(x), R(x);
        relational { a = rename R {x -> x}; b = rename R {x -> x};
        both = natural_join a with b; output both as answer(x); }",
    );
    let source = source(&plan);
    let mut witness = Witness {
        occurrences: vec!["a".into(), "b".into()],
        reduction_tree: None,
    };
    planned_ra::verify(
        &source,
        plan.clone(),
        &witness,
        policy(ProjectionPolicy::Eager),
    )
    .unwrap();
    witness.occurrences[1] = "a".into();
    let error =
        planned_ra::verify(&source, plan, &witness, policy(ProjectionPolicy::Eager)).unwrap_err();
    assert!(error.to_string().contains("occurrence coverage"), "{error}");
}

#[test]
fn composite_join_keys_and_empty_keys_are_derived_completely() {
    for (name, result, expected) in [
        ("composite", "joined", vec!["a", "b"]),
        ("loomis_whitney_4", "checked_acd", vec!["a", "c", "d"]),
        ("cartesian", "result", vec![]),
        ("ya_composite", "up_r", vec!["a", "b"]),
        ("ya_composite", "up_t", vec!["b", "c"]),
    ] {
        let plan = parse(case(name).plan);
        let (witness, _) = reference(name);
        let verified = planned_ra::verify(
            &source(&plan),
            plan,
            &witness,
            policy(ProjectionPolicy::Eager),
        )
        .unwrap();
        assert_eq!(
            verified.report().join_keys[result],
            expected.into_iter().map(String::from).collect()
        );
    }
}

#[test]
fn arity_counts_reduction_join_results_before_projection() {
    let plan = parse(case("ya_composite").plan);
    let (witness, _) = reference("ya_composite");
    let error = planned_ra::verify(
        &source(&plan),
        plan,
        &witness,
        Policy {
            projection: ProjectionPolicy::Eager,
            max_arity: Some(3),
        },
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("arity 4 exceeds limit 3"),
        "{error}"
    );
}

#[test]
fn stale_reduction_versions_are_rejected_despite_correct_final_answers() {
    for (name, from, to) in [
        (
            "ya_delivery",
            "down_d = natural_join d0 with s2;",
            "down_d = natural_join d0 with s1;",
        ),
        (
            "ya_deep",
            "up_s = natural_join s0 with t1;",
            "up_s = natural_join s0 with t0;",
        ),
        (
            "ya_deep",
            "down_u = natural_join u0 with t2;",
            "down_u = natural_join u0 with t1;",
        ),
    ] {
        let fixture = case(name);
        let text = fixture.plan.replace(from, to);
        assert_ne!(text, fixture.plan);
        let wrong = parse(&text);
        let db = database(fixture);
        assert_eq!(
            evaluate_ra(&wrong, &db).output,
            evaluate_cq(&wrong.program, &db)
        );
        reject(name, text, "latest reduction versions");
    }
}

#[test]
fn reduction_cannot_shrink_an_occurrence_schema() {
    // Still locally valid: later operands restore the output columns.
    reject(
        "ya_delivery",
        case("ya_delivery").plan.replace(
            "d1 = project down_d keep {driver, depot};",
            "d1 = project down_d keep {driver};",
        ),
        "reduction schema",
    );
}

#[test]
fn reductions_cannot_be_claimed_without_a_tree_witness() {
    let plan = parse(case("ya_delivery").plan);
    let (mut witness, _) = reference("ya_delivery");
    witness.reduction_tree = None;
    let error = planned_ra::verify(
        &source(&plan),
        plan,
        &witness,
        policy(ProjectionPolicy::Safe),
    )
    .unwrap_err();
    // Without the reduction judgment this is ordinary assembly: projecting
    // away D's head variable is already illegal at the first upward step.
    assert!(error.to_string().contains("unsafe projection"), "{error}");
}

#[test]
fn tree_witnesses_require_connectivity_and_running_intersection() {
    for (name, parents, message) in [
        (
            "triangle",
            vec![None, Some(0), Some(1)],
            "running intersection",
        ),
        (
            "ya_deep",
            vec![None, Some(0), Some(3), Some(2)],
            "connected tree",
        ),
        ("ya_deep", vec![None, None, Some(1), Some(2)], "one root"),
        (
            "ya_deep",
            vec![None, Some(9), Some(1), Some(2)],
            "parent occurrence",
        ),
    ] {
        let plan = parse(case(name).plan);
        let (mut witness, _) = reference(name);
        witness.reduction_tree = Some(parents);
        let error = planned_ra::verify(
            &source(&plan),
            plan,
            &witness,
            policy(ProjectionPolicy::Safe),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(message),
            "{name}: expected {message}, got {error}"
        );
    }
}

#[test]
fn final_assembly_must_consume_the_frozen_reduced_occurrences() {
    reject(
        "ya_deep",
        case("ya_deep").plan.replace(
            "rst = natural_join live_c with t2;",
            "rst = natural_join live_c with t1;",
        ),
        "assembly frontier",
    );
}

#[test]
fn omitted_downward_pass_is_rejected_with_correct_answers() {
    let fixture = case("ya_disconnected");
    let text = fixture
        .plan
        .replace("    down_s = natural_join s0 with r1;\n", "")
        .replace("    s1 = project down_s keep {y, z};\n", "")
        .replace("exists = project s1", "exists = project s0");
    let wrong = parse(&text);
    let db = database(fixture);
    assert_eq!(
        evaluate_ra(&wrong, &db).output,
        evaluate_cq(&wrong.program, &db)
    );
    reject("ya_disconnected", text, "reduction step requires");
}

#[test]
fn zero_column_intermediates_remain_existence_obligations() {
    // Source inputs and heads require positive arity. Zero-column results
    // arise inside the algebra through projection of existential components.
    let fixture = case("existential");
    let plan = parse(fixture.plan);
    let witness = reference("existential").0;
    let verified = planned_ra::verify(
        &source(&plan),
        plan.clone(),
        &witness,
        policy(ProjectionPolicy::Eager),
    )
    .unwrap();
    assert!(verified.report().headings["exists"].is_empty());
    let wrong = parse(
        &fixture
            .plan
            .replace("    s = rename S {first -> y, second -> z};\n", "")
            .replace("exists = project s keep {};", "exists = unit;"),
    );
    let error = planned_ra::verify(
        &source(&plan),
        wrong,
        &witness,
        policy(ProjectionPolicy::Eager),
    )
    .unwrap_err();
    assert!(error.to_string().contains("occurrence coverage"), "{error}");
}

#[test]
fn singleton_tree_requires_no_reduction_steps() {
    let plan = parse(case("scan").plan);
    let mut witness = reference("scan").0;
    witness.reduction_tree = Some(vec![None]);
    planned_ra::verify(
        &source(&plan),
        plan,
        &witness,
        policy(ProjectionPolicy::Eager),
    )
    .unwrap();
}

#[test]
fn leaf_columns_are_pruned_before_combination() {
    let query = "struct P; relation R(x: i32, unused: i32); relation S(x: i32);
        answer(x) :- R(x, unused), S(x);";
    let source: cq::Module = syn::parse_str(query).unwrap();
    let witness = Witness {
        occurrences: vec!["r".into(), "s".into()],
        reduction_tree: None,
    };
    let wrong = parse(&format!(
        "{query} relational {{
        r = rename R {{x -> x, unused -> unused}}; s = rename S {{x -> x}};
        j = natural_join r with s; result = project j keep {{x}};
        output result as answer(x); }}"
    ));
    let error =
        planned_ra::verify(&source, wrong, &witness, policy(ProjectionPolicy::Eager)).unwrap_err();
    assert!(error.to_string().contains("eager pruning"), "{error}");
    let pruned = parse(&format!(
        "{query} relational {{
        r = rename R {{x -> x, unused -> unused}}; s = rename S {{x -> x}};
        live = project r keep {{x}}; result = natural_join live with s;
        output result as answer(x); }}"
    ));
    planned_ra::verify(&source, pruned, &witness, policy(ProjectionPolicy::Eager)).unwrap();
}

#[test]
fn valid_names_and_definition_order_are_not_canonicalized() {
    let text = case("composite")
        .plan
        .replace("r = rename", "left_input = rename")
        .replace("natural_join r with s", "natural_join s with left_input");
    let plan = parse(&text);
    let mut witness = reference("composite").0;
    witness.occurrences[0] = "left_input".into();
    planned_ra::verify(
        &source(&plan),
        plan,
        &witness,
        policy(ProjectionPolicy::Eager),
    )
    .unwrap();
}
