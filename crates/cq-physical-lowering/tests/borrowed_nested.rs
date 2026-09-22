//! Recovered borrowed-relation domain contract.
use cq_ir::index_requirements as ix;
use std::collections::BTreeMap;

#[cfg(test)]
mod tests {
    use super::*;
    use cq_relation_lowering::borrowed_nested::{Options, compile};

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
