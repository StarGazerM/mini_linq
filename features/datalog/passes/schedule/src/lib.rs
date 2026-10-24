//! Checked Round schedule selection. Use a direct `cq-datalog-schedule` dependency
//! and `round = [cq_datalog_schedule::schedule(by_head)]` in the Datalog pipeline.
//! `whole` groups each phase, `per_hole` separates holes, and `by_head` groups
//! each phase by output relation. `strata(0 { seed [0] step [0] [1] })`
//! replaces only the listed components; adjacent brackets inherit their phase.
//! All waves read the committed snapshot and preserve synchronous progress.

mod rewrite {
    use cq_lang_datalog_round as round;
    use proc_macro2::TokenStream;
    use round::schedule::{Schedule, Wave};
    use syn::parse::{Parse, ParseStream};
    use syn::{Ident, LitInt, Token, braced, bracketed, parenthesized};

    enum Advice {
        Whole,
        PerHole,
        ByHead,
        Strata(Vec<(usize, Vec<Wave>)>),
    }

    impl Parse for Advice {
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            let name: Ident = input.parse()?;
            match name.to_string().as_str() {
                "whole" => Ok(Self::Whole),
                "per_hole" => Ok(Self::PerHole),
                "by_head" => Ok(Self::ByHead),
                "strata" => {
                    let entries;
                    parenthesized!(entries in input);
                    let mut strata = Vec::new();
                    while !entries.is_empty() {
                        let index: LitInt = entries.parse()?;
                        let index = index.base10_parse()?;
                        if strata.iter().any(|(old, _)| *old == index) {
                            return Err(entries.error("component listed more than once"));
                        }
                        let body;
                        braced!(body in entries);
                        let mut waves = Vec::new();
                        while !body.is_empty() {
                            let phase: Ident = body.parse()?;
                            if phase != "seed" && phase != "step" {
                                return Err(syn::Error::new_spanned(
                                    phase,
                                    "expected seed or step",
                                ));
                            }
                            loop {
                                let holes;
                                bracketed!(holes in body);
                                let mut indices = Vec::new();
                                while !holes.is_empty() {
                                    indices.push(holes.parse::<LitInt>()?.base10_parse()?);
                                }
                                waves.push(Wave {
                                    phase: phase.clone(),
                                    holes: indices,
                                });
                                if !body.peek(syn::token::Bracket) {
                                    break;
                                }
                            }
                        }
                        strata.push((index, waves));
                        if entries.is_empty() {
                            break;
                        }
                        entries.parse::<Token![,]>()?;
                    }
                    Ok(Self::Strata(strata))
                }
                _ => Err(syn::Error::new_spanned(
                    name,
                    "expected whole, per_hole, by_head, or strata",
                )),
            }
        }
    }

    pub fn compile_with(source: &round::Program, args: TokenStream) -> syn::Result<round::Program> {
        let advice: Advice = syn::parse2(args)?;
        let analysis = cq_lang_datalog_storage::contract::check(&source.storage)?;
        let mut target = source.clone();
        target.schedule = match advice {
            Advice::Whole => Schedule::whole(&source.storage, &analysis, &source.context)?,
            Advice::PerHole => Schedule::per_hole(&source.storage, &analysis, &source.context)?,
            Advice::ByHead => Schedule::by_head(&source.storage, &analysis, &source.context)?,
            Advice::Strata(strata) => {
                let mut schedule = source.schedule.clone();
                for (index, waves) in strata {
                    if index >= schedule.strata.len() {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            "schedule component index out of range",
                        ));
                    }
                    schedule = schedule.with_stratum_waves(index, waves, &source.context);
                }
                schedule
            }
        };
        check(source, &target)?;
        Ok(target)
    }

    /// Formal rule CK-DL-Schedule:
    /// Both rounds are well formed and erasure differs only in its schedule.
    pub fn check(source: &round::Program, target: &round::Program) -> syn::Result<()> {
        round::contract::check(source)?;
        round::contract::check(target)?;
        round::check_rewrite(source, target)?;
        let source = round::erase(source)?;
        let mut target = round::erase(target)?;
        target.schedule = source.schedule.clone();
        if target != source {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "schedule rewrite changed non-schedule data",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests;
}

/// Select a checked evaluation wave partition at the Round boundary.
#[proc_macro]
pub fn schedule(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(input.into(), rewrite::compile_with).into()
}
