use cq_lang_physical as physical;
use quote::quote;

const HOIST_SOURCE: &str = "physical {
    relation L(a:i32,b:i32);
    relation R(b:i32,c:i32);
    relation S(a:i32,c:i32);
    l = pipe rename L {a -> a,b -> b};
    r = pipe rename R {b -> b,c -> c};
    candidate = pipe probe first [b] l,r;
    s = pipe rename S {a -> a,c -> c};
    checked = pipe canon probe first [a,c] candidate,s;
    return checked as answer(a,b,c);
}";

fn hoisted_smallest(source: &physical::Program) -> physical::Program {
    let mut target = source.clone();
    let renamed = target.program.definitions.remove(3);
    target.program.definitions.insert(2, renamed);
    let physical::Operator::Probe(_, key) = &mut target.program.definitions[3].operator else {
        panic!("candidate is a probe");
    };
    key.choice = Some(physical::Choice::Smallest(Default::default()));
    target
}

#[test]
fn independent_rename_hoist_enables_smallest_through_public_rewrite() {
    let source: physical::Program = syn::parse_str(HOIST_SOURCE).unwrap();
    let target = hoisted_smallest(&source);
    physical::contract::check(&source).unwrap();
    physical::contract::check(&target).unwrap();
    physical::check_rewrite(&source, &target).unwrap();
    // Relational definitions can commute in either direction within one region.
    physical::check_rewrite(&target, &source).unwrap();

    let continued = cq_stage::rewrite::<physical::Program>(
        quote!(pipeline { finish }; #source),
        |parsed, _| Ok(hoisted_smallest(parsed)),
    );
    assert_eq!(
        continued.to_string(),
        quote!(finish! { pipeline {}; #target }).to_string()
    );
}

const RUST_SOURCE: &str = "physical {
    relation L(a:i32);
    relation R(a:i32,b:i32);
    relation S(a:i32);
    r = pipe rename L {a -> a};
    s = pipe rename R {a -> a,b -> b};
    selected = pipe select s if (b = 0);
    projected = let project selected keep {a};
    g = pipe guard r if (accept(a));
    t = pipe rename S {a -> a};
    b = pipe bind g with c:i32 = (compute(a));
    j = pipe probe first [a] b,projected;
    checked = pipe semi eager [a] j,t;
    return checked as answer(a,c);
}";

fn move_definition(source: &physical::Program, from: usize, to: usize) -> physical::Program {
    let mut target = source.clone();
    let definition = target.program.definitions.remove(from);
    target.program.definitions.insert(to, definition);
    target
}

fn reject_valid_target(source: &physical::Program, target: &physical::Program) -> String {
    physical::contract::check(source).unwrap();
    physical::contract::check(target).unwrap();
    physical::check_rewrite(source, target)
        .unwrap_err()
        .to_string()
}

fn reject_changed_graph(source: &physical::Program, target: &physical::Program) {
    assert!(reject_valid_target(source, target).contains("RA erasure"));
}

#[test]
fn relational_hoist_across_rust_preserves_payloads_with_admissible_decisions() {
    let source: physical::Program = syn::parse_str(RUST_SOURCE).unwrap();
    // Hoist t past g; g and b retain their written relative order and edges.
    let mut target = move_definition(&source, 5, 1);
    physical::check_rewrite(&source, &target).unwrap();
    assert!(reject_valid_target(&target, &source).contains("Before"));

    target.program.definitions[0].mode = syn::parse_quote!(let);
    target.program.definitions[0].canon = Some(Default::default());
    target.program.definitions[6].mode = syn::parse_quote!(let);
    target.program.definitions[6].canon = Some(Default::default());
    target.program.definitions[7].operator = syn::parse_quote!(merge [a] b,projected);
    target.program.definitions[8].operator = syn::parse_quote!(semi lazy [a] j,t);
    physical::contract::check(&target).unwrap();
    physical::check_rewrite(&source, &target).unwrap();
}

#[test]
fn every_source_before_obligation_survives_each_rust_boundary() {
    let source: physical::Program = syn::parse_str(RUST_SOURCE).unwrap();
    // Move a projection later across g, or a rename later across b.
    for (from, to) in [(3, 4), (5, 6)] {
        let target = move_definition(&source, from, to);
        assert!(reject_valid_target(&source, &target).contains("Before"));
    }
    // A whole dependency-preserving relational branch also cannot move later.
    let mut target = source.clone();
    let branch: Vec<_> = target.program.definitions.drain(1..4).collect();
    target.program.definitions.splice(2..2, branch);
    assert!(reject_valid_target(&source, &target).contains("Before"));

    // Moving g earlier loses the same preceding branch, even though g's input
    // and the written Rust chain are still accepted by the target contract.
    let target = move_definition(&source, 4, 1);
    assert!(reject_valid_target(&source, &target).contains("Before"));
    let rejected =
        cq_stage::rewrite::<physical::Program>(quote!(pipeline { finish }; #source), |_, _| {
            Ok(target)
        });
    assert!(rejected.to_string().contains("compile_error"));
    assert!(rejected.to_string().contains("Before"));
}

#[test]
fn rewritten_rust_chain_order_and_operands_are_rejected() {
    let source: physical::Program = syn::parse_str(RUST_SOURCE).unwrap();
    let mut target = source.clone();
    target.program.definitions[4] = syn::parse_quote!(b = pipe bind r with c:i32 = (compute(a)););
    target.program.definitions[6] = syn::parse_quote!(g = pipe guard b if (accept(a)););
    target.program.definitions[7].operator = syn::parse_quote!(probe first [a] g,projected);
    reject_changed_graph(&source, &target);

    // A written reorder with the original edges is not topological.
    target = source.clone();
    target.program.definitions.swap(4, 6);
    assert!(physical::check_rewrite(&source, &target).is_err());
}

#[test]
fn named_graph_comparison_rejects_changed_relational_and_opaque_payloads() {
    let source: physical::Program = syn::parse_str(RUST_SOURCE).unwrap();
    for (position, operator) in [
        (0, syn::parse_quote!(rename S {a -> a})),
        (2, syn::parse_quote!(select s if (b = 1))),
        (4, syn::parse_quote!(guard r if (other_accept(a)))),
        (6, syn::parse_quote!(bind g with c:i32 = (other_compute(a)))),
        (6, syn::parse_quote!(bind g with c:i64 = (compute(a)))),
        (8, syn::parse_quote!(probe first [a] j,t)),
    ] {
        // Include a legal reorder so payloads must be matched by name.
        let mut target = move_definition(&source, 5, 1);
        let definition = target
            .program
            .definitions
            .iter_mut()
            .find(|d| d.name == source.program.definitions[position].name)
            .unwrap();
        definition.operator = operator;
        reject_changed_graph(&source, &target);
    }

    // Join keys are operator payload too, even when both orders are admissible.
    let source: physical::Program = syn::parse_str(HOIST_SOURCE).unwrap();
    let mut target = hoisted_smallest(&source);
    target.program.definitions[4].operator = syn::parse_quote!(probe first [c,a] candidate,s);
    reject_changed_graph(&source, &target);
}

#[test]
fn changed_edges_are_rejected_even_when_reads_and_headings_are_valid() {
    let source: physical::Program = syn::parse_str(RUST_SOURCE).unwrap();
    let mut target = source.clone();
    target.program.definitions[6].mode = syn::parse_quote!(let);
    target.program.definitions[7].operator = syn::parse_quote!(probe first [a] projected,b);
    reject_changed_graph(&source, &target);
}

#[test]
fn dropped_extra_or_renamed_definitions_are_rejected() {
    let source: physical::Program = syn::parse_str(RUST_SOURCE).unwrap();
    let mut dropped = source.clone();
    dropped.program.definitions.remove(2);
    dropped.program.definitions[2].operator = syn::parse_quote!(project s keep {a});
    reject_changed_graph(&source, &dropped);

    let mut extra = source.clone();
    extra.program.definitions[2].operator = syn::parse_quote!(select extra if (b = 0));
    extra
        .program
        .definitions
        .insert(2, syn::parse_quote!(extra = pipe select s if (b = 1);));
    reject_changed_graph(&source, &extra);

    let mut renamed = source.clone();
    renamed.program.definitions[2].name = syn::parse_quote!(replacement);
    renamed.program.definitions[3].operator = syn::parse_quote!(project replacement keep {a});
    reject_changed_graph(&source, &renamed);
}

#[test]
fn declarations_head_and_returned_result_are_preserved() {
    let source: physical::Program = syn::parse_str(RUST_SOURCE).unwrap();
    let mut declaration = source.clone();
    declaration.program.declarations[0] = syn::parse_quote!(relation L(a:i64););
    let mut head_name = source.clone();
    head_name.program.head.relation = syn::parse_quote!(other);
    let mut head_args = source.clone();
    head_args.program.head = syn::parse_quote!(answer(c, a));
    for target in [declaration, head_name, head_args] {
        reject_changed_graph(&source, &target);
    }
    let mut returned = source.clone();
    returned.program.returned = syn::parse_quote!(j);
    assert!(physical::check_rewrite(&source, &returned).is_err());
}

#[test]
fn hoisting_does_not_bypass_either_contract_or_smallest_read_mode_premises() {
    let source: physical::Program = syn::parse_str(HOIST_SOURCE).unwrap();
    let target = hoisted_smallest(&source);
    let mut nonadjacent = source.clone();
    nonadjacent.program.definitions[2].operator = syn::parse_quote!(probe smallest [b] l,r);
    assert!(physical::check_rewrite(&nonadjacent, &target).is_err());
    assert!(physical::check_rewrite(&target, &nonadjacent).is_err());

    let mut retained_candidate = target.clone();
    retained_candidate.program.definitions[3].mode = syn::parse_quote!(let);
    let mut canonical_candidate = target.clone();
    canonical_candidate.program.definitions[3].canon = Some(Default::default());
    let mut repeated_canon_pipe = target.clone();
    repeated_canon_pipe.program.definitions[2].canon = Some(Default::default());
    let mut missing_dependency = target.clone();
    missing_dependency.program.definitions.swap(1, 3);
    for invalid in [
        retained_candidate,
        canonical_candidate,
        repeated_canon_pipe,
        missing_dependency,
    ] {
        assert!(physical::check_rewrite(&source, &invalid).is_err());
        assert!(physical::check_rewrite(&invalid, &target).is_err());
    }
}
