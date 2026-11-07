use cq_lang_sequence::{Program, check_rewrite, contract};
use cq_sequence_plan::rewrites::{choose, grade, output, partition, segment, store_level, tile};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use cq_lang_sequence::structures::{Program as Structures, physical::Program as Physical};

type Stack = cq_stage::Sequence<Physical, Structures, Program>;

macro_rules! capture {
    (arguments {}; pipeline {} $(planning_tables { $($tables:tt)* })?; realization {
        physical { $($physical:tt)* }
        structures { $($structures:tt)* }
        sequence { $($sequence:tt)* }
    }) => { stringify!($($sequence)*) };
    (pipeline {} $(planning_tables { $($tables:tt)* })?; realization {
        physical { $($physical:tt)* }
        structures { $($structures:tt)* }
        sequence { $($sequence:tt)* }
    }) => { stringify!($($sequence)*) };
    (arguments {}; pipeline {} $(planning_tables { $($tables:tt)* })?; $($program:tt)*) => { stringify!($($program)*) };
    (pipeline {} $(planning_tables { $($tables:tt)* })?; $($program:tt)*) => { stringify!($($program)*) };
}

macro_rules! apply_macro {
    ($($pass:ident)::+, $($advice:tt)*) => {
        $($pass)::+! {
            arguments { $($advice)* };
            pipeline { capture };
            physical {
                relation R(a:i32);
                r = let canon rename R {a -> a};
                return r as answer(a);
            }
            structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; } r host scan = @reader as Reader []; r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows; }
            sequence {
                a = rows r;
                b = flatten tile 8 a;
                c = canon a;
                d = store capacity 64 a;
            }
        }
    };
}

fn source() -> Stack {
    syn::parse2(quote! {
        physical {
            relation R(a:i32);
            r = let canon rename R {a -> a};
            return r as answer(a);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; } r host scan = @reader as Reader []; r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows; }
        sequence {
            a = rows r;
            b = flatten tile 8 a;
            c = canon a;
            d = store capacity 64 a;
        }
    })
    .unwrap()
}

type Rewrite = fn(&Physical, &Structures, &Program, TokenStream) -> syn::Result<Program>;

#[test]
fn every_registered_macro_applies_and_round_trips() {
    let cases = [
        (
            apply_macro!(cq_sequence_rewrites::store_level, b),
            store_level as Rewrite,
            quote!(b),
        ),
        (
            apply_macro!(cq_sequence_rewrites::tile, b, 4, items 2),
            tile,
            quote!(b, 4, items 2),
        ),
        (
            apply_macro!(cq_sequence_rewrites::partition, d, 4, items 2),
            partition,
            quote!(d, 4, items 2),
        ),
        (
            apply_macro!(cq_sequence_rewrites::segment, b, 4, items 2),
            segment,
            quote!(b, 4, items 2),
        ),
        (
            apply_macro!(cq_sequence_rewrites::choose, c, len r < 32 => a, else => b),
            choose,
            quote!(c, len r < 32 => a, else => b),
        ),
        (
            apply_macro!(cq_sequence_rewrites::output, bounded),
            output,
            quote!(bounded),
        ),
        (
            apply_macro!(cq_sequence_rewrites::output, reserve_previous),
            output,
            quote!(reserve_previous),
        ),
        (
            apply_macro!(cq_sequence_rewrites::grade, custom, a at host, b at device.block),
            grade,
            quote!(custom, a at host, b at device.block),
        ),
    ];
    let source = source();
    for (captured, pass, advice) in cases {
        let target: Program = syn::parse_str(captured).unwrap();
        assert_eq!(target, pass(&source.physical, &source.structures, &source.sequence, advice).unwrap());
        contract::check(&source.physical, &source.structures, &target).unwrap();
        check_rewrite(&source.physical, &source.structures, &source.sequence, &source.physical, &source.structures, &target).unwrap();
        assert_eq!(target, syn::parse2(target.to_token_stream()).unwrap());
    }
}

#[test]
fn rejected_advice_stops_each_macro_before_continuation() {
    let source = source();
    for (pass, advice) in [
        (store_level as Rewrite, quote!(a)),
        (tile, quote!(a, 0)),
        (partition, quote!(d, 0)),
        (segment, quote!(a, 4)),
        (choose, quote!(b, len r < 4 => a, else => missing)),
        (output, quote!(invalid)),
        (grade, quote!(custom, missing at host)),
    ] {
        let result = cq_stage::Sequence::<cq_lang_sequence::structures::physical::Program, cq_lang_sequence::structures::Program, cq_lang_sequence::Program>::rewrite(
            quote! {
                arguments { #advice }; pipeline { forbidden_continuation }; #source
            },
            |physical, structures, source, arguments| {
                let target = pass(physical, structures, source, arguments)?;
                Ok((physical.clone(), structures.clone(), target))
            },
        )
        .to_string();
        assert!(result.contains("compile_error"), "{result}");
        assert!(!result.contains("forbidden_continuation"), "{result}");
    }
}

#[test]
fn registered_macros_chain_and_forward_advice() {
    let captured = cq_sequence_rewrites::store_level! {
        arguments { b };
        pipeline {
            cq_sequence_rewrites::tile(b, 4),
            cq_sequence_rewrites::choose(c, len r < 32 => a, else => b),
            cq_sequence_rewrites::output(reserve),
            cq_sequence_rewrites::grade(custom, a at host, b at host, c at host, d at host),
            capture
        };
        physical {
            relation R(a:i32);
            r = let canon rename R {a -> a};
            return r as answer(a);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; } r host scan = @reader as Reader []; r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows; }
        sequence {
            a = rows r;
            b = flatten tile 8 a;
            c = canon a;
            d = store capacity 64 a;
        }
    };
    let source = source();
    let expected = store_level(&source.physical, &source.structures, &source.sequence, quote!(b)).unwrap();
    let expected = tile(&source.physical, &source.structures, &expected, quote!(b, 4)).unwrap();
    let expected = choose(&source.physical, &source.structures, &expected, quote!(c, len r < 32 => a, else => b)).unwrap();
    let expected = output(&source.physical, &source.structures, &expected, quote!(reserve)).unwrap();
    let expected = grade(
        &source.physical,
        &source.structures,
        &expected,
        quote!(custom, a at host, b at host, c at host, d at host),
    )
    .unwrap();
    assert_eq!(syn::parse_str::<Program>(captured).unwrap(), expected);
}
