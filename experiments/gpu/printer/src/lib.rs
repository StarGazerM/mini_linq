//! Checked Structures → CUDA tile streams. Storage choices are never inferred
//! from query names: keyed reads use their recorded SortedRun, PrefixRun, or
//! exact integral membership provider;
//! each canon requires an explicitly recorded `cq_gpu_runtime::DeviceSet<_>`.
//! The result is an iterator of owned typed tuples. Unsupported operators and
//! representations are rejected before CUDA code is emitted.

use proc_macro::TokenStream;

mod compiler;
mod options;
mod predicate;
mod prefix;

/// Terminal pipeline pass. Optional control arguments are `tile(256,4)`,
/// `frontier(8)` (zero disables publishing), and `directory(rank)`.
/// Data/evaluation decisions come exclusively from checked Structures.
#[proc_macro]
pub fn print(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(input.into(), |source, arguments| {
        compiler::compile(source, syn::parse2(arguments)?)
    })
    .into()
}

/// Explicit Structures rewrite selecting a rank-backed prefix representation.
/// Remaining recorded key levels are exact residual predicates, not discarded.
#[proc_macro]
pub fn prefix_runs(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), prefix::compile).into()
}

#[cfg(test)]
mod tests;
