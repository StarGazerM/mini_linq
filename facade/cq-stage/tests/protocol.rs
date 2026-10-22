use cq_stage::{Language, pass, rewrite};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};

#[derive(Clone)]
struct Number(syn::LitInt);
impl Parse for Number {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse().map(Self)
    }
}
impl ToTokens for Number {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.0.to_tokens(output);
    }
}
impl Language for Number {
    fn check(&self) -> syn::Result<()> {
        if self.0.base10_parse::<u32>()? == 0 {
            Err(syn::Error::new_spanned(&self.0, "zero is inadmissible"))
        } else {
            Ok(())
        }
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        if source.0 == target.0 {
            Ok(())
        } else {
            Err(syn::Error::new_spanned(&target.0, "value changed"))
        }
    }
}
#[test]
fn target_contract_runs_before_continuation() {
    let tokens = pass::<Number, Number>(quote!(pipeline { consumer }; 1), |_, _| {
        Ok(Number(syn::parse_quote!(0)))
    });
    assert!(tokens.to_string().contains("zero is inadmissible"));
    assert!(!tokens.to_string().contains("consumer !"));
}
#[test]
fn rewrite_correspondence_is_additional_to_the_contract() {
    let tokens = rewrite::<Number>(quote!(pipeline { consumer }; 1), |_, _| {
        Ok(Number(syn::parse_quote!(2)))
    });
    assert!(tokens.to_string().contains("value changed"));
}
#[test]
fn source_contract_precedes_the_transform() {
    let tokens = pass::<Number, Number>(quote!(pipeline { consumer }; 0), |_, _| {
        panic!("invalid source reached f")
    });
    assert!(tokens.to_string().contains("zero is inadmissible"));
}
#[test]
fn opaque_arguments_and_remaining_pipeline_are_forwarded() {
    let tokens = rewrite::<Number>(
        quote!(arguments { x + (y * z) }; pipeline { other(a ! { b }), final_pass }; 1),
        |source, args| {
            assert_eq!(args.to_string(), "x + (y * z)");
            Ok(source.clone())
        },
    );
    assert_eq!(
        tokens.to_string(),
        quote!(other! { arguments { a ! { b } }; pipeline { final_pass }; 1 }).to_string()
    );
}
#[test]
fn only_a_terminal_printer_can_end_the_pipeline() {
    let tokens = pass::<Number, syn::Expr>(quote!(pipeline {}; 1), |n, _| {
        syn::parse2(n.to_token_stream())
    });
    assert_eq!(tokens.to_string(), "1");
    let tokens = rewrite::<Number>(quote!(pipeline {}; 1), |n, _| Ok(n.clone()));
    assert!(tokens.to_string().contains("before its Rust printer"));
}
#[test]
fn named_grammar_is_generic_and_rejects_duplicates() {
    let config: cq_stage::Configuration =
        syn::parse2(quote!(pipeline { plan = a(x); physical = [b, c(y)]; surface = [d]; }))
            .unwrap();
    let defaults: cq_stage::Configuration =
        syn::parse2(quote!(pipeline { plan = w; ix = x; lower = y; select = z; represent = r; stage = s; print = p; }))
            .unwrap();
    let pipeline = config.resolve(defaults).unwrap();
    assert_eq!(
        pipeline.to_token_stream().to_string(),
        quote!(pipeline { d, a(x), x, y, b, c(y), z, r, s, p }).to_string()
    );
    assert!(
        syn::parse2::<cq_stage::Configuration>(quote!(pipeline { plan = a; plan = b; })).is_err()
    );
    assert!(syn::parse2::<cq_stage::Configuration>(quote!(pipeline { unknown = a; })).is_err());
}

#[test]
fn representation_and_stage_overrides_keep_other_defaults() {
    let config: cq_stage::Configuration = syn::parse2(quote!(pipeline {
        represent = positional(positions);
        stage = chunked(batch(64));
    }))
    .unwrap();
    let defaults: cq_stage::Configuration = syn::parse2(quote!(pipeline {
        plan = p; ix = i; lower = l; select = s;
        represent = r; stage = b; print = out;
    }))
    .unwrap();
    assert_eq!(
        config
            .resolve(defaults)
            .unwrap()
            .to_token_stream()
            .to_string(),
        quote!(pipeline { p, i, l, s, positional(positions), chunked(batch(64)), out }).to_string()
    );
}

#[test]
fn sequence_phases_are_ordered_and_require_a_complete_target() {
    let defaults: cq_stage::Configuration = syn::parse_quote!(pipeline {
        plan = p; ix = i; lower = l; select = s;
        represent = r; stage = b; print = print_pull;
    });
    let config: cq_stage::Configuration = syn::parse_quote!(pipeline {
        surface = [surface];
        physical = [physical];
        structures = [structures];
        sequence = sequence;
        decompose = [tile(32), store];
        grade = sequential;
        print = translate_pull;
    });
    assert_eq!(
        config
            .resolve(defaults.clone())
            .unwrap()
            .to_token_stream()
            .to_string(),
        quote!(pipeline {
            surface, p, i, l, physical, s, structures, r, b,
            sequence, tile(32), store, sequential, translate_pull
        })
        .to_string()
    );
    for invalid in [
        quote!(sequence = sequence;),
        quote!(grade = sequential;),
        quote!(print = translate_pull;),
        quote!(sequence = sequence; grade = sequential;),
        quote!(sequence = sequence; print = translate_pull;),
        quote!(grade = sequential; print = translate_pull;),
        quote!(sequence = sequence; grade = sequential; print = print_pull;),
        quote!(sequence = sequence; grade = sequential; print = own_push::push;),
        quote!(decompose = [tile(32)];),
    ] {
        let config: cq_stage::Configuration = syn::parse2(quote!(pipeline { #invalid })).unwrap();
        assert!(config.resolve(defaults.clone()).is_err(), "{invalid}");
    }
    let custom: cq_stage::Configuration = syn::parse_quote!(pipeline {
        sequence = lower_custom; grade = target_custom; print = custom::translate;
    });
    assert!(custom.resolve(defaults.clone()).is_ok());
    let mut sequence_defaults = defaults;
    sequence_defaults.sequence = Some(syn::parse_quote!(sequence));
    sequence_defaults.grade = Some(syn::parse_quote!(sequential));
    sequence_defaults.print = Some(syn::parse_quote!(translate_pull));
    assert!(
        cq_stage::Configuration::default()
            .resolve(sequence_defaults)
            .is_ok()
    );
}
