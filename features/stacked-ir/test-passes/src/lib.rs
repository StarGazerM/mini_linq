//! Feature-owned passes reproducing the ten facade physical_query scenarios.
//! Only public core APIs are used. Every choice is recorded in the target IR;
//! protocol rewrite checks validate Physical and Structures correspondence.
//! The stacked consumer forwards union/anti and occurrence paths through its wrapper; canonicality analysis preserves anti's left facts and does not assume union arms are unique.
//!
//! Core blocks denote Set: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust is opaque, pure and total; placement follows free-variable binders within
//! each branch, not written order. Fallback keeps its relational anti steps.
//! Relational constraints use `where`; Rust `||` is not a relational operator.
//! Only differing literals on one interface column prove union arms disjoint;
//! no data keys or constraints are assumed.
//!
use cq_lang_physical as physical;
use cq_lang_structures::Structure;
use proc_macro::TokenStream;

fn no_arguments(args: proc_macro2::TokenStream) -> syn::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            args,
            "test passes accept no arguments",
        ))
    }
}

fn planned(input: TokenStream, options: cq_order::Options, reductions: bool) -> TokenStream {
    cq_stage::pass::<cq_lang_cq::Program, cq_lang_ra::Program>(input.into(), |source, args| {
        no_arguments(args)?;
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
        let target = cq_elaborate::compile(&plan)?;
        cq_elaborate::check(&plan, &target)?;
        Ok(target)
    })
    .into()
}
#[proc_macro]
pub fn plan_generic(input: TokenStream) -> TokenStream {
    planned(
        input,
        cq_order::Options {
            heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
            split: cq_order::Split::Fine,
        },
        false,
    )
}
#[proc_macro]
pub fn plan_hypergraph(input: TokenStream) -> TokenStream {
    planned(
        input,
        cq_order::Options {
            heuristic: cq_order::Heuristic::Written,
            split: cq_order::Split::Coarse,
        },
        true,
    )
}

#[proc_macro]
pub fn merged(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_testkit::rewrites::merged(source)
    })
    .into()
}
#[proc_macro]
pub fn canonical_projects(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_testkit::rewrites::canonical_projects(source)
    })
    .into()
}
#[proc_macro]
pub fn smallest(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<physical::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_testkit::rewrites::smallest(source)
    })
    .into()
}
#[proc_macro]
pub fn ordered_dedup(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_testkit::rewrites::ordered_dedup(source)
    })
    .into()
}
#[proc_macro]
pub fn custom_indexes(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), |source, args| {
        no_arguments(args)?;
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
