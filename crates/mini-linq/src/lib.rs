//! Procedural entry points at the compiler's explicit language boundaries.
//!
//! CQ planning produces RA; IX records index demands; Physical
//! fixes one physical operator per result and the mode its reads allow,
//! under a caller-supplied, query-independent policy; Structures answers
//! every keyed read with a host key fact or a Rust type built over the
//! result, with per-relation `#[index]` advice; realization prints one pull
//! program over the runtime's traits. Access and Pipeline remain as
//! hand-staged entry points into the execution endpoints. Caller import
//! aliases bind policy and provider slots. No global storage default or
//! fallback whole-query backend is supplied.

use proc_macro::TokenStream;
#[cfg(any(
    feature = "logical-selection",
    feature = "written-order",
    feature = "hypergraph",
    feature = "index-demands",
    feature = "structure-selection",
    feature = "physical-lowering",
    feature = "physical-realization"
))]
mod entry;
mod workload;

/// Capture a typed CQ for the workload catalog without compiling a host API.
#[proc_macro]
pub fn workload_query(input: TokenStream) -> TokenStream {
    workload::expand(input)
}

/// Select a logical planner without bypassing its caller-resolved macro binding.
#[cfg(feature = "logical-selection")]
#[proc_macro]
pub fn mini_linq(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::cq::Module| {
        cq_ir::cq::contract::check(source)?;
        Ok(quote::quote!(mini_linq::cq_to_ra! { #source }))
    })
}

/// CQ → RA in written order: normalize, schedule clauses as written, one
/// full subatom per atom, elaborate to named RA.
#[cfg(feature = "written-order")]
#[proc_macro]
pub fn cq_to_ra(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        |source: &cq_ir::cq::Module| {
            let normal = cq_ir::cq::normal::normalize(source)?;
            let plan = cq_order::compile(&normal, &cq_order::Options::default())?;
            cq_elaborate::compile(&plan)
        },
    )
}

/// CQ → RA variable at a time (generic join): normalize, schedule clauses
/// as written, bind one variable per step and check every other atom that
/// holds it before enumerating further, elaborate to named RA.
#[cfg(feature = "written-order")]
#[proc_macro]
pub fn cq_to_ra_generic(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        |source: &cq_ir::cq::Module| {
            let normal = cq_ir::cq::normal::normalize(source)?;
            let options = cq_order::Options {
                split: cq_order::Split::Fine,
                ..cq_order::Options::default()
            };
            let plan = cq_order::compile(&normal, &options)?;
            cq_elaborate::compile(&plan)
        },
    )
}

/// CQ → RA with Yannakakis reduction: normalize, schedule clauses as
/// written, push single-atom guards onto their atoms, semijoin-reduce along
/// the GYO join tree when the query is acyclic, elaborate to named RA.
#[cfg(feature = "hypergraph")]
#[proc_macro]
pub fn cq_to_ra_hypergraph(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        |source: &cq_ir::cq::Module| {
            let normal = cq_ir::cq::normal::normalize(source)?;
            let plan = cq_order::compile(&normal, &cq_order::Options::default())?;
            let semijoins = if cq_ir::cq::hypergraph::Analysis::of(&normal).acyclic {
                cq_reduce::Reductions::Tree
            } else {
                cq_reduce::Reductions::None
            };
            let reduced = cq_reduce::compile(
                &plan,
                &cq_reduce::Options {
                    guards: cq_reduce::Pushdown::Atoms,
                    semijoins,
                },
            )?;
            cq_elaborate::compile(&reduced)
        },
    )
}

/// Construct exact-version index demands for every natural join and semijoin.
///
/// The caller binds `mini_linq::physical_policy` to a macro that appends its
/// query-independent `policy { … }` block and invokes [`lower_physical!`].
#[cfg(feature = "index-demands")]
#[proc_macro]
pub fn index_demands(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::physical_policy)),
        cq_index_policy::compile,
    )
}

/// Fix one physical operator per RA result under the appended policy.
///
/// Input is `IX policy { … }`. Every natural join becomes a `probe` of the
/// RA join's right operand (or a `merge` under `join = merge`), every semijoin a
/// `semi`, a select bounding an ordered host key a `range`; modes follow
/// from use; the return is flagged `canon` unless uniqueness is inferred
/// from `canon` hosts. The policy is re-appended for [`select_structures!`].
#[cfg(feature = "physical-lowering")]
#[proc_macro]
pub fn lower_physical(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::select_structures)),
        |source: &entry::Policed<cq_ir::index_requirements::Module, cq_ir::policy::Policy>| {
            let target = cq_physical_lowering::compile(&source.language, &source.policy)?;
            let policy = &source.policy;
            Ok(quote::quote! { #target #policy })
        },
    )
}

/// Bind every keyed read to a value that answers it under the appended policy.
///
/// Input is `Physical policy { … }`. Host facts come from the policy (`R
/// keyed [a] ordered canon`). Each read no host key answers gets a
/// structure: the type the relation's `#[index(…)]` advice names, else
/// `cq_storage::HashIndex` for probes, `HashSet` for semijoins, `BTreeMap`
/// for merges and ranges. Whether a type implements the traits the plan uses
/// is rustc's check. Continues to [`structures_to_rust!`].
#[cfg(feature = "structure-selection")]
#[proc_macro]
pub fn select_structures(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::structures_to_rust)),
        |source: &entry::Policed<cq_ir::physical::Module, cq_ir::policy::Policy>| {
            cq_structure_selection::compile(&source.language, &source.policy)
        },
    )
}

/// Print Structures as one pull program: a block that builds the chosen
/// structures, guards the traits each read needs, and evaluates to an
/// iterator of the answer tuples in head order. A `canon` return
/// deduplicates through the caller's `mini_linq::distinct_state!` provider.
#[cfg(feature = "physical-realization")]
#[proc_macro]
pub fn structures_to_rust(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_ir::structures::Module| {
        let endpoints = cq_physical_realization::Endpoints {
            distinct_state: syn::parse_quote!(mini_linq::distinct_state),
        };
        cq_physical_realization::compile(source, &endpoints)
    })
}
