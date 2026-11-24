//! Named pipeline adapters for checked GPU Datalog realization.
use proc_macro::TokenStream;

#[proc_macro]
pub fn device_storage(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(
        input.into(),
        cq_gpu_datalog_core::storage::compile_with,
    )
    .into()
}

/// `print_rounds(frontier("rule@delta", ...), manual("rule@delta" => path, ...), observe(...))`.
/// Each section is optional. `frontier` selects the generated GPU frontier plan
/// and cannot replace a manual version. Observers may accompany either.
#[proc_macro]
pub fn print_rounds(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |source, args| {
        cq_gpu_datalog_core::print::compile_with(source, &hooks(args)?)
    })
    .into()
}

fn hooks(args: proc_macro2::TokenStream) -> syn::Result<cq_gpu_datalog_core::print::HoleHooks> {
    use syn::parse::{ParseStream, Parser};
    let parse = |input: ParseStream| {
        let mut hooks = cq_gpu_datalog_core::print::HoleHooks::default();
        while !input.is_empty() {
            let name: syn::Ident = input.parse()?;
            if name != "frontier" && name != "manual" && name != "observe" {
                return Err(syn::Error::new_spanned(name, "expected frontier(...), manual(...) or observe(...)"));
            }
            let content;
            syn::parenthesized!(content in input);
            while !content.is_empty() {
                let label: syn::LitStr = content.parse()?;
                let key = label.value();
                let duplicate = if name == "frontier" {
                    !hooks.frontier.insert(key)
                } else {
                    content.parse::<syn::Token![=>]>()?;
                    let path = content.parse()?;
                    let entries = if name == "manual" { &mut hooks.manual } else { &mut hooks.observe };
                    entries.insert(key, path).is_some()
                };
                if duplicate {
                    let message = format!("duplicate {name} label `{}`", label.value());
                    return Err(syn::Error::new_spanned(label, message));
                }
                if !content.is_empty() {
                    content.parse::<syn::Token![,]>()?;
                }
            }
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }
        Ok(hooks)
    };
    parse.parse2(args)
}

#[proc_macro]
pub fn bind_inputs(input: TokenStream) -> TokenStream {
    cq_stage::Structures::<cq_lang_structures::physical::Program, cq_lang_structures::Program>::rewrite(
        input.into(),
        |physical: &cq_lang_structures::physical::Program, source: &cq_lang_structures::Program, args| {
            let target = cq_gpu_datalog_core::bind_inputs(physical, source, args)?;
            Ok((physical.clone(), target))
        },
    )
    .into()
}

#[proc_macro]
pub fn bind_round(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(
        input.into(),
        cq_gpu_datalog_core::bindings::compile_with,
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::hooks;

    #[test]
    fn frontier_labels_are_bare_strings_and_can_be_observed() {
        let parsed = hooks(r#"observe("A@0" => watch), frontier("A@0", "B@seed",)"#.parse().unwrap()).unwrap();
        assert_eq!(parsed.frontier.len(), 2);
        assert!(parsed.frontier.contains("A@0") && parsed.observe.contains_key("A@0"));
        assert!(parsed.frontier.contains("B@seed"));
        assert!(hooks(r#"frontier("A@0" => watch)"#.parse().unwrap()).is_err());
    }

    #[test]
    fn rejects_duplicate_frontier_labels_across_sections() {
        let error = hooks(r#"frontier("A@0"), frontier("A@0")"#.parse().unwrap()).err().unwrap();
        assert!(error.to_string().contains("duplicate frontier label `A@0`"));
    }
}
