//! Domain and borrowed signatures for the scan-based physical implementation.

use cq_ir::{cq, index_requirements as ix, relational_plan as ra, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::BTreeMap;

pub(super) fn check(
    source: &ix::Module,
    inputs: &BTreeMap<String, syn::Path>,
) -> syn::Result<BTreeMap<String, ra::RelationInfo>> {
    ix::contract::check(source)?;
    if !source.indexes.is_empty() {
        return Err(syn::Error::new_spanned(
            source,
            "borrowed scans require an empty index overlay; select the no_indexes policy",
        ));
    }
    let source = &source.relational_plan;
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
                "borrowed scans does not admit {unsupported}; Rust expression operators require a different realization"
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
                "borrowed scans output must project attributes; Rust expression outputs are not admitted",
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

pub(super) fn arguments(source: &ra::Module) -> BTreeMap<String, syn::Ident> {
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

pub(super) fn parameters(
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

pub(super) fn output_type(
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
    use crate::borrowed_scans::{Options, compile};

    fn source() -> ix::Module {
        syn::parse_quote! {
            struct Query;
            relation R(value: i32);
            answer(x) :- R(x);
            relational {
                rows = rename R { value -> x };
                output rows as answer(x);
            }
            indexes {}
        }
    }

    fn options() -> Options {
        Options {
            inputs: BTreeMap::from([("R".into(), syn::parse_quote!(rows))]),
            stream: syn::parse_quote!(support),
            output: syn::parse_quote!(answers),
            exists: syn::parse_quote!(witness),
            replay: syn::parse_quote!(replay),
        }
    }

    fn rejects(source: &ix::Module, options: &Options, diagnostic: &str) {
        // Each fixture is valid IX; these are the selected realization's
        // additional obligations, rather than parser or input-IR failures.
        ix::contract::check(source).unwrap();
        let error = compile(source, options).unwrap_err();
        assert!(error.to_string().contains(diagnostic), "{error}");
    }

    #[test]
    fn provider_bindings_must_match_declared_inputs_exactly() {
        let source = source();
        let mut missing = options();
        missing.inputs.clear();
        rejects(&source, &missing, "missing input provider for `R`");
        let mut extra = options();
        extra
            .inputs
            .insert("Unknown".into(), syn::parse_quote!(other_rows));
        rejects(
            &source,
            &extra,
            "input provider binding `Unknown` does not name an input relation",
        );
    }

    #[test]
    fn a_nonempty_index_overlay_cannot_be_silently_ignored() {
        for requirement in [syn::parse_quote!(R[0]), syn::parse_quote!(@rows[0])] {
            let mut source = source();
            source.indexes.push(requirement);
            rejects(&source, &options(), "require an empty index overlay");
        }
    }

    #[test]
    fn selection_and_extension_require_another_realization() {
        let selection = syn::parse_quote! {
            struct Query;
            relation R(value: i32);
            answer(x) :- R(x), if x > 0;
            relational {
                rows = rename R { value -> x };
                filtered = select rows if (x > 0);
                output filtered as answer(x);
            }
            indexes {}
        };
        rejects(&selection, &options(), "does not admit Select");
        let extension = syn::parse_quote! {
            struct Query;
            relation R(value: i32);
            answer(y) :- R(x), let y: i32 = x + 1;
            relational {
                rows = rename R { value -> x };
                computed = extend rows with { y: i32 } value ((x + 1,));
                result = project computed keep { y };
                output result as answer(y);
            }
            indexes {}
        };
        rejects(&extension, &options(), "does not admit Extend");
    }

    #[test]
    fn computed_output_cannot_be_emitted_as_a_borrowed_projection() {
        let source = syn::parse_quote! {
            struct Query;
            relation R(value: i32);
            relation answer(value: i32);
            answer(x + 1) :- R(x);
            relational {
                rows = rename R { value -> x };
                output rows as answer(x + 1);
            }
            indexes {}
        };
        rejects(
            &source,
            &options(),
            "Rust expression outputs are not admitted",
        );
    }
}
