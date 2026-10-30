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
    /// An explicit occurrence/subatom order, checked by WF-VO and lookup-phase obligations.
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

/// Preserve authored atom/block selectors when normalization inserts clauses.
fn normalized_advice(
    source: &cq::Program,
    normal: &cq::Program,
    args: &PlanOptions,
) -> syn::Result<Option<PlanOptions>> {
    let selectors = args
        .order
        .iter()
        .flatten()
        .chain(args.reduce.iter().flat_map(|(a, b)| [a, b]))
        .chain(args.skip.iter().flat_map(|(a, b)| [a, b]));
    if !selectors
        .into_iter()
        .any(|selector| selector.occurrence.is_some())
    {
        return Ok(None);
    }
    let occurrences = |program: &cq::Program| {
        program
            .query
            .body
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                matches!(item, cq::BodyItem::Positive { .. } | cq::BodyItem::Block(_))
                    .then_some(index)
            })
            .collect::<Vec<_>>()
    };
    // Normalization preserves these occurrences and their relative order;
    // generated constraints and Rust bindings are not authored selectors.
    let mapping: std::collections::BTreeMap<_, _> = occurrences(source)
        .into_iter()
        .zip(occurrences(normal))
        .collect();
    let remap = |selector: &cq_order::OrderAtom| -> syn::Result<cq_order::OrderAtom> {
        let mut selector = selector.clone();
        if selector.occurrence.is_some() {
            let source_clause = selector.resolve(source)?;
            selector.occurrence = Some(mapping[&source_clause]);
        }
        Ok(selector)
    };
    let pairs = |pairs: &[(cq_order::OrderAtom, cq_order::OrderAtom)]| {
        pairs
            .iter()
            .map(|(a, b)| Ok((remap(a)?, remap(b)?)))
            .collect::<syn::Result<Vec<_>>>()
    };
    Ok(Some(PlanOptions {
        order: args
            .order
            .as_ref()
            .map(|order| order.iter().map(remap).collect())
            .transpose()?,
        no_reduce: args.no_reduce,
        reduce: pairs(&args.reduce)?,
        skip: pairs(&args.skip)?,
    }))
}

/// Normalize, order and reduce CQ using the shared production composition.
/// An explicit order overrides Prefix(Connected), not the reduction policy.
pub fn prefix_valuation(source: &cq::Program, args: &PlanOptions) -> syn::Result<vo::Program> {
    let normal = cq::normal::normalize(source)?;
    let remapped = normalized_advice(source, &normal, args)?;
    let args = remapped.as_ref().unwrap_or(args);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_advice_elaborates_lookup_and_expand_on_the_same_occurrence() {
        let source: cq::Program = syn::parse_quote! {
            struct Q;
            relation Seed(x:i32);
            relation R(x:i32,y:i32);
            answer(x,y) :- Seed(x), R(x,y);
        };
        let options: PlanOptions = syn::parse_quote!(no_reduce, order(Seed, R@1(x), R@1));
        let plan = compile_prefix(&source, &options).unwrap();
        let (lookup_index, lookup) = plan
            .plan
            .definitions
            .iter()
            .enumerate()
            .find_map(|(i, definition)| match &definition.operator {
                ra::Operator::Semijoin(join) => Some((i, join)),
                _ => None,
            })
            .expect("lookup is E-Check semijoin");
        let (expand_index, expand) = plan
            .plan
            .definitions
            .iter()
            .enumerate()
            .find_map(|(i, definition)| match &definition.operator {
                ra::Operator::NaturalJoin(join) if join.right == lookup.right => Some((i, join)),
                _ => None,
            })
            .expect("expansion joins the exact same occurrence relation");
        assert!(lookup_index < expand_index);
        assert_eq!(expand.left, plan.plan.definitions[lookup_index].result);
    }

    #[test]
    fn subatom_selectors_survive_normalization_inserted_clauses() {
        let source: cq::Program = syn::parse_quote! {
            struct Q;
            relation Seed(tag:i32,x:i32);
            relation R(x:i32,y:i32);
            answer(x,y) :- Seed({ wanted_tag },x), R(x,y);
        };
        let options: PlanOptions = syn::parse_quote!(no_reduce, order(Seed, R@1(x), R@1));
        let plan = prefix_valuation(&source, &options).unwrap();
        let occurrence = plan
            .cq
            .query
            .body
            .iter()
            .position(
                |item| matches!(item, cq::BodyItem::Positive { atom } if atom.relation == "R"),
            )
            .unwrap();
        assert_ne!(
            occurrence, 1,
            "normalization inserts Seed's closed-operand constraint"
        );
        let reads: Vec<_> = plan
            .valuation
            .steps
            .iter()
            .filter_map(|step| match step {
                vo::Step::Atom(atom, _) if atom.relation == "R" => {
                    Some((atom.clause(), atom.variables().count()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(reads, [(occurrence, 1), (occurrence, 2)]);
    }

    #[test]
    fn authored_selectors_survive_closed_atom_restrictions() {
        let source: cq::Program = syn::parse_quote! {
            struct Q;
            relation R(key:i32,value:CodeAlias);
            relation S(key:i32);
            answer(x) :- R(x, { first }), R(x, { second }),
                { S(x) } | { S(x) };
        };
        let normal = cq::normal::normalize(&source).unwrap();
        let args: PlanOptions = syn::parse_quote!(
            order(R@1,R@0,block@2), no_reduce,
            reduce(R@0 with R@1), skip(R@1 with R@0)
        );
        let mapped = normalized_advice(&source, &normal, &args).unwrap().unwrap();
        assert_eq!(
            mapped
                .order
                .as_ref()
                .unwrap()
                .iter()
                .map(|selector| selector.occurrence)
                .collect::<Vec<_>>(),
            [Some(2), Some(0), Some(4)]
        );
        assert_eq!(mapped.reduce[0].0.occurrence, Some(0));
        assert_eq!(mapped.reduce[0].1.occurrence, Some(2));
        assert_eq!(mapped.skip[0].0.occurrence, Some(2));
        assert_eq!(mapped.skip[0].1.occurrence, Some(0));
        let executable: PlanOptions = syn::parse_quote!(order(R @ 1, R @ 0, block @ 2), no_reduce);
        prefix_valuation(&source, &executable).unwrap();
        compile_prefix(&source, &executable).unwrap();
    }

    #[test]
    fn closed_disjunction_domains_preserve_literal_reduction_topology() {
        let mut reference = None;
        for (ty, first, second) in [
            ("String", "\"FRANCE\"", "\"GERMANY\""),
            ("CodeAlias", "france", "germany"),
        ] {
            let source: cq::Program = syn::parse_str(&format!(
                r#"
                struct Q;
                relation Lineitem(order_key:i64,supplier_key:i64,ship_date:Date);
                relation Supplier(supplier_key:i64,nation_key:i32);
                relation Orders(order_key:i64,customer_key:i64);
                relation Customer(customer_key:i64,nation_key:i32);
                relation Nation(nation_key:i32,name:{ty});
                answer(order_key,supplier_nation,customer_nation) :-
                    Lineitem(order_key,supplier_key,ship_date),
                    where ship_date >= start && ship_date <= end,
                    Supplier(supplier_key,supplier_nation),
                    Orders(order_key,customer_key), Customer(customer_key,customer_nation),
                    Nation(supplier_nation,supplier_name), Nation(customer_nation,customer_name),
                    where (supplier_name == {first} && customer_name == {second}) ||
                        (supplier_name == {second} && customer_name == {first});
            "#
            ))
            .unwrap();
            // This isolates operand representation from A-Closed atom normalization.
            assert_eq!(cq::normal::normalize(&source).unwrap(), source);
            let planned = compile_prefix(&source, &PlanOptions::default()).unwrap();
            assert_eq!(
                planned
                    .plan
                    .definitions
                    .iter()
                    .filter(|definition| matches!(definition.operator, ra::Operator::Semijoin(_)))
                    .count(),
                3
            );
            let mut topology = planned.plan.definitions;
            for definition in &mut topology {
                if let ra::Operator::Select(selection) = &mut definition.operator {
                    selection.condition = Default::default();
                }
            }
            if let Some(reference) = &reference {
                assert_eq!(
                    &topology, reference,
                    "only constant operand payloads and schema types may differ"
                );
            } else {
                reference = Some(topology);
            }
        }
    }

    #[test]
    fn fallback_aggregate_preserves_all_joined_prefix_origins() {
        let source: cq::Program = syn::parse_quote! {
            struct Q;
            relation Lineitem(id:i64,key:i64,value:i64,date:i32);
            relation Part(key:i64,kind:i32);
            relation Qualified(kind:i32);
            relation answer(promotional:i64,total:i64);
            answer(sum(promo),sum(revenue)) :-
                Lineitem(id,key,value,date),
                where date >= start && date < end,
                Part(key,kind),
                let revenue:i64 = *value,
                { Qualified(kind), let promo:i64 = *revenue }
                else { let promo:i64 = 0 };
        };
        assert_eq!(
            source
                .query
                .body
                .iter()
                .position(|item| matches!(item, cq::BodyItem::Block(_))),
            Some(5)
        );
        for options in [
            PlanOptions::default(),
            syn::parse_quote!(no_reduce),
            syn::parse_quote!(no_reduce, order(Lineitem, Part, block @ 5)),
        ] {
            compile_prefix(&source, &options).unwrap();
        }
    }
}
