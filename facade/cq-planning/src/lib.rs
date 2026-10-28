//! Shared parser and checked Plan-Prefix composition for all compiler drivers.
#![forbid(unsafe_code)]

use cq_lang_cq as cq;
use cq_lang_ra as ra;
use cq_lang_vo as vo;
use cq_order::{Heuristic, Options, Ranking, Split};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token};

/// Arguments shared by facade, build-time and diagnostic Plan-Prefix callers.
#[derive(Default)]
pub struct PlanOptions {
    /// An explicit relation order, checked by the ordinary ordering pass.
    pub order: Option<Vec<cq_order::OrderAtom>>,
    pub no_reduce: bool,
    /// `(target, support)` atom selectors to add, in advice order.
    pub reduce: Vec<(cq_order::OrderAtom, cq_order::OrderAtom)>,
    /// `(target, support)` atom selectors whose Masks reduction is dropped.
    pub skip: Vec<(cq_order::OrderAtom, cq_order::OrderAtom)>,
}

impl Parse for PlanOptions {
    /// Parse comma-separated order, no_reduce, reduce and skip options.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut options = Self::default();
        while !input.is_empty() {
            let option: Ident = input.parse()?;
            if option == "order" && options.order.is_none() {
                let content;
                syn::parenthesized!(content in input);
                options.order = Some(
                    content
                        .parse_terminated(cq_order::OrderAtom::parse, Token![,])?
                        .into_iter()
                        .collect(),
                );
            } else if option == "no_reduce" && !options.no_reduce {
                options.no_reduce = true;
            } else if option == "reduce" || option == "skip" {
                let content;
                syn::parenthesized!(content in input);
                let target: cq_order::OrderAtom = content.parse()?;
                let with: Ident = content.parse()?;
                if with != "with" {
                    return Err(syn::Error::new_spanned(with, "expected `with`"));
                }
                let support: cq_order::OrderAtom = content.parse()?;
                if !content.is_empty() {
                    return Err(content.error("expected (Target with Support)"));
                }
                if option == "reduce" {
                    options.reduce.push((target, support));
                } else {
                    options.skip.push((target, support));
                }
            } else {
                return Err(syn::Error::new_spanned(
                    option,
                    "expected order(Relation, ...), no_reduce, or reduce/skip(Target with Support)",
                ));
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
                if input.is_empty() {
                    return Err(input.error("trailing comma after plan options"));
                }
            }
        }
        Ok(options)
    }
}

/// Normalize, order and reduce CQ using the shared production composition.
/// An explicit order overrides Prefix(Connected), not the reduction policy.
pub fn prefix_valuation(source: &cq::Program, args: &PlanOptions) -> syn::Result<vo::Program> {
    let normal = cq::normal::normalize(source)?;
    let ordering = Options {
        heuristic: Heuristic::Prefix(Ranking::Connected),
        split: Split::Coarse,
    };
    let plan = if let Some(relations) = &args.order {
        cq_order::compile_explicit(&normal, relations)?
    } else {
        let plan = cq_order::compile(&normal, &ordering)?;
        cq_order::check(&normal, &plan, &ordering)?;
        plan
    };
    let options = cq_reduce::Options {
        semijoins: if args.no_reduce {
            cq_reduce::Reductions::None
        } else {
            cq_reduce::Reductions::Masks
        },
    };
    let mut reduced = cq_reduce::compile(&plan, &options)?;
    cq_reduce::check(&plan, &reduced, &options)?;
    advise_prefix(&mut reduced, args)?;
    Ok(reduced)
}

/// Formal rule R-Advice: Add admissible top-level reductions before the stream; skip only existing Masks reductions.
fn advise_prefix(reduced: &mut vo::Program, args: &PlanOptions) -> syn::Result<()> {
    if args.reduce.is_empty() && args.skip.is_empty() {
        return Ok(());
    }
    let analysis = vo::contract::check(reduced)?;
    let graph = cq::hypergraph::Analysis::of(&reduced.cq);
    let pair = |target: &cq_order::OrderAtom,
                support: &cq_order::OrderAtom|
     -> syn::Result<(usize, usize)> {
        let (t, s) = (
            target.positive_clause(&reduced.cq)?,
            support.positive_clause(&reduced.cq)?,
        );
        if t == s || graph.shared(t, s).is_empty() {
            return Err(syn::Error::new_spanned(
                &support.relation,
                "reduce advice needs two atoms sharing a variable",
            ));
        }
        Ok((t, s))
    };
    let mut skipped = Vec::new();
    for (target, support) in &args.skip {
        let (t, s) = pair(target, support)?;
        let step = cq_reduce::semijoin(&analysis, &graph, t, s);
        if !reduced.valuation.steps.contains(&step) {
            return Err(syn::Error::new_spanned(
                &target.relation,
                "skip advice names no Masks reduction",
            ));
        }
        skipped.push(step);
    }
    reduced
        .valuation
        .steps
        .retain(|step| !skipped.contains(step));
    let mut steps = Vec::new();
    for (target, support) in &args.reduce {
        let (t, s) = pair(target, support)?;
        steps.push(cq_reduce::semijoin(&analysis, &graph, t, s));
    }
    let first = reduced
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, vo::Step::Atom(..) | vo::Step::Block(..)))
        .unwrap_or(reduced.valuation.steps.len());
    reduced.valuation.steps.splice(first..first, steps);
    vo::contract::check(reduced)?;
    Ok(())
}

/// CQ → RA′ through the same checked composition used by the facade.
/// Formal rule Plan-Prefix: Normalize, order Prefix(Connected)/Coarse or explicit order, reduce by Masks unless disabled, apply advice, and elaborate.
pub fn compile_prefix(source: &cq::Program, options: &PlanOptions) -> syn::Result<ra::Program> {
    let reduced = prefix_valuation(source, options)?;
    let target = cq_elaborate::compile(&reduced)?;
    cq_elaborate::check(&reduced, &target)?;
    Ok(target)
}
