//! Lower direct-input Rename using the selected result representation.

use cq_ir::{physical::lookup as layout, provider_protocol as protocol, relational_plan::Operator};
use proc_macro2::TokenStream;
use quote::quote;

pub fn compile(tokens: TokenStream) -> syn::Result<TokenStream> {
    let request: protocol::Request = syn::parse2(tokens)?;
    let protocol::Task::Lower { name, .. } = &request.request else {
        return Err(syn::Error::new_spanned(
            &request.request,
            "rename is an RA lowering rule",
        ));
    };
    let Operator::Rename(rename) = &layout::definition(&request.plan, name)?.operator else {
        return Err(syn::Error::new_spanned(
            name,
            "selected rename rule requires a Rename definition",
        ));
    };
    let (source, _) = layout::stored(&request.environment, &rename.relation)?;
    let provider = layout::relation_provider(&request.plan, name)?;
    let callback = &request.reply_to;
    let state = &request.state;
    let plan = &request.plan;
    let environment = &request.environment;
    // The result provider re-establishes its declared representation. Merely
    // copying the source operations would ignore Φ_R(result).
    Ok(quote! { #provider! {
        reply_to (#callback) { #state }
        plan { #plan }
        environment { #environment }
        request { input #name from (#source) }
    } })
}
