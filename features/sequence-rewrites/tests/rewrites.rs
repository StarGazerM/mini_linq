use cq_lang_sequence::{Graded, OutputProtocol, Program, Term, check_rewrite, contract};
use cq_sequence_plan::rewrites::{choose, grade, output, store, store_level, tile};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

type Rewrite = fn(&Program, TokenStream) -> syn::Result<Program>;

fn program(body: TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        }
        structures { r host scan = Reader []; r canon = Canon; }
        sequence { #body }
    })
    .unwrap()
}

fn apply(pass: Rewrite, source: &Program, advice: TokenStream) -> Program {
    let target = pass(source, advice).unwrap();
    contract::check(&target).unwrap();
    check_rewrite(source, &target).unwrap();
    assert_eq!(
        syn::parse2::<Program>(target.to_token_stream()).unwrap(),
        target
    );
    target
}

#[test]
fn store_materializes_a_named_producer_and_is_idempotent() {
    let source = program(quote!(a = rows r; b = canon a; out = store b;));
    let target = apply(store, &source, quote!(b));
    assert_eq!(
        target.sequence.definitions[1].term,
        syn::parse2::<Term>(quote!(store canon a)).unwrap()
    );
    assert_eq!(
        target.sequence.definitions[0],
        source.sequence.definitions[0]
    );
    assert_eq!(
        target.sequence.definitions[2],
        source.sequence.definitions[2]
    );
    assert_eq!(apply(store, &target, quote!(b)), target);
    for advice in [quote!(), quote!(missing), quote!(a, b)] {
        assert!(store(&source, advice).is_err());
    }
}

#[test]
fn store_flatten_and_select_results() {
    let source = program(quote!(a = rows r; b = flatten tile 8 a; c = flatten tile 4 b;));
    let target = apply(store_level, &source, quote!(b));
    assert_eq!(
        target.sequence.definitions[1].term,
        syn::parse2::<Term>(quote!(store flatten tile 8 a)).unwrap()
    );
    assert_eq!(
        target.sequence.definitions[2],
        source.sequence.definitions[2]
    );
    for advice in [quote!(), quote!(all), quote!(b, c), quote!(b, c,)] {
        let target = apply(store_level, &source, advice);
        assert!(matches!(
            target.sequence.definitions[1].term,
            Term::Store { .. }
        ));
        assert!(matches!(
            target.sequence.definitions[2].term,
            Term::Store { .. }
        ));
    }
    for advice in [quote!(a), quote!(missing), quote!(b, b), quote!(all, b)] {
        assert!(store_level(&source, advice).is_err());
    }
}

#[test]
fn tile_wraps_and_preserves_lengths() {
    let source = program(quote!(a = rows r;));
    for (advice, expected) in [
        (quote!(a, 8), quote!(tile 8 rows r)),
        (quote!(a, 8, items 4), quote!(tile 8 items 4 rows r)),
    ] {
        let target = apply(tile, &source, advice);
        assert_eq!(
            target.sequence.definitions[0].term,
            syn::parse2::<Term>(expected).unwrap()
        );
        assert_eq!(
            contract::check(&source).unwrap().lengths,
            contract::check(&target).unwrap().lengths
        );
    }
    for advice in [
        quote!(missing, 8),
        quote!(a, 0),
        quote!(a, 8, items 0),
        quote!(a, 8, junk 4),
        quote!(a, 8, items 4,),
    ] {
        assert!(tile(&source, advice).is_err());
    }
}

#[test]
fn tile_requires_capacity_for_unknown_store() {
    let source = program(quote! {
        a = rows r;
        b = canon a;
        d = store choose [len r < 4 => a, else => b];
    });
    assert!(tile(&source, quote!(d, 8)).is_err());
    let source = program(quote! {
        a = rows r;
        b = canon a;
        d = store capacity 64 choose [len r < 4 => a, else => b];
    });
    apply(tile, &source, quote!(d, 8));
}

#[test]
fn choose_uses_exhaustive_equal_support_alternatives() {
    let source = program(quote!(a = rows r; b = canon a; d = a;));
    let target = apply(choose, &source, quote!(d, len r < 64 => a, else => b));
    assert_eq!(
        target.sequence.definitions[2].term,
        syn::parse2::<Term>(quote!(choose [len r < 64 => a, else => b])).unwrap()
    );
    for advice in [
        quote!(missing, len r < 64 => a, else => b),
        quote!(d, len r < 64 => a, else => d),
        quote!(d, len r >= 64 => a, else => b),
        quote!(d, len r < 64 => a),
        quote!(d, len missing < 64 => a, else => b),
    ] {
        assert!(choose(&source, advice).is_err());
    }
    let different = program(quote!(a = rows r; b = fold count a; d = a;));
    assert!(choose(&different, quote!(d, len r < 64 => a, else => b)).is_err());
    // The alternatives agree with one another, but changing the final support is forbidden.
    let changed = program(quote!(a = rows r; b = a; d = fold count a;));
    assert!(choose(&changed, quote!(d, len r < 64 => a, else => b)).is_err());
}

#[test]
fn output_sets_each_protocol_and_enforces_capacity() {
    let source = program(quote!(a = store capacity 64 rows r;));
    for (advice, protocol) in [
        (quote!(count), OutputProtocol::Count),
        (quote!(reserve), OutputProtocol::Reserve),
        (quote!(bounded), OutputProtocol::Bounded),
    ] {
        let target = apply(output, &source, advice);
        assert_eq!(target.sequence.output, Some(protocol));
    }
    assert!(output(&program(quote!(a = store rows r;)), quote!(bounded)).is_err());
    assert!(output(&program(quote!(a = rows r;)), quote!(count)).is_err());
    assert!(output(&source, quote!(unknown)).is_err());
    assert!(output(&source, quote!(count reserve)).is_err());
}

#[test]
fn grade_extends_and_updates_without_resolving_machine() {
    let source = program(quote!(a = rows r; b = store a;));
    let first = apply(grade, &source, quote!(custom, a at host));
    let second = apply(grade, &first, quote!(custom, b at device.block.lane));
    let target = apply(grade, &second, quote!(custom, a at device));
    let expected: Graded =
        syn::parse2(quote!(graded custom { a at device; b at device.block.lane; })).unwrap();
    assert_eq!(target.graded, Some(expected));
    for advice in [
        quote!(other, a at host),
        quote!(custom, absent at host),
        quote!(custom, a at host, a at device),
        quote!(custom, a at),
        quote!(custom, a at host.),
    ] {
        assert!(grade(&first, advice).is_err());
    }
    apply(grade, &source, quote!(custom));
}

#[test]
fn every_pass_checks_the_source_contract() {
    let invalid = program(quote!(a = rows missing; b = flatten tile 8 a;));
    for (pass, advice) in [
        (store_level as Rewrite, quote!(b)),
        (tile, quote!(a, 8)),
        (choose, quote!(b, len r < 8 => a, else => a)),
        (output, quote!(count)),
        (grade, quote!(custom, a at host)),
    ] {
        assert!(pass(&invalid, advice).is_err());
    }
}

#[test]
fn evidence_advice_is_checked_replayable_metadata() {
    let source = program(quote!(out = store rows r;));
    let target = apply(
        cq_sequence_plan::rewrites::evidence,
        &source,
        quote!(evidence {
            count R = 0;
            key R [a];
            foreign_key R [a] -> r [a];
            selectivity out = 0 / 1;
            bounds R.a = 42 ..= -42;
            sorted R [a];
        }),
    );
    assert_eq!(target.sequence, source.sequence);
    assert_eq!(target.structures, source.structures);
    assert_eq!(target.graded, source.graded);
    assert_eq!(target.evidence.count("R"), Some(0));
    assert_eq!(contract::check(&target).unwrap().evidence, target.evidence);
    assert_eq!(
        cq_sequence_plan::rewrites::record_evidence(&target, &target.evidence).unwrap(),
        target
    );
    let extended = apply(
        cq_sequence_plan::rewrites::evidence,
        &target,
        quote!(evidence { count R = 123; }),
    );
    assert_eq!(
        &extended.evidence.facts[..target.evidence.facts.len()],
        &target.evidence.facts
    );
    assert_eq!(extended.evidence.count("R"), Some(123));
    assert!(
        cq_sequence_plan::rewrites::evidence(&source, quote!(evidence { count absent = 1; }))
            .is_err()
    );
    assert!(
        cq_sequence_plan::rewrites::evidence(&source, quote!(evidence { sorted R [absent]; }))
            .is_err()
    );
}

#[test]
fn grouped_key_premises_remove_only_proven_valuation_canon() {
    let source: Program = syn::parse_quote! {
        physical {
            relation R(id:i64, value:i64);
            r = pipe canon rename R@0 {id -> id, value -> value};
            grouped = let group r by [] { total: i64 = sum(value); };
            return grouped as answer(total);
        }
        structures { r host scan = Reader []; r canon = Canon; grouped rows = Rows; }
        sequence { values = store canon rows r; answer = store group grouped values; }
    };
    assert!(cq_sequence_plan::rewrites::group_without_canon(&source, quote!(answer)).is_err());
    let mut keyed = source.clone();
    keyed.evidence = syn::parse_quote!(evidence { R(id) key; });
    let target = cq_sequence_plan::rewrites::group_without_canon(&keyed, quote!(answer)).unwrap();
    assert!(matches!(target.sequence.definitions[0].term, Term::Rows(_)));
    contract::check_rewrite(&keyed, &target).unwrap();
    assert!(
        contract::check_rewrite(&source, &target).is_err(),
        "the rewrite cannot invent an unasserted key"
    );
    let mut checker = cq_lang_sequence::contract::Checker::new(&target).unwrap();
    let mut missing = target.clone();
    missing.evidence.facts.clear();
    assert!(
        checker.check(&missing).is_err(),
        "cached proofs must retain key premises"
    );
    let mut projected = keyed.clone();
    if let cq_lang_sequence::structures::physical::Operator::Rename(rename) =
        &mut projected.structures.physical.program.definitions[0].operator
    {
        rename.mapping = rename
            .mapping
            .iter()
            .filter(|mapping| mapping.source != "id")
            .cloned()
            .collect();
    }
    assert!(cq_sequence_plan::rewrites::group_without_canon(&projected, quote!(answer)).is_err());
}

#[test]
fn grouped_join_requires_keys_for_every_enumerated_atom() {
    let mut source: Program = syn::parse_quote! {
        physical {
            relation L(id:i64, a:i64); relation R(a:i64,b:i64);
            l = pipe rename L@0 {id -> id,a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = let canon probe first [a] l,r;
            grouped = let group j by [] { total:i64=sum(b); };
            return grouped as answer(total);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            r = Provider [a]; j rows = Rows; j canon = Canon; grouped rows = Rows;
        }
        sequence {
            l = rows l; r = store rows r;
            j = store canon flatten lookup r [a] l;
            result = store group grouped j;
        }
        evidence { L(id) key; }
    };
    assert!(cq_sequence_plan::rewrites::group_without_canon(&source, quote!(result)).is_err());
    source.evidence = syn::parse_quote!(evidence { L(id) key; R(a,b) key; });
    let target = cq_sequence_plan::rewrites::group_without_canon(&source, quote!(result)).unwrap();
    let Term::Store { inner, .. } = &target.sequence.definitions[2].term else {
        panic!("expanding input keeps its flat publication");
    };
    assert!(matches!(inner.as_ref(), Term::Flatten(_)));
}
