//! Proc-macro ABI adapters for the public checked pipeline.
use proc_macro::TokenStream;
use syn::parse::{Parse, ParseStream};

mod load;
mod workload;

struct Source {
    config: cq_stage::Configuration,
    tokens: proc_macro2::TokenStream,
}
impl Parse for Source {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let config = input.parse()?;
        if input.peek(syn::Token![;]) {
            input.parse::<syn::Token![;]>()?;
        }
        Ok(Self {
            config,
            tokens: input.parse()?,
        })
    }
}
fn defaults() -> cq_stage::Configuration {
    syn::parse_quote! { pipeline {
        plan = ::mini_linq::passes::plan;
        ix = ::mini_linq::passes::index_demands;
        lower = ::mini_linq::passes::lower;
        select = ::mini_linq::passes::select;
        print = ::mini_linq::passes::print_pull;
    } }
}

/// Compile CQ with a named pipeline; omitted phases use the core defaults.
#[proc_macro]
pub fn compile(input: TokenStream) -> TokenStream {
    (|| {
        let source: Source = syn::parse(input)?;
        source.config.resolve(defaults())?.start(source.tokens)
    })()
    .unwrap_or_else(syn::Error::into_compile_error)
    .into()
}

/// Compile CQ with the deterministic default pipeline.
/// Formal rule ST-Entry: Start CQ surface expansion and the selected checked pipeline.
#[proc_macro]
pub fn mini_linq(input: TokenStream) -> TokenStream {
    defaults()
        .resolve(cq_stage::Configuration::default())
        .and_then(|pipeline| pipeline.start(input.into()))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn no_arguments(arguments: proc_macro2::TokenStream) -> syn::Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            arguments,
            "this default pass accepts no arguments",
        ))
    }
}

#[derive(Default)]
struct PlanOptions {
    no_reduce: bool,
}
impl Parse for PlanOptions {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self::default());
        }
        let option: syn::Ident = input.parse()?;
        if option != "no_reduce" {
            return Err(syn::Error::new_spanned(option, "expected no_reduce"));
        }
        Ok(Self { no_reduce: true })
    }
}

/// CQ → RA′ with static filtered-connected ordering and checked reductions.
/// `plan(no_reduce)` disables reductions, not static ordering.
/// Formal rule Plan-Prefix: Normalize, order Filtered/Coarse, optionally reduce and absorb, then record static build orientation before IX.
#[proc_macro]
pub fn plan(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_cq::Program, cq_lang_ra::Program>(input.into(), |source, args| {
        let args: PlanOptions = syn::parse2(args)?;
        let normal = cq_lang_cq::normal::normalize(source)?;
        let ordering = cq_order::Options {
            heuristic: cq_order::Heuristic::Filtered,
            split: cq_order::Split::Coarse,
        };
        let plan = cq_order::compile(&normal, &ordering)?;
        cq_order::check(&normal, &plan, &ordering)?;
        let options = cq_reduce::Options {
            guards: cq_reduce::Pushdown::None,
            semijoins: if args.no_reduce {
                cq_reduce::Reductions::None
            } else {
                cq_reduce::Reductions::Hypergraph
            },
        };
        let reduced = cq_reduce::compile(&plan, &options)?;
        cq_reduce::check(&plan, &reduced, &options)?;
        let target = cq_elaborate::compile(&reduced)?;
        cq_elaborate::check(&reduced, &target)?;
        let target = if args.no_reduce {
            target
        } else {
            cq_elaborate::simplify_reductions(&target)?
        };
        cq_physical_lowering::orient(&target)
    })
    .into()
}

/// RA′ → IX exact demand analysis.
#[proc_macro]
pub fn index_demands(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_ra::Program, cq_lang_ix::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_index_policy::compile(source)
    })
    .into()
}

/// IX → Physical with probe/keep/first/eager defaults.
#[proc_macro]
pub fn lower(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_ix::Program, cq_lang_physical::Program>(
        input.into(),
        |source, args| {
            no_arguments(args)?;
            cq_physical_lowering::compile(source)
        },
    )
    .into()
}

/// Physical → Structures with concrete default Rust types.
/// Optional `positional(H, ...)` arguments assert the default typed-column
/// capability for those hosts; unspecified hosts retain their original path.
#[proc_macro]
pub fn select(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_physical::Program, cq_lang_structures::Program>(
        input.into(),
        cq_structure_selection::compile_with,
    )
    .into()
}

/// Structures → final Rust pull expression.
/// Formal rule ST-Terminal: Print checked Structures with no remaining continuation.
#[proc_macro]
pub fn print_pull(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_physical_realization::compile(source)
    })
    .into()
}

/// Generate typed source readers from relation declarations or a CQ callback.
#[proc_macro]
pub fn relation_loaders(input: TokenStream) -> TokenStream {
    load::expand(input)
}

/// Capture authored CQ and export a callback invoking a public consumer macro.
#[proc_macro]
pub fn workload_query(input: TokenStream) -> TokenStream {
    workload::expand(input)
}

#[cfg(test)]
mod tests {
    use super::PlanOptions;
    use quote::quote;

    #[test]
    fn plan_options_reduce_by_default_and_allow_explicit_opt_out() {
        assert!(!syn::parse2::<PlanOptions>(quote!()).unwrap().no_reduce);
        assert!(
            syn::parse2::<PlanOptions>(quote!(no_reduce))
                .unwrap()
                .no_reduce
        );
    }

    #[test]
    fn plan_options_reject_unknown_and_extra_arguments() {
        for arguments in [
            quote!(unknown),
            quote!(no_reduce no_reduce),
            quote!(no_reduce, no_reduce),
            quote!(no_reduce,),
            quote!(no_reduce = true),
        ] {
            assert!(syn::parse2::<PlanOptions>(arguments).is_err());
        }
    }

    #[test]
    fn filtered_default_handles_correlated_test_blocks() {
        let source = syn::parse_quote! {
            struct Q; relation P(x:i32);
            answer(x) :- P(x), { if *x == 1 } | { if *x == 2 };
        };
        let normal = cq_lang_cq::normal::normalize(&source).unwrap();
        let ordering = cq_order::Options {
            heuristic: cq_order::Heuristic::Filtered,
            split: cq_order::Split::Coarse,
        };
        let vo = cq_order::compile(&normal, &ordering).unwrap();
        cq_order::check(&normal, &vo, &ordering).unwrap();
        let reductions = cq_reduce::Options {
            guards: cq_reduce::Pushdown::None,
            semijoins: cq_reduce::Reductions::Hypergraph,
        };
        let vo = cq_reduce::compile(&vo, &reductions).unwrap();
        let ra = cq_elaborate::compile(&vo).unwrap();
        cq_elaborate::check(&vo, &ra).unwrap();
        let ra = cq_elaborate::simplify_reductions(&ra).unwrap();
        let ra = cq_physical_lowering::orient(&ra).unwrap();
        let ix = cq_index_policy::compile(&ra).unwrap();
        cq_physical_lowering::compile(&ix).unwrap();
    }
}
