//! Procedural entry points at the compiler's explicit language boundaries.
//!
//! Queries denote `bottom + Set`: one panic aborts evaluation. Opaque Rust
//! binds and guards keep their written chain; relational steps may move only
//! earlier across them. Their free variables determine their partial-function
//! meaning; I/O, mutation and evaluation counts are outside that meaning.
//! Equality, keyed storage and deduplication require Eq, with Hash or Ord
//! supplied by the selected representation.
//!
//! CQ planning produces RA; IX records index demands; Physical
//! fixes one physical operator per result and the mode its reads allow,
//! under a caller-supplied, query-independent policy; Structures answers
//! built let reads with a provider-selected Rust type over the
//! result, using the caller-bound `index_provider!`; realization prints one pull
//! program over the runtime's traits. Caller import
//! aliases bind policy and provider slots. No global storage default or
//! fallback whole-query backend is supplied.
//!
//! [`relation_loaders!`] is a caller-side convenience independent of query
//! planning. Invoke it through an authored workload callback, for example
//! `workload::with_query!(mini_linq::relation_loaders)`, to reuse the exact CQ
//! tokens. It also accepts just relation declarations. An input declaration
//! `relation Lineitem(orderkey: i64, price: cq_load::Decimal);` emits a
//! `LineitemLoader` type; result declarations in a complete CQ are excluded.
//! No new CQ syntax or loading semantics are introduced into the language.
//!
//! `LineitemLoader::csv(path, cq_load::CsvOptions::default())` returns owned
//! `(i64, Decimal)` tuples. `csv_hash::<1>` and `csv_btree::<1>` group into
//! `(i64,)` keys and `Vec<(Decimal,)>` payloads. The const parameter selects
//! any leading prefix, including zero and the entire row. Only key fields
//! require `Eq + Hash` or `Ord`; payloads may contain `f64` or user-defined
//! `cq_load::Load` implementations. Rows and duplicate payloads retain input
//! order within each group. `csv_batches` yields bounded typed row batches.
//! With `cq-load`'s optional `parquet` feature, the same generated type also
//! provides `parquet`, `parquet_hash`, `parquet_btree`, and `parquet_batches`,
//! using `cq_load::ParquetOptions`. Source options and scalar syntax are
//! documented by the runtime; all parse errors retain source context.
//! A local `relation_loader!(Lineitem)` selector expands to `LineitemLoader`,
//! allowing declarative callbacks to call `<relation_loader!(Lineitem)>::csv`
//! in the same scope without concatenating identifiers. Relation names remain
//! available for authored Rust column types, including `String` and user types.

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
mod load;
mod workload;

/// Generate typed source readers from declarations or a complete CQ callback.
#[proc_macro]
pub fn relation_loaders(input: TokenStream) -> TokenStream {
    load::expand(input)
}

/// Capture a typed CQ for the workload catalog and export a token callback.
/// The leaf re-exports `with_<head>!` (also `with_query!`); passing a macro path
/// invokes it with the authored token stream, preserving Rust hygiene.
#[proc_macro]
pub fn workload_query(input: TokenStream) -> TokenStream {
    workload::expand(input)
}

/// Select a logical planner without bypassing its caller-resolved macro binding.
#[cfg(feature = "logical-selection")]
#[proc_macro]
/// Formal rule ST-Entry: Check and dispatch unchanged CQ to the caller-resolved logical slot.
pub fn mini_linq(input: TokenStream) -> TokenStream {
    entry::stage(input, None, |source: &cq_lang_cq::Program| {
        cq_lang_cq::contract::check(source)?;
        Ok(quote::quote!(mini_linq::cq_to_ra! { #source }))
    })
}

/// CQ → RA in written order: normalize, schedule clauses as written, one
/// full subatom per atom, elaborate to named RA.
#[cfg(feature = "written-order")]
#[proc_macro]
/// Formal rule Plan-Prefix: Normalize, order Written/Coarse, elaborate and continue with RA.
pub fn cq_to_ra(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        |source: &cq_lang_cq::Program| {
            let normal = cq_lang_cq::normal::normalize(source)?;
            let plan = cq_order::compile(&normal, &cq_order::Options::default())?;
            cq_elaborate::compile(&plan)
        },
    )
}

/// CQ → RA variable at a time (generic join): normalize, rank connected
/// prefixes and check every atom holding each bound variable. The Rust chain
/// and its written relational prerequisites remain fixed.
#[cfg(feature = "written-order")]
#[proc_macro]
/// Formal rule Plan-Generic: Normalize, order Prefix(Connected)/Fine, elaborate and continue with RA.
pub fn cq_to_ra_generic(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        |source: &cq_lang_cq::Program| {
            let normal = cq_lang_cq::normal::normalize(source)?;
            let options = cq_order::Options {
                heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
                split: cq_order::Split::Fine,
            };
            let plan = cq_order::compile(&normal, &options)?;
            cq_elaborate::compile(&plan)
        },
    )
}

// These selectors retain cq_to_ra!'s payload and caller-bound continuation.
macro_rules! planner_selector {
    ($name:ident, $heuristic:expr, $split:ident) => {
        #[cfg(feature = "written-order")]
        #[proc_macro]
        #[doc = concat!("CQ → RA using ", stringify!($heuristic), "/", stringify!($split), ".")]
        pub fn $name(input: TokenStream) -> TokenStream {
            entry::stage(
                input,
                Some(syn::parse_quote!(mini_linq::choose_index_policy)),
                |source: &cq_lang_cq::Program| {
                    let normal = cq_lang_cq::normal::normalize(source)?;
                    let plan = cq_order::compile(
                        &normal,
                        &cq_order::Options {
                            heuristic: $heuristic,
                            split: cq_order::Split::$split,
                        },
                    )?;
                    cq_elaborate::compile(&plan)
                },
            )
        }
    };
}
planner_selector!(
    cq_to_ra_prefix_connected,
    cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
    Coarse
);
planner_selector!(
    cq_to_ra_prefix_connected_fine,
    cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
    Fine
);
planner_selector!(
    cq_to_ra_prefix_greedy,
    cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
    Coarse
);
planner_selector!(
    cq_to_ra_prefix_greedy_fine,
    cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
    Fine
);
planner_selector!(
    cq_to_ra_elimination,
    cq_order::Heuristic::Elimination,
    Coarse
);
planner_selector!(
    cq_to_ra_elimination_fine,
    cq_order::Heuristic::Elimination,
    Fine
);

/// CQ → RA with Yannakakis reduction: normalize, schedule clauses as
/// written, push single-atom guards onto their atoms, semijoin-reduce along
/// the GYO join tree when the query is acyclic, elaborate to named RA.
#[cfg(feature = "hypergraph")]
#[proc_macro]
/// Formal rule Plan-Graph: Normalize, Written/Coarse order, Atoms pushdown, Tree iff acyclic, elaborate.
pub fn cq_to_ra_hypergraph(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        |source: &cq_lang_cq::Program| {
            let normal = cq_lang_cq::normal::normalize(source)?;
            let plan = cq_order::compile(&normal, &cq_order::Options::default())?;
            let semijoins = if cq_lang_cq::hypergraph::Analysis::of(&normal).acyclic {
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
        |source: &cq_lang_ra::Program| cq_index_policy::compile(source),
    )
}

/// Fix one physical operator per RA result under the appended policy.
///
/// Input is `IX policy { … }`. A natural join becomes a right-side `probe`,
/// or a `merge` under `join = merge` when the shared heading is nonempty;
/// Cartesian joins remain `probe[]`. Every semijoin becomes `semi`, a select
/// bounding a leading declared column a `range`; modes follow
/// from use; the return is flagged `canon` unless uniqueness is inferred
/// from plan-internal canon boundaries. The policy is re-appended for [`select_structures!`].
#[cfg(feature = "physical-lowering")]
#[proc_macro]
pub fn lower_physical(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::select_structures)),
        |source: &entry::Policed<cq_lang_ix::Program, cq_lang_physical::policy::Policy>| {
            let target = cq_physical_lowering::compile(&source.language, &source.policy)?;
            let policy = &source.policy;
            Ok(quote::quote! { #target #policy })
        },
    )
}

/// Bind every keyed read to a value that answers it under the appended policy.
///
/// Input is `Physical policy { … }`. Built let reads select `index_provider!`
/// types by read kind. Host reads use type dispatch and declaration layout;
/// rustc checks direct keyed and ordered operations. Continues to [`structures_to_rust!`].
#[cfg(feature = "structure-selection")]
#[proc_macro]
pub fn select_structures(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        Some(syn::parse_quote!(mini_linq::structures_to_rust)),
        |source: &entry::Policed<cq_lang_physical::Program, cq_lang_physical::policy::Policy>| {
            let target = cq_structure_selection::compile(&source.language, &source.policy)?;
            let policy = &source.policy;
            Ok(quote::quote! { #target #policy })
        },
    )
}

/// Print Structures as one pull program: a block that builds the chosen
/// structures, guards the traits each read needs, and evaluates to an
/// iterator of the answer tuples in head order. A `canon` result
/// uses the caller's `mini_linq::distinct_state!` provider, including before a canonical let is materialized.
#[cfg(feature = "physical-realization")]
#[proc_macro]
/// Formal rule ST-Terminal: Realize the policed Structures payload with no next continuation.
pub fn structures_to_rust(input: TokenStream) -> TokenStream {
    entry::stage(
        input,
        None,
        |source: &entry::Policed<cq_lang_structures::Program, cq_lang_physical::policy::Policy>| {
            let endpoints = cq_physical_realization::Endpoints {
                distinct_state: syn::parse_quote!(mini_linq::distinct_state),
                index_provider: syn::parse_quote!(mini_linq::index_provider),
                intersect: source.policy.smallest_intersections(),
            };
            cq_physical_realization::compile(&source.language, &endpoints)
        },
    )
}
