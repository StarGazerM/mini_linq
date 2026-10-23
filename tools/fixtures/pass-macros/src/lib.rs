//! External test passes exercising only public core APIs.
use cq_lang_physical as physical;
use cq_lang_structures::Structure;
use proc_macro::TokenStream;

fn planned(input: TokenStream, options: cq_order::Options, reductions: bool) -> TokenStream {
    cq_stage::pass::<cq_lang_cq::Program, cq_lang_ra::Program>(input.into(), |source, args| {
        if !args.is_empty() {
            return Err(syn::Error::new_spanned(
                args,
                "unexpected planner arguments",
            ));
        }
        let normal = cq_lang_cq::normal::normalize(source)?;
        let mut plan = cq_order::compile(&normal, &options)?;
        cq_order::check(&normal, &plan, &options)?;
        if reductions {
            let options = cq_reduce::Options {
                semijoins: if cq_lang_cq::hypergraph::Analysis::of(&normal).acyclic {
                    cq_reduce::Reductions::Tree
                } else {
                    cq_reduce::Reductions::None
                },
            };
            let reduced = cq_reduce::compile(&plan, &options)?;
            cq_reduce::check(&plan, &reduced, &options)?;
            plan = reduced;
        }
        cq_elaborate::compile(&plan)
    })
    .into()
}
macro_rules! planner {
    ($name:ident, $heuristic:expr, $split:ident, $reduce:expr) => {
        #[proc_macro]
        pub fn $name(input: TokenStream) -> TokenStream {
            planned(
                input,
                cq_order::Options {
                    heuristic: $heuristic,
                    split: cq_order::Split::$split,
                },
                $reduce,
            )
        }
    };
}
planner!(
    plan_generic,
    cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
    Fine,
    false
);
planner!(plan_hypergraph, cq_order::Heuristic::Written, Coarse, true);
planner!(
    plan_prefix_connected,
    cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
    Coarse,
    false
);
planner!(
    plan_prefix_connected_fine,
    cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
    Fine,
    false
);
planner!(
    plan_prefix_greedy,
    cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
    Coarse,
    false
);
planner!(
    plan_prefix_greedy_fine,
    cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
    Fine,
    false
);
planner!(
    plan_elimination,
    cq_order::Heuristic::Elimination,
    Coarse,
    false
);
planner!(
    plan_elimination_fine,
    cq_order::Heuristic::Elimination,
    Fine,
    false
);

#[proc_macro]
pub fn merged(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, _| {
        cq_testkit::rewrites::merged(source)
    })
    .into()
}
#[proc_macro]
pub fn canonical_projects(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, _| {
        cq_testkit::rewrites::canonical_projects(source)
    })
    .into()
}
#[proc_macro]
pub fn smallest(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, _| {
        cq_testkit::rewrites::smallest(source)
    })
    .into()
}
#[proc_macro]
pub fn lazy_anti(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, _| {
        let mut target = source.clone();
        for definition in &mut target.program.definitions {
            if let physical::Operator::Anti(_, keyed) = &mut definition.operator {
                keyed.choice = Some(physical::Choice::Lazy(Default::default()));
            }
        }
        Ok(target)
    })
    .into()
}
#[proc_macro]
pub fn ordered_dedup(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, _| {
        cq_testkit::rewrites::ordered_dedup(source)
    })
    .into()
}
#[proc_macro]
pub fn ordered_indexes(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, _| {
        let mut target = source.clone();
        for entry in &mut target.structures.entries {
            if let Structure::Built { keyed, .. } = &mut entry.structure {
                keyed.ty = syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>);
            }
        }
        Ok(target)
    })
    .into()
}
/// Surface example: expands identity!{ R(a) } as a normal CQ clause.
#[proc_macro]
pub fn expand_identity(input: TokenStream) -> TokenStream {
    cq_stage::pass::<proc_macro2::TokenStream, proc_macro2::TokenStream>(
        input.into(),
        |source, args| {
            if args.to_string() != "expected" {
                return Err(syn::Error::new_spanned(
                    args,
                    "opaque arguments were not forwarded",
                ));
            }
            let tokens: Vec<_> = source.clone().into_iter().collect();
            let mut output = proc_macro2::TokenStream::new();
            let mut i = 0;
            while i < tokens.len() {
                if matches!(&tokens[i], proc_macro2::TokenTree::Ident(id) if id == "identity")
                    && tokens.get(i + 1).is_some_and(
                        |t| matches!(t, proc_macro2::TokenTree::Punct(p) if p.as_char() == '!'),
                    )
                {
                    if let Some(proc_macro2::TokenTree::Group(group)) = tokens.get(i + 2) {
                        output.extend(group.stream());
                        i += 3;
                        continue;
                    }
                }
                output.extend([tokens[i].clone()]);
                i += 1;
            }
            Ok(output)
        },
    )
    .into()
}
#[proc_macro]
pub fn custom_indexes(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, _| {
        let mut target = source.clone();
        for entry in &mut target.structures.entries {
            if let Structure::Built { keyed, .. } = &mut entry.structure {
                keyed.ty = syn::parse_quote!(SortedIndex<_, _>);
            }
        }
        Ok(target)
    })
    .into()
}
#[proc_macro]
pub fn identity_structures(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, _| Ok(source.clone()))
        .into()
}

/// A real alternative policy: prefer reserve when no output protocol is authored.
/// Every other decision is left to the shared rule-based planner.
struct ReservePlanner;

impl cq_sequence_plan::Planner for ReservePlanner {
    fn plan(
        &self,
        source: &cq_lang_sequence::Program,
        machine: &cq_lang_sequence::Machine,
        catalog: &cq_lang_sequence::Catalog,
        evidence: &cq_lang_sequence::Evidence,
    ) -> syn::Result<cq_sequence_plan::Planned> {
        if source.sequence.output.is_some() {
            return cq_sequence_plan::Planner::plan(
                &cq_sequence_plan::RuleBased,
                source,
                machine,
                catalog,
                evidence,
            );
        }
        let advice = cq_sequence_plan::Advice::Output(cq_lang_sequence::OutputProtocol::Reserve);
        let advised = advice.apply(source)?;
        let mut planned = cq_sequence_plan::Planner::plan(
            &cq_sequence_plan::RuleBased,
            &advised,
            machine,
            catalog,
            evidence,
        )?;
        planned.trace.insert(0, advice);
        Ok(planned)
    }
}

/// Ordinary external checked pass, selected by the facade's named plan phase.
#[proc_macro]
pub fn plan_reserve(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, args| {
        if args.to_string() != "expected" {
            return Err(syn::Error::new_spanned(
                args,
                "planner arguments were not forwarded",
            ));
        }
        let machine = cq_sequence_pull::machine();
        let planned = cq_sequence_plan::Planner::plan(
            &ReservePlanner,
            source,
            &machine,
            &cq_sequence_pull::catalog(),
            &source.evidence,
        )?;
        let mut replayed = source.clone();
        for advice in &planned.trace {
            let next = advice.apply(&replayed)?;
            cq_lang_sequence::check_rewrite(&replayed, &next)?;
            replayed = next;
        }
        if replayed != planned.program {
            return Err(syn::Error::new_spanned(
                source,
                "planner trace did not replay",
            ));
        }
        cq_lang_sequence::check_grading(&planned.program, &machine)?;
        Ok(planned.program)
    })
    .into()
}
