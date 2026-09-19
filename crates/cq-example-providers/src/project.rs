//! Selected RA projection through the existing exact-family output operation.
//!
//! Terminal projection retains the source clauses and asks the recorded result
//! provider to construct its carrier from the retained logical fields. A later
//! operator could reintroduce a dropped name that is still bound in those
//! clauses; such intermediate projections require separate scope handling and
//! are explicitly outside this rule's admitted domain.

use cq_ir::{
    CommaList,
    provider_protocol::{self as protocol, exact},
    relational_plan::{Operator, Project},
    rust_access_plan, symbol_name,
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::{
    composition::{self, field_values},
    layout,
    syntax::AccessState,
};

fn projection(request: &protocol::Request) -> syn::Result<(&Ident, &Project)> {
    let protocol::Task::Lower { name, .. } = &request.request else {
        return Err(syn::Error::new_spanned(
            &request.request,
            "project is an RA lowering rule",
        ));
    };
    match &layout::definition(&request.plan, name)?.operator {
        Operator::Project(project) => {
            let ra = &request.plan.index_requirements.relational_plan.plan;
            if symbol_name(&ra.output.input) != symbol_name(name)
                || !ra
                    .definitions
                    .last()
                    .is_some_and(|definition| symbol_name(&definition.result) == symbol_name(name))
            {
                return Err(syn::Error::new_spanned(
                    name,
                    "selected project rule supports only the final output definition; intermediate projection requires explicit logical-variable scope handling",
                ));
            }
            Ok((name, project))
        }
        _ => Err(syn::Error::new_spanned(
            name,
            "selected project rule requires Project",
        )),
    }
}

pub fn lower(tokens: TokenStream) -> syn::Result<TokenStream> {
    let request: protocol::Request = syn::parse2(tokens)?;
    let (_, project) = projection(&request)?;
    let input = &project.input;
    match request.environment.relation(input) {
        Some(protocol::Binding::Stored { operations, .. }) => {
            let plan = &request.plan;
            let environment = &request.environment;
            Ok(quote! { #operations! {
                reply_to (::cq_example_providers::project_rows) { #request }
                plan { #plan }
                environment { #environment }
                request { rows #input }
            } })
        }
        Some(protocol::Binding::Region { access, .. }) => {
            let values = &access.head.atom.args;
            let ordinals = (0..values.len()).map(syn::Index::from);
            let values = values.iter();
            let row = &access.head.output;
            let witness: exact::Witness = syn::parse2(quote! {
                row (#row) fields { #(#ordinals = (#values);)* }
            })?;
            request_output(&request, &access.body, &witness)
        }
        _ => Err(syn::Error::new_spanned(
            input,
            "project input has no established representation",
        )),
    }
}

pub fn rows(tokens: TokenStream) -> syn::Result<TokenStream> {
    let reply: exact::IteratorReply = syn::parse2(tokens)?;
    let request: protocol::Request = syn::parse2(reply.resume.clone())?;
    let (_, project) = projection(&request)?;
    let clause = composition::bind(&request.plan, &project.input, &reply)?;
    let clauses = [clause].into_iter().collect();
    request_output(&request, &clauses, &reply.witness)
}

fn request_output(
    request: &protocol::Request,
    clauses: &CommaList<rust_access_plan::Clause>,
    witness: &exact::Witness,
) -> syn::Result<TokenStream> {
    let (name, project) = projection(request)?;
    let fields = field_values(witness, &layout::heading(&request.plan, &project.input)?)?;
    let heading = layout::heading(&request.plan, name)?;
    let values = heading
        .iter()
        .map(|attribute| {
            fields.get(&symbol_name(attribute)).ok_or_else(|| {
                syn::Error::new_spanned(attribute, "projected field has no input witness")
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let ordinals = (0..values.len()).map(syn::Index::from);
    let provider = layout::relation_provider(&request.plan, name)?;
    let plan = &request.plan;
    let environment = &request.environment;
    let input = &project.input;
    let row = &witness.row;
    Ok(quote! { #provider! {
        reply_to (::cq_example_providers::project_output) {
            request { #request }
            access { #clauses }
            witness { #witness }
        }
        plan { #plan }
        environment { #environment }
        request { output #name fields { #(#ordinals = (#values);)* }
            whole { #input = (#row); }
        }
    } })
}

pub fn output(tokens: TokenStream) -> syn::Result<TokenStream> {
    let reply: exact::WitnessReply = syn::parse2(tokens)?;
    let state: AccessState = syn::parse2(reply.resume)?;
    let (name, _) = projection(&state.request)?;
    let heading = layout::heading(&state.request.plan, name)?;
    let fields = field_values(&reply.witness, &heading)?;
    let values: Vec<_> = heading
        .iter()
        .map(|attribute| &fields[&symbol_name(attribute)])
        .collect();
    let clauses = &state.clauses;
    let row = &reply.witness.row;
    let access: rust_access_plan::Plan = syn::parse2(quote! {
        #name(#(#values),*) => (#row) :- #clauses;
    })?;
    rust_access_plan::contract::check(&access)?;
    let callback = &state.request.reply_to;
    let resume = &state.request.state;
    Ok(quote! { #callback! {
        resume { #resume }
        emitted {}
        result { region #name { #access } }
    } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intermediate_projection_rejects_before_reusing_dropped_variable_scope() {
        let plan: cq_ir::physical::Module = syn::parse_quote! {
            struct Intermediate;
            relation R(a:i32,b:i32);
            relation S(a:i32,b:i32);
            answer(a,b) :- R(a,unused), S(a,b);
            relational {
                r = rename R {a -> a, b -> b};
                p = project r keep {a};
                s = rename S {a -> a, b -> b};
                j = natural_join p with s;
                output j as answer(a,b);
            }
            indexes {i = @s[0];}
            uses {j.right = i;}
            physical {
                relations {R = ext::map; S = ext::map; r = ext::map; p = ext::fields; s = ext::map; j = ext::fields;}
                indexes {i = ext::map;}
                lowerings {r = ext::rename; p = ext::project; s = ext::rename; j = ext::join;}
            }
        };
        cq_ir::physical::contract::check_structure(&plan).unwrap();
        let error = lower(quote! {
            reply_to (driver::resume) {}
            plan {#plan}
            environment {}
            request {lower p}
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only the final output definition")
        );
    }
}
