use cq_stage::{Boundary, Configuration, Language};
use proc_macro2::TokenStream;
use syn::parse::ParseStream;
use syn::{Token, braced};

mod kw {
    syn::custom_keyword!(observe);
    syn::custom_keyword!(boundary);
    syn::custom_keyword!(stage);
}

pub(super) struct Observation {
    boundary: Boundary,
    receiver: syn::Path,
}

pub(super) fn parse(input: ParseStream<'_>) -> syn::Result<Option<Observation>> {
    if !input.peek(kw::observe) {
        return Ok(None);
    }
    input.parse::<kw::observe>()?;
    let options;
    braced!(options in input);
    options.parse::<kw::boundary>()?;
    options.parse::<Token![=]>()?;
    let boundary = options.parse()?;
    options.parse::<Token![;]>()?;
    options.parse::<kw::stage>()?;
    options.parse::<Token![=]>()?;
    let receiver = options.parse()?;
    options.parse::<Token![;]>()?;
    if !options.is_empty() {
        return Err(options.error("expected boundary and stage observer only"));
    }
    Ok(Some(Observation { boundary, receiver }))
}

impl Observation {
    pub(super) fn expand(
        self,
        config: Configuration,
        tokens: TokenStream,
    ) -> syn::Result<TokenStream> {
        if let Ok(sequence) = syn::parse2::<cq_lang_sequence::Program>(tokens.clone()) {
            let mut pipeline = super::sequence_pipeline(config)?;
            let cutoff = match self.boundary {
                Boundary::Sequence => {
                    sequence.check()?;
                    0
                }
                Boundary::Decompose => pipeline.passes.len() - 2,
                Boundary::Grade => pipeline.passes.len() - 1,
                Boundary::Rust => pipeline.passes.len(),
                _ => {
                    return Err(syn::Error::new_spanned(
                        self.receiver,
                        "boundary precedes complete Sequence input",
                    ));
                }
            };
            pipeline.passes.truncate(cutoff);
            pipeline.observer = Some(self.receiver);
            return pipeline.start(tokens);
        }
        if self.boundary == Boundary::Vo && config.plan.is_some() {
            return Err(syn::Error::new_spanned(
                self.receiver,
                "VO observation requires an omitted plan phase; custom planners expose their own internal boundaries",
            ));
        }
        let boundary = if self.boundary == Boundary::Vo {
            Boundary::RelationalPlan
        } else {
            self.boundary
        };
        let mut pipeline = config.resolve_observed(super::defaults(), boundary, self.receiver)?;
        match self.boundary {
            Boundary::Cq => pipeline
                .passes
                .push(syn::parse_quote!(::mini_linq::passes::observe_cq)),
            Boundary::Vo => {
                let planner = pipeline.passes.last_mut().expect("resolved planning phase");
                planner.path = syn::parse_quote!(::mini_linq::passes::prefix_valuation);
            }
            _ => {}
        }
        pipeline.start(tokens)
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    #[test]
    fn absent_and_custom_internal_boundaries_are_rejected() {
        for tokens in [
            quote! {
                observe { boundary = sequence; stage = ::core::stringify; }
                pipeline { print_input = structures; print = custom::terminal; }
                struct Q; relation R(x:i32); answer(x) :- R(x);
            },
            quote! {
                observe { boundary = vo; stage = ::core::stringify; }
                pipeline { plan = alias::planner; }
                struct Q; relation R(x:i32); answer(x) :- R(x);
            },
        ] {
            assert!(crate::expand_compile(tokens).is_err());
        }
    }
}
