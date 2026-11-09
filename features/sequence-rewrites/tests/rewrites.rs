use cq_lang_sequence::{Graded, OutputProtocol, Program, Term, check_rewrite, contract};
use cq_sequence_plan::rewrites::{choose, grade, output, store, store_level, tile};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use cq_lang_sequence::structures::{Program as Structures, physical::Program as Physical};

fn parse_program(tokens: TokenStream) -> (Physical, Structures, Program) {
    use syn::parse::Parser;
    let parser = |input: syn::parse::ParseStream<'_>| {
        let physical: Physical = input.parse()?;
        let structures = Structures::parse(input, &physical)?;
        let program = input.parse()?;
        Ok((physical, structures, program))
    };
    parser.parse2(tokens).unwrap()
}

type Rewrite = fn(&Physical, &Structures, &Program, TokenStream) -> syn::Result<Program>;

fn program(body: TokenStream) -> (Physical, Structures, Program) {
    parse_program(quote! {
        physical {
            relation R(a:i32);
            r = let canon rename R {a -> a};
            return r as answer(a);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; } r host scan = @reader as Reader []; r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows; }
        sequence { #body }
    })
}

fn apply(pass: Rewrite, physical: &Physical, structures: &Structures, source: &Program, advice: TokenStream) -> Program {
    let target = pass(physical, structures, source, advice).unwrap();
    contract::check(physical, structures, &target).unwrap();
    check_rewrite(physical, structures, source, physical, structures, &target).unwrap();
    assert_eq!(
        syn::parse2::<Program>(target.to_token_stream()).unwrap(),
        target
    );
    target
}

#[test]
fn store_materializes_a_named_producer_and_is_idempotent() {
    let (physical, structures, source) = program(quote!(a = rows r; b = canon a; out = store b;));
    let target = apply(store, &physical, &structures, &source, quote!(b));
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
    assert_eq!(apply(store, &physical, &structures, &target, quote!(b)), target);
    for advice in [quote!(), quote!(missing), quote!(a, b)] {
        assert!(store(&physical, &structures, &source, advice).is_err());
    }
}

#[test]
fn store_flatten_and_select_results() {
    let (physical, structures, source) = program(quote!(a = rows r; b = flatten tile 8 a; c = flatten tile 4 b;));
    let target = apply(store_level, &physical, &structures, &source, quote!(b));
    assert_eq!(
        target.sequence.definitions[1].term,
        syn::parse2::<Term>(quote!(store flatten tile 8 a)).unwrap()
    );
    assert_eq!(
        target.sequence.definitions[2],
        source.sequence.definitions[2]
    );
    for advice in [quote!(), quote!(all), quote!(b, c), quote!(b, c,)] {
        let target = apply(store_level, &physical, &structures, &source, advice);
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
        assert!(store_level(&physical, &structures, &source, advice).is_err());
    }
}

#[test]
fn tile_wraps_and_preserves_lengths() {
    let (physical, structures, source) = program(quote!(a = rows r;));
    for (advice, expected) in [
        (quote!(a, 8), quote!(tile 8 rows r)),
        (quote!(a, 8, items 4), quote!(tile 8 items 4 rows r)),
    ] {
        let target = apply(tile, &physical, &structures, &source, advice);
        assert_eq!(
            target.sequence.definitions[0].term,
            syn::parse2::<Term>(expected).unwrap()
        );
        assert_eq!(
            contract::check(&physical, &structures, &source).unwrap().lengths,
            contract::check(&physical, &structures, &target).unwrap().lengths
        );
    }
    for advice in [
        quote!(missing, 8),
        quote!(a, 0),
        quote!(a, 8, items 0),
        quote!(a, 8, junk 4),
        quote!(a, 8, items 4,),
    ] {
        assert!(tile(&physical, &structures, &source, advice).is_err());
    }
}

#[test]
fn tile_requires_capacity_for_unknown_store() {
    let (physical, structures, source) = program(quote! {
        a = rows r;
        b = canon a;
        d = store choose [len r < 4 => a, else => b];
    });
    assert!(tile(&physical, &structures, &source, quote!(d, 8)).is_err());
    let (physical, structures, source) = program(quote! {
        a = rows r;
        b = canon a;
        d = store capacity 64 choose [len r < 4 => a, else => b];
    });
    apply(tile, &physical, &structures, &source, quote!(d, 8));
}

#[test]
fn choose_uses_exhaustive_equal_support_alternatives() {
    let (physical, structures, source) = program(quote!(a = rows r; b = canon a; d = a;));
    let target = apply(choose, &physical, &structures, &source, quote!(d, len r < 64 => a, else => b));
    assert_eq!(
        target.sequence.definitions[2].term,
        syn::parse2::<Term>(quote!(choose [len r < 64 => a, else => b])).unwrap()
    );
    let target = apply(choose, &physical, &structures, &source, quote!(d, workers < 2 => a, else => b));
    assert_eq!(
        target.sequence.definitions[2].term,
        syn::parse2::<Term>(quote!(choose [workers < 2 => a, else => b])).unwrap()
    );
    for advice in [
        quote!(missing, len r < 64 => a, else => b),
        quote!(d, len r < 64 => a, else => d),
        quote!(d, len r >= 64 => a, else => b),
        quote!(d, len r < 64 => a),
        quote!(d, len missing < 64 => a, else => b),
        quote!(d, workers < 0 => a, else => b),
        quote!(d, workers < 2 => a),
    ] {
        assert!(choose(&physical, &structures, &source, advice).is_err());
    }
    let (physical, structures, different) = program(quote!(a = rows r; b = fold count a; d = a;));
    assert!(choose(&physical, &structures, &different, quote!(d, len r < 64 => a, else => b)).is_err());
    // The alternatives agree with one another, but changing the final support is forbidden.
    let (physical, structures, changed) = program(quote!(a = rows r; b = a; d = fold count a;));
    assert!(choose(&physical, &structures, &changed, quote!(d, len r < 64 => a, else => b)).is_err());
}

#[test]
fn output_sets_each_protocol_and_enforces_capacity() {
    let (physical, structures, source) = program(quote!(a = store capacity 64 rows r;));
    for (advice, protocol) in [
        (quote!(count), OutputProtocol::Count),
        (quote!(reserve), OutputProtocol::Reserve),
        (quote!(reserve_previous), OutputProtocol::ReservePrevious),
        (quote!(bounded), OutputProtocol::Bounded),
    ] {
        let target = apply(output, &physical, &structures, &source, advice);
        assert_eq!(target.sequence.output, Some(protocol));
    }
    let (physical, structures, unbounded) = program(quote!(a = store rows r;));
    let target = apply(output, &physical, &structures, &unbounded, quote!(reserve_previous));
    assert_eq!(
        target.sequence.output,
        Some(OutputProtocol::ReservePrevious)
    );
    assert_eq!(target.sequence.definitions, unbounded.sequence.definitions);
    for (body, advice) in [
        (quote!(a = rows r;), quote!(reserve_previous)),
        (quote!(a = store rows r;), quote!(bounded)),
        (quote!(a = rows r;), quote!(count)),
    ] {
        let (physical, structures, source) = program(body);
        assert!(output(&physical, &structures, &source, advice).is_err());
    }
    assert!(output(&physical, &structures, &source, quote!(unknown)).is_err());
    assert!(output(&physical, &structures, &source, quote!(count reserve)).is_err());
}

#[test]
fn grade_extends_and_updates_without_resolving_machine() {
    let (physical, structures, source) = program(quote!(a = rows r; b = store a;));
    let first = apply(grade, &physical, &structures, &source, quote!(custom, a at host));
    let second = apply(grade, &physical, &structures, &first, quote!(custom, b at device.block.lane));
    let target = apply(grade, &physical, &structures, &second, quote!(custom, a at device));
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
        assert!(grade(&physical, &structures, &first, advice).is_err());
    }
    apply(grade, &physical, &structures, &source, quote!(custom));
}

#[test]
fn every_pass_checks_the_source_contract() {
    let (physical, structures, invalid) = program(quote!(a = rows missing; b = flatten tile 8 a;));
    for (pass, advice) in [
        (store_level as Rewrite, quote!(b)),
        (tile, quote!(a, 8)),
        (choose, quote!(b, len r < 8 => a, else => a)),
        (output, quote!(count)),
        (grade, quote!(custom, a at host)),
    ] {
        assert!(pass(&physical, &structures, &invalid, advice).is_err());
    }
}

#[test]
fn evidence_advice_is_checked_replayable_metadata() {
    let (physical, structures, source) = program(quote!(out = store rows r;));
    let target = apply(cq_sequence_plan::rewrites::evidence, &physical, &structures, &source, quote!(evidence from authored("features/sequence-rewrites/tests/rewrites.rs fixture") {
        count R = 0;
        trusted_key R [a];
        foreign_key R [a] -> r [a];
        selectivity out = 0 / 1;
        bounds R.a = 42 ..= -42;
        sorted R [a];
    }));
    assert_eq!(target.sequence, source.sequence);
    assert_eq!(target.graded, source.graded);
    assert_eq!(target.evidence.count("R"), Some(0));
    assert_eq!(contract::check(&physical, &structures, &target).unwrap().evidence, target.evidence);
    assert_eq!(
        cq_sequence_plan::rewrites::record_evidence(&physical, &structures, &target, &target.evidence).unwrap(),
        target
    );
    let extended = apply(cq_sequence_plan::rewrites::evidence, &physical, &structures, &target, quote!(evidence from vo("original VO append fixture") { count R = 123; planning_key R [a]; }));
    assert_eq!(
        &extended.evidence.facts[..target.evidence.facts.len()],
        &target.evidence.facts
    );
    assert_eq!(extended.evidence.count("R"), Some(123));
    assert!(matches!(
        extended.evidence.facts.last().unwrap().source(),
        cq_lang_sequence::EvidenceSource::Vo { provenance }
            if provenance.value() == "original VO append fixture"
    ));
    assert!(
        cq_sequence_plan::rewrites::evidence(&physical, &structures, &source, quote!(evidence from authored("features/sequence-rewrites/tests/rewrites.rs fixture") { count absent = 1; }))
            .is_err()
    );
    assert!(
        cq_sequence_plan::rewrites::evidence(&physical, &structures, &source, quote!(evidence from authored("features/sequence-rewrites/tests/rewrites.rs fixture") { sorted R [absent]; }))
            .is_err()
    );
}
