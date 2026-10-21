//! Test-only bridge from checked execution certificates to the base Round oracle.
use quote::quote;

/// Check the extension language, then evaluate its recorded Round meaning.
/// The base interpreter reads certified whole-input bindings as complete sets;
/// shard ownership and barriers are validated by the extension contract.
#[proc_macro]
pub fn eval_round(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::pass::<cq_datalog_partition::ShardedProgram, syn::Expr>(
        input.into(),
        |program, arguments| {
            if !arguments.is_empty() {
                return Err(syn::Error::new_spanned(
                    arguments,
                    "reference terminal takes no arguments",
                ));
            }
            let round = &program.round;
            syn::parse2(quote!(
                ::cq_datalog_test_passes::eval_round! { pipeline {}; #round }
            ))
        },
    )
    .into()
}
