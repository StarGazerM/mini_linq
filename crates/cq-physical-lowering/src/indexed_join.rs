//! Executable lowering rules selected separately from storage providers.

use cq_ir::{
    CommaList,
    provider_protocol::{self as protocol, exact},
    relational_plan::Operator,
    rust_access_plan, symbol_name,
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use cq_ir::physical::lookup as layout;

use crate::{
    composition::{self, field_values},
    syntax::AccessState,
};

fn join(
    request: &protocol::Request,
) -> syn::Result<(&Ident, &cq_ir::relational_plan::NaturalJoin)> {
    let protocol::Task::Lower { name, .. } = &request.request else {
        return Err(syn::Error::new_spanned(
            &request.request,
            "indexed_join is an RA lowering rule",
        ));
    };
    match &layout::definition(&request.plan, name)?.operator {
        Operator::NaturalJoin(join) => Ok((name, join)),
        _ => Err(syn::Error::new_spanned(
            name,
            "selected indexed_join rule requires NaturalJoin",
        )),
    }
}

pub fn compile(tokens: TokenStream) -> syn::Result<TokenStream> {
    let request: protocol::Request = syn::parse2(tokens)?;
    let (_, join) = join(&request)?;
    let left = &join.left;
    match request.environment.relation(left) {
        Some(protocol::Binding::Stored { operations, .. }) => {
            let plan = &request.plan;
            let environment = &request.environment;
            Ok(quote! { #operations! {
                reply_to (::mini_linq::indexed_join_rows) { #request }
                plan { #plan }
                environment { #environment }
                request { rows #left }
            } })
        }
        Some(protocol::Binding::Region { access, .. }) => {
            let values: Vec<_> = access.head.atom.args.iter().collect();
            let ordinals = (0..values.len()).map(syn::Index::from);
            let output = &access.head.output;
            let witness: exact::Witness = syn::parse2(quote! {
                row (#output) fields { #(#ordinals = (#values);)* }
            })?;
            matches_request(&request, &access.body, &witness)
        }
        _ => Err(syn::Error::new_spanned(
            left,
            "left operand has no established representation",
        )),
    }
}

pub fn rows(tokens: TokenStream) -> syn::Result<TokenStream> {
    let reply: exact::IteratorReply = syn::parse2(tokens)?;
    let request: protocol::Request = syn::parse2(reply.resume.clone())?;
    let (_, join) = join(&request)?;
    let clause = composition::bind(&request.plan, &join.left, &reply)?;
    let clauses = [clause].into_iter().collect();
    matches_request(&request, &clauses, &reply.witness)
}

fn matches_request(
    request: &protocol::Request,
    clauses: &CommaList<rust_access_plan::Clause>,
    left: &exact::Witness,
) -> syn::Result<TokenStream> {
    let (name, join) = join(request)?;
    let left_values = field_values(left, &layout::heading(&request.plan, &join.left)?)?;
    let index_use = request
        .plan
        .index_requirements
        .uses
        .iter()
        .find(|u| symbol_name(&u.result) == symbol_name(name))
        .ok_or_else(|| syn::Error::new_spanned(name, "missing explicit IX join use"))?;
    let index = request
        .plan
        .index_requirements
        .indexes
        .iter()
        .find(|i| symbol_name(&i.name) == symbol_name(&index_use.index))
        .ok_or_else(|| syn::Error::new_spanned(index_use, "join use has no IX definition"))?;
    if symbol_name(&index.requirement.relation) != symbol_name(&join.right) {
        return Err(syn::Error::new_spanned(
            index_use,
            "index use is not the exact right relation version",
        ));
    }
    let right_heading = layout::heading(&request.plan, &join.right)?;
    // Consume D and U exactly. IX alone discovers which columns need indexes.
    let values = index
        .requirement
        .columns
        .iter()
        .map(|column| {
            right_heading
                .get(column.index as usize)
                .and_then(|attribute| left_values.get(&symbol_name(attribute)))
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        column,
                        "IX lookup column is not supplied by the left witness",
                    )
                })
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let index_name = &index.name;
    let Some(protocol::Binding::Indexed { operations, .. }) = request.environment.index(index_name)
    else {
        return Err(syn::Error::new_spanned(
            index_name,
            "selected exact index must be established before join lowering",
        ));
    };
    let plan = &request.plan;
    let environment = &request.environment;
    Ok(quote! { #operations! {
        reply_to (::mini_linq::indexed_join_matches) {
            request { #request }
            access { #clauses }
            witness { #left }
        }
        plan { #plan }
        environment { #environment }
        request { matches #index_name values [#((#values)),*] }
    } })
}

pub fn matches(tokens: TokenStream) -> syn::Result<TokenStream> {
    let reply: exact::IteratorReply = syn::parse2(tokens)?;
    let mut state: AccessState = syn::parse2(reply.resume.clone())?;
    let request = &state.request;
    let (name, join) = join(request)?;
    let mut values = field_values(&state.witness, &layout::heading(&request.plan, &join.left)?)?;
    for (attribute, value) in field_values(
        &reply.witness,
        &layout::heading(&request.plan, &join.right)?,
    )? {
        values.entry(attribute).or_insert(value);
    }
    let heading = layout::heading(&request.plan, name)?;
    let values = heading
        .iter()
        .map(|attribute| {
            values.get(&symbol_name(attribute)).ok_or_else(|| {
                syn::Error::new_spanned(attribute, "join result field has no operand witness")
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let ordinals = (0..values.len()).map(syn::Index::from);
    let provider = layout::relation_provider(&request.plan, name)?;
    let left_name = &join.left;
    let right_name = &join.right;
    let left_row = &state.witness.row;
    let right_row = &reply.witness.row;
    state
        .clauses
        .push(composition::bind(&request.plan, &join.right, &reply)?);
    let clauses = &state.clauses;
    let witness = &state.witness;
    let plan = &request.plan;
    let environment = &request.environment;
    Ok(quote! { #provider! {
        reply_to (::mini_linq::indexed_join_output) {
            request { #request }
            access { #clauses }
            witness { #witness }
        }
        plan { #plan }
        environment { #environment }
        request { output #name fields { #(#ordinals = (#values);)* }
            whole { #left_name = (#left_row); #right_name = (#right_row); }
        }
    } })
}

pub fn output(tokens: TokenStream) -> syn::Result<TokenStream> {
    let reply: exact::WitnessReply = syn::parse2(tokens)?;
    let state: AccessState = syn::parse2(reply.resume)?;
    let (name, _) = join(&state.request)?;
    let heading = layout::heading(&state.request.plan, name)?;
    let fields = field_values(&reply.witness, &heading)?;
    let values: Vec<_> = heading
        .iter()
        .map(|attribute| &fields[&symbol_name(attribute)])
        .collect();
    let clauses = &state.clauses;
    let output = &reply.witness.row;
    let access: rust_access_plan::Plan = syn::parse2(quote! {
        #name(#(#values),*) => (#output) :- #clauses;
    })?;
    rust_access_plan::contract::check(&access)?;
    let callback = &state.request.reply_to;
    let resume = &state.request.state;
    Ok(quote! { #callback! {
        resume { #resume }
        emitted { }
        result { region #name { #access } }
    } })
}
