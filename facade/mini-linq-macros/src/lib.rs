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
        plan = ::mini_linq::passes::plan_written;
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

/// CQ → RA′ in written order.
/// Formal rule Plan-Prefix: Normalize, order Written/Coarse, elaborate and continue with RA.
#[proc_macro]
pub fn plan_written(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_cq::Program, cq_lang_ra::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        let normal = cq_lang_cq::normal::normalize(source)?;
        let plan = cq_order::compile(&normal, &cq_order::Options::default())?;
        cq_order::check(&normal, &plan, &cq_order::Options::default())?;
        let target = cq_elaborate::compile(&plan)?;
        cq_elaborate::check(&plan, &target)?;
        Ok(target)
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
#[proc_macro]
pub fn select(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_physical::Program, cq_lang_structures::Program>(
        input.into(),
        |source, args| {
            no_arguments(args)?;
            cq_structure_selection::compile(source)
        },
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
