//! Shared domain checks and Rust signature spelling for the pure nested routes.
//! Execution environments and control flow belong to each route separately.

use cq_ir::{cq, relational_plan as ra, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::BTreeMap;

pub(crate) fn check(
    source: &ra::Module,
    inputs: &BTreeMap<String, syn::Path>,
) -> syn::Result<BTreeMap<String, ra::RelationInfo>> {
    ra::contract::check(source)?;
    let infos = ra::check_in(source.program.inputs(), &source.plan)?;
    for definition in &source.plan.definitions {
        let unsupported = match &definition.operator {
            ra::Operator::Select(_) => "Select",
            ra::Operator::Extend(_) => "Extend",
            _ => continue,
        };
        return Err(syn::Error::new_spanned(
            &definition.operator,
            format!(
                "nested execution does not admit {unsupported}; Rust expression operators require a different realization"
            ),
        ));
    }
    // This boundary interprets checked RA. Preservation of its source CQ is
    // the planner's responsibility; repeated output attributes remain valid.
    let heading = &infos[&symbol_name(&source.plan.output.input)].heading;
    for expression in &source.plan.output.head.args {
        if cq::rust::variable(expression)
            .is_none_or(|variable| !heading.contains(&symbol_name(variable)))
        {
            return Err(syn::Error::new_spanned(
                expression,
                "nested execution output must project attributes; Rust expression outputs are not admitted",
            ));
        }
    }
    let declarations = source
        .program
        .inputs()
        .map(|input| (symbol_name(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    for (name, provider) in inputs {
        if !declarations.contains_key(name) {
            return Err(syn::Error::new_spanned(
                provider,
                format!("input provider binding `{name}` does not name an input relation"),
            ));
        }
    }
    for (name, declaration) in declarations {
        if !inputs.contains_key(&name) {
            return Err(syn::Error::new_spanned(
                &declaration.name,
                format!("missing input provider for `{name}`"),
            ));
        }
    }
    Ok(infos)
}

pub(crate) fn arguments(source: &ra::Module) -> BTreeMap<String, syn::Ident> {
    source
        .program
        .inputs()
        .enumerate()
        .map(|(index, input)| {
            (
                symbol_name(&input.name),
                format_ident!("__cq_input{index}", span = Span::mixed_site()),
            )
        })
        .collect()
}

pub(crate) fn parameters(
    source: &ra::Module,
    inputs: &BTreeMap<String, syn::Path>,
    arguments: &BTreeMap<String, syn::Ident>,
    lifetime: &syn::Lifetime,
) -> Vec<TokenStream> {
    source
        .program
        .inputs()
        .map(|input| {
            let name = symbol_name(&input.name);
            let argument = &arguments[&name];
            let provider = &inputs[&name];
            let schema = input
                .columns
                .iter()
                .map(|column| crate::rust_bindings::qualify_type(&column.ty));
            quote!(#argument: &#lifetime #provider! { type (#(#schema,)*) })
        })
        .collect()
}

pub(crate) fn output_type(
    source: &ra::Module,
    info: &ra::RelationInfo,
    lifetime: &syn::Lifetime,
) -> syn::Result<TokenStream> {
    let types = ra::output_types(&source.program, info)?
        .iter()
        .map(crate::rust_bindings::qualify_type)
        .collect::<Vec<_>>();
    Ok(quote!((#(&#lifetime #types,)*)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn providers() -> BTreeMap<String, syn::Path> {
        BTreeMap::from([("R".into(), syn::parse_quote!(rows))])
    }

    fn source() -> ra::Module {
        syn::parse_quote! {
            struct Query;
            relation R(value: i32);
            answer(x) :- R(x);
            relational {
                rows = rename R { value -> x };
                output rows as answer(x);
            }
        }
    }

    fn rejects(source: &ra::Module, inputs: &BTreeMap<String, syn::Path>, message: &str) {
        // The fixtures are valid RA: it is this realization's smaller domain
        // or its provider configuration that must reject them.
        ra::contract::check(source).unwrap();
        #[cfg(feature = "nested-push")]
        assert!(
            crate::nested_push::compile(source, inputs)
                .unwrap_err()
                .to_string()
                .contains(message)
        );
        #[cfg(feature = "nested-pull")]
        assert!(
            crate::nested_pull::compile(source, inputs)
                .unwrap_err()
                .to_string()
                .contains(message)
        );
    }

    #[test]
    fn selection_requires_a_different_realization() {
        let source = syn::parse_quote! {
            struct Query;
            relation R(value: i32);
            answer(x) :- R(x), if x > 0;
            relational {
                rows = rename R { value -> x };
                filtered = select rows if (x > 0);
                output filtered as answer(x);
            }
        };
        rejects(&source, &providers(), "does not admit Select");
    }

    #[test]
    fn extension_requires_a_different_realization() {
        let source = syn::parse_quote! {
            struct Query;
            relation R(value: i32);
            answer(y) :- R(x), let y: i32 = x + 1;
            relational {
                rows = rename R { value -> x };
                computed = extend rows with { y: i32 } value ((x + 1,));
                result = project computed keep { y };
                output result as answer(y);
            }
        };
        rejects(&source, &providers(), "does not admit Extend");
    }

    #[test]
    fn computed_output_requires_a_different_realization() {
        let source = syn::parse_quote! {
            struct Query;
            relation R(value: i32);
            relation answer(value: i32);
            answer(x + 1) :- R(x);
            relational {
                rows = rename R { value -> x };
                output rows as answer(x + 1);
            }
        };
        rejects(
            &source,
            &providers(),
            "Rust expression outputs are not admitted",
        );
    }

    #[test]
    fn missing_provider_is_rejected_before_emission() {
        rejects(
            &source(),
            &BTreeMap::new(),
            "missing input provider for `R`",
        );
    }

    #[test]
    fn extra_provider_is_rejected_before_emission() {
        let mut inputs = providers();
        inputs.insert("Unknown".into(), syn::parse_quote!(other_rows));
        rejects(
            &source(),
            &inputs,
            "input provider binding `Unknown` does not name an input relation",
        );
    }
}
