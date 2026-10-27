//! Checked Round schedule selection. Use a direct `cq-datalog-schedule` dependency
//! and `round = [cq_datalog_schedule::schedule(by_head)]` in the Datalog pipeline.
//! `whole` groups each phase, `per_hole` separates holes, and `by_head` groups
//! each phase by output relation. `strata(0 { seed [0] step [0] [1] })`
//! replaces only the listed components; adjacent brackets inherit their phase.
//! Combine forms with commas: `by_head, grain(128), inline_below(0)`.
//! Global policy applies to all holes; `policy(0 { step 1 grain 64 })`
//! overrides only the named fields of individual holes, after wave selection.
//! `synchronous` or `relational` selects the step-phase progress mode
//! (WF-DL-Progress); otherwise the source's mode is kept. Relational progress
//! drops the round barrier between steps; seeds stay synchronous.

mod rewrite {
    use cq_lang_datalog_round as round;
    use proc_macro2::TokenStream;
    use round::schedule::{HolePolicy, Schedule, Wave};
    use syn::parse::{Parse, ParseStream};
    use syn::{Ident, LitInt, Token, braced, bracketed, parenthesized};

    enum WaveAdvice {
        Whole,
        PerHole,
        ByHead,
        Strata(Vec<(usize, Vec<Wave>)>),
    }

    impl Parse for WaveAdvice {
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
    struct Advice {
        waves: Option<WaveAdvice>,
        progress: Option<Ident>,
        grain: Option<usize>,
        inline_below: Option<usize>,
        policy: Vec<(usize, HolePolicy)>,
    }

    impl Parse for Advice {
        fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
            let mut advice = Self {
                waves: None,
                progress: None,
                grain: None,
                inline_below: None,
                policy: Vec::new(),
            };
            if input.is_empty() {
                return Err(input.error("expected schedule advice"));
            }
            while !input.is_empty() {
                let name: Ident = input.fork().parse()?;
                match name.to_string().as_str() {
                    "whole" | "per_hole" | "by_head" | "strata" => {
                        if advice.waves.is_some() {
                            return Err(input.error("wave advice listed more than once"));
                        }
                        advice.waves = Some(input.parse()?);
                    }
                    "synchronous" | "relational" => {
                        let name: Ident = input.parse()?;
                        if advice.progress.replace(name.clone()).is_some() {
                            return Err(syn::Error::new_spanned(
                                name,
                                "progress advice listed more than once",
                            ));
                        }
                    }
                    "grain" | "inline_below" => {
                        let name: Ident = input.parse()?;
                        let value;
                        parenthesized!(value in input);
                        let number: usize = value.parse::<LitInt>()?.base10_parse()?;
                        if !value.is_empty() {
                            return Err(value.error("expected one integer"));
                        }
                        let slot = if name == "grain" {
                            if number == 0 {
                                return Err(syn::Error::new_spanned(
                                    name,
                                    "grain must be positive",
                                ));
                            }
                            &mut advice.grain
                        } else {
                            &mut advice.inline_below
                        };
                        if slot.replace(number).is_some() {
                            return Err(syn::Error::new_spanned(
                                name,
                                "policy field listed more than once",
                            ));
                        }
                    }
                    "policy" => {
                        input.parse::<Ident>()?;
                        let entries;
                        parenthesized!(entries in input);
                        while !entries.is_empty() {
                            let component = entries.parse::<LitInt>()?.base10_parse()?;
                            let body;
                            braced!(body in entries);
                            while !body.is_empty() {
                                let phase: Ident = body.parse()?;
                                if phase != "seed" && phase != "step" {
                                    return Err(syn::Error::new_spanned(
                                        phase,
                                        "expected seed or step",
                                    ));
                                }
                                let index = body.parse::<LitInt>()?.base10_parse()?;
                                let mut policy = HolePolicy {
                                    phase,
                                    index,
                                    grain: None,
                                    inline_below: None,
                                };
                                while !body.is_empty() {
                                    let field: Ident = body.fork().parse()?;
                                    if field == "seed" || field == "step" {
                                        break;
                                    }
                                    let field: Ident = body.parse()?;
                                    let slot = match field.to_string().as_str() {
                                        "grain" => &mut policy.grain,
                                        "inline_below" => &mut policy.inline_below,
                                        _ => {
                                            return Err(syn::Error::new_spanned(
                                                field,
                                                "expected grain or inline_below",
                                            ));
                                        }
                                    };
                                    let value = body.parse::<LitInt>()?.base10_parse()?;
                                    if field == "grain" && value == 0 {
                                        return Err(syn::Error::new_spanned(
                                            field,
                                            "grain must be positive",
                                        ));
                                    }
                                    if slot.replace(value).is_some() {
                                        return Err(syn::Error::new_spanned(
                                            field,
                                            "policy field listed more than once",
                                        ));
                                    }
                                }
                                if advice.policy.iter().any(|(old, p)| {
                                    *old == component
                                        && p.phase == policy.phase
                                        && p.index == policy.index
                                }) {
                                    return Err(body.error("hole policy listed more than once"));
                                }
                                advice.policy.push((component, policy));
                            }
                            if entries.is_empty() {
                                break;
                            }
                            entries.parse::<Token![,]>()?;
                        }
                    }
                    _ => return Err(syn::Error::new_spanned(name, "unknown schedule advice")),
                }
                if input.is_empty() {
                    break;
                }
                input.parse::<Token![,]>()?;
            }
            Ok(advice)
        }
    }

    pub fn compile_with(source: &round::Program, args: TokenStream) -> syn::Result<round::Program> {
        let advice: Advice = syn::parse2(args)?;
        let analysis = cq_lang_datalog_storage::contract::check(&source.storage)?;
        let mut target = source.clone();
        target.schedule = match advice.waves {
            Some(WaveAdvice::Whole) => {
                Schedule::whole(&source.storage, &analysis, &source.context)?
            }
            Some(WaveAdvice::PerHole) => {
                Schedule::per_hole(&source.storage, &analysis, &source.context)?
            }
            Some(WaveAdvice::ByHead) => {
                Schedule::by_head(&source.storage, &analysis, &source.context)?
            }
            Some(WaveAdvice::Strata(strata)) => {
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
            None => source.schedule.clone(),
        };
        target.schedule.progress = advice
            .progress
            .unwrap_or_else(|| source.schedule.progress.clone());
        if advice.grain.is_some() || advice.inline_below.is_some() {
            for (component, stratum) in source.schedule.strata.iter().enumerate() {
                for hole in &stratum.reads {
                    let old = target.schedule.strata[component]
                        .policy
                        .iter()
                        .find(|p| p.phase == hole.phase && p.index == hole.index);
                    let policy = HolePolicy {
                        phase: hole.phase.clone(),
                        index: hole.index,
                        grain: advice.grain.or_else(|| old.and_then(|p| p.grain)),
                        inline_below: advice
                            .inline_below
                            .or_else(|| old.and_then(|p| p.inline_below)),
                    };
                    target.schedule = target.schedule.with_policy(component, policy);
                }
            }
        }
        for (component, mut policy) in advice.policy {
            let stratum = target.schedule.strata.get(component).ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "schedule component index out of range",
                )
            })?;
            if let Some(old) = stratum
                .policy
                .iter()
                .find(|p| p.phase == policy.phase && p.index == policy.index)
            {
                policy.grain = policy.grain.or(old.grain);
                policy.inline_below = policy.inline_below.or(old.inline_below);
            }
            target.schedule = target.schedule.with_policy(component, policy);
        }
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
