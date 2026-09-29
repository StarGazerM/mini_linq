//! IX → Physical: the pass's output equals independently authored Physical
//! programs, and every output satisfies the Physical contract.
use cq_ir::policy::Policy;
use cq_ir::{cq, index_requirements as ix, physical};
use cq_physical_lowering::{check, compile};
use quote::{ToTokens, quote};

fn parse_policy(tokens: proc_macro2::TokenStream) -> Policy {
    syn::parse2(tokens).unwrap()
}

/// No reductions: relation-at-a-time joins in written order.
const UNREDUCED: cq_reduce::Options = cq_reduce::Options {
    guards: cq_reduce::Pushdown::None,
    semijoins: cq_reduce::Reductions::None,
};

/// Single-atom guards pushed onto their atoms and the Yannakakis full reducer.
const REDUCED: cq_reduce::Options = cq_reduce::Options {
    guards: cq_reduce::Pushdown::Atoms,
    semijoins: cq_reduce::Reductions::Tree,
};

/// CQ text → NF → VO in written order → reduced under `options` → RA → IX.
fn ix_of(tokens: proc_macro2::TokenStream, options: &cq_reduce::Options) -> ix::Module {
    let source: cq::Module = syn::parse2(tokens).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let plan = cq_order::compile(&normal, &cq_order::Options::default()).unwrap();
    let plan = cq_reduce::compile(&plan, options).unwrap();
    let target = cq_elaborate::compile(&plan).unwrap();
    cq_index_policy::compile(&target).unwrap()
}

/// `r0 = L; r1 = R; r2 = r0 ⋈ r1; r3 = S; r4 = r2 ⋈ r3; r5 = π r4`.
fn chain_ix() -> ix::Module {
    ix_of(
        quote! {
            struct Chain;
            relation L(a: i32);
            relation R(a: i32, b: i32);
            relation S(b: i32, c: i32);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        },
        &UNREDUCED,
    )
}

/// The pass's output, its contract checked, as the authored program text.
fn lower(source: &ix::Module, policy: &Policy) -> (physical::Module, String) {
    let target = compile(source, policy).unwrap();
    physical::contract::check(&target).unwrap();
    assert!(check(source, &target, policy));
    let text = target.program.to_token_stream().to_string();
    (target, text)
}

fn assert_program(target: &physical::Module, program: proc_macro2::TokenStream) {
    let source = &target.index_requirements;
    let expected: physical::Module = syn::parse2(quote! { #source physical { #program } }).unwrap();
    assert_eq!(
        target.program.to_token_stream().to_string(),
        expected.program.to_token_stream().to_string()
    );
    assert_eq!(*target, expected);
}

#[test]
fn seek_policy_probes_the_hash_chain_and_flags_the_projected_return() {
    let source = chain_ix();
    let policy = parse_policy(quote! { policy { L; R; S; } });
    let (target, _) = lower(&source, &policy);
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = let rename;
            r2 = pipe probe[a] r0, r1;
            r3 = let rename;
            r4 = pipe probe[b] r2, r3;
            r5 = pipe canon project;
            return r5;
        },
    );
    // The merge policy decides a different program.
    let merge = parse_policy(quote! { policy { L; R; S; join = merge; } });
    assert!(!check(&source, &target, &merge));
}

#[test]
fn keyed_hosts_answer_probes_in_place_and_canon_hosts_make_the_return_unique() {
    let source = chain_ix();
    let policy = parse_policy(quote! {
        policy { L canon; R keyed [a] canon; S keyed [b] canon; }
    });
    let (target, _) = lower(&source, &policy);
    // `project` keeps `a` and `c` but drops the key column `b`: not unique.
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = alias rename;
            r2 = pipe probe[a] r0, r1;
            r3 = alias rename;
            r4 = pipe probe[b] r2, r3;
            r5 = pipe canon project;
            return r5;
        },
    );

    // `r3 = project r2 keep {a, b}` keeps the whole key of L ⋈ R.
    let full = ix_of(
        quote! {
            struct Full;
            relation L(a: i32);
            relation R(a: i32, b: i32);
            answer(a, b) :- L(a), R(a, b);
        },
        &UNREDUCED,
    );
    let policy = parse_policy(quote! { policy { L canon; R keyed [a] canon; } });
    let (_, text) = lower(&full, &policy);
    assert!(text.contains("r3 = pipe project ;"), "{text}");
    let policy = parse_policy(quote! { policy { L canon; R keyed [a]; } });
    let (_, text) = lower(&full, &policy);
    assert!(text.contains("r3 = pipe canon project ;"), "{text}");
}

#[test]
fn merge_policy_merges_every_join_and_materializes_the_side_whose_key_changes() {
    let source = chain_ix();
    let policy = parse_policy(quote! { policy { L; R; S; join = merge; } });
    let (target, _) = lower(&source, &policy);
    assert_program(
        &target,
        quote! {
            r0 = let rename;
            r1 = let rename;
            r2 = let merge[a] r0, r1;
            r3 = let rename;
            r4 = pipe merge[b] r2, r3;
            r5 = pipe canon project;
            return r5;
        },
    );

    // Ordered hosts are merged in place; a merge sorted on the next key
    // stays a pipe.
    let same_key = ix_of(
        quote! {
            struct SameKey;
            relation L(a: i32);
            relation R(a: i32, b: i32);
            relation S(a: i32, c: i32);
            answer(a, b, c) :- L(a), R(a, b), S(a, c);
        },
        &UNREDUCED,
    );
    let policy = parse_policy(quote! {
        policy { L keyed [a] ordered; R keyed [a] ordered; S keyed [a] ordered; join = merge; }
    });
    let (target, _) = lower(&same_key, &policy);
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = alias rename;
            r2 = pipe merge[a] r0, r1;
            r3 = alias rename;
            r4 = pipe merge[a] r2, r3;
            r5 = pipe canon project;
            return r5;
        },
    );
    // An unordered key does not answer a merge: the rename is built over.
    let unordered = parse_policy(quote! {
        policy { L keyed [a]; R keyed [a] ordered; S keyed [a] ordered; join = merge; }
    });
    let (_, text) = lower(&same_key, &unordered);
    assert!(text.contains("r0 = let rename ;"), "{text}");
    assert!(text.contains("r1 = alias rename ;"), "{text}");
}

#[test]
fn a_cartesian_join_is_a_unit_probe_under_either_join_rule() {
    let source = ix_of(
        quote! {
            struct Cross;
            relation L(a: i32);
            relation R(b: i32);
            answer(a, b) :- L(a), R(b);
        },
        &UNREDUCED,
    );
    for policy in [
        parse_policy(quote! { policy { L; R; } }),
        parse_policy(quote! { policy { L; R; join = merge; } }),
    ] {
        let (target, text) = lower(&source, &policy);
        assert!(text.contains("r2 = pipe probe [] r0 , r1 ;"), "{text}");
        let analysis = physical::contract::check(&target).unwrap();
        assert_eq!(analysis.key_reads["r1"], vec![("r2".to_string(), vec![])]);
    }
}

#[test]
fn comparisons_on_the_ordered_host_key_become_a_range() {
    let source = ix_of(
        quote! {
            struct Windowed;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if a >= 2 && 9 > a && b != 5, if b >= 0;
        },
        &UNREDUCED,
    );
    let ordered = parse_policy(quote! { policy { R keyed [a] ordered; } });
    let (target, _) = lower(&source, &ordered);
    // The second select's input is not a host rename: it keeps its guard.
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = pipe range[a];
            r2 = pipe select;
            r3 = pipe canon project;
            return r3;
        },
    );
    let analysis = physical::contract::check(&target).unwrap();
    assert_eq!(
        analysis.range_reads["r0"],
        vec![("r1".to_string(), "a".to_string())]
    );

    // The key must be ordered, single, and bounded by the condition.
    let unordered = parse_policy(quote! { policy { R keyed [a]; } });
    let (_, text) = lower(&source, &unordered);
    assert!(text.contains("r1 = pipe select ;"), "{text}");
    let other_column = parse_policy(quote! { policy { R keyed [b] ordered; } });
    let (_, text) = lower(&source, &other_column);
    assert!(text.contains("r1 = pipe select ;"), "{text}");
    let unbounded = ix_of(
        quote! {
            struct Unbounded;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if a != 3 && b < 4;
        },
        &UNREDUCED,
    );
    let (_, text) = lower(&unbounded, &ordered);
    assert!(text.contains("r1 = pipe select ;"), "{text}");
    let reversed = ix_of(
        quote! {
            struct Reversed;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if b < 4 && 7 <= a;
        },
        &UNREDUCED,
    );
    let (_, text) = lower(&reversed, &ordered);
    assert!(text.contains("r1 = pipe range [a] ;"), "{text}");
    // A strict bound past the end of `i32` is no window.
    let overflowing = ix_of(
        quote! {
            struct Overflowing;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if a > 2147483647;
        },
        &UNREDUCED,
    );
    let (_, text) = lower(&overflowing, &ordered);
    assert!(text.contains("r1 = pipe select ;"), "{text}");
}

#[test]
fn full_reduction_semijoins_test_membership_and_shared_results_are_lets() {
    // r0 = R; r1 = L; r2 = r0 ⋉ r1; r3 = r1 ⋉ r2; r4 = r3 ⋈ r2; r5 = π r4.
    let source = ix_of(
        quote! {
            struct Reduced;
            relation L(a: i32, b: i32);
            relation R(b: i32, c: i32);
            answer(a) :- L(a, b), R(b, c);
        },
        &REDUCED,
    );
    let policy = parse_policy(quote! { policy { L keyed [b]; R; } });
    let (target, _) = lower(&source, &policy);
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = alias rename;
            r2 = let semi[b] r0, r1;
            r3 = pipe semi[b] r1, r2;
            r4 = pipe probe[b] r3, r2;
            r5 = pipe canon project;
            return r5;
        },
    );
    let analysis = physical::contract::check(&target).unwrap();
    assert_eq!(
        analysis.key_reads["r1"],
        vec![("r2".to_string(), vec!["b".to_string()])]
    );
    assert_eq!(
        analysis.key_reads["r2"],
        vec![
            ("r3".to_string(), vec!["b".to_string()]),
            ("r4".to_string(), vec!["b".to_string()])
        ]
    );

    // Without the key, L is built over; with canonical hosts the return is
    // still flagged because `r5` projects the join key away.
    let policy = parse_policy(quote! { policy { L canon; R canon; } });
    let (_, text) = lower(&source, &policy);
    assert!(text.contains("r1 = let rename ;"), "{text}");
    assert!(text.contains("r5 = pipe canon project ;"), "{text}");
}

#[test]
fn every_output_of_the_chain_query_satisfies_the_contract_under_each_policy() {
    let source = ix_of(
        quote! {
            struct Chain;
            relation L(a: i32);
            relation R(a: i32, b: i32);
            relation S(b: i32, c: i32);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        },
        &REDUCED,
    );
    for policy in [
        quote! { policy { L; R; S; } },
        quote! { policy { L; R; S; join = merge; } },
        quote! { policy { L keyed [a] ordered canon; R keyed [a]; S keyed [b] ordered; } },
        quote! { policy { L keyed [a] ordered canon; R keyed [b] ordered canon; S keyed [b] ordered canon; join = merge; semijoin = prefer_lazy; } },
    ] {
        let policy = parse_policy(policy);
        let (target, _) = lower(&source, &policy);
        assert!(physical::contract::check(&target).is_ok());
    }
}

#[test]
fn the_policy_must_describe_exactly_the_declared_hosts() {
    let source = chain_ix();
    let missing = parse_policy(quote! { policy { L; R; } });
    let error = compile(&source, &missing).unwrap_err().to_string();
    assert!(error.contains("`S`"), "{error}");
    let extra = parse_policy(quote! { policy { L; R; S; T; } });
    let error = compile(&source, &extra).unwrap_err().to_string();
    assert!(error.contains("undeclared"), "{error}");
    let column = parse_policy(quote! { policy { L; R keyed [z]; S; } });
    let error = compile(&source, &column).unwrap_err().to_string();
    assert!(error.contains("no column `z`"), "{error}");
    let twice = parse_policy(quote! { policy { L; R; S; S; } });
    let error = compile(&source, &twice).unwrap_err().to_string();
    assert!(error.contains("twice"), "{error}");
}
