use cq_lang_sequence::{Graded, OutputProtocol, Program, Term, check_rewrite, contract};
use cq_sequence_rewrite_passes::{choose, grade, output, store, store_level, tile};
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
