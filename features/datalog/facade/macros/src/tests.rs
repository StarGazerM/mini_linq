use super::*;
use quote::quote;

fn source() -> proc_macro2::TokenStream {
    quote! {
        observe = (watch);
        struct Q;
        relation E(x:i32) = (input);
        relation R(x:i32);
        output R;
        R(*x) :- E(x);
    }
}

#[test]
fn entry_starts_surface_expansion_before_parsing_source() {
    let source = source();
    let expanded = expand(source.clone()).unwrap();
    let expected = DatalogConfiguration::default()
        .resolve(DatalogConfiguration::defaults())
        .unwrap()
        .start(source)
        .unwrap();
    assert_eq!(expanded.to_string(), expected.to_string());
    assert!(expanded.to_string().contains("surface_if_let !"));
    assert!(expanded.to_string().contains("observe = (watch)"));
}

#[test]
fn configured_surface_gets_opaque_args_and_remaining_datalog_queue() {
    let source = source();
    let expanded = expand(quote! {
        pipeline {
            surface = [custom_surface(opaque(tokens))];
            cq = pipeline { plan = planner(no_reduce); };
        };
        #source
    })
    .unwrap()
    .to_string();
    assert!(expanded.starts_with("custom_surface !"));
    assert!(expanded.contains("arguments { opaque (tokens) }"));
    assert!(expanded.contains("planner (no_reduce)"));
    assert!(expanded.contains("cq_datalog :: passes :: stratify"));
}

#[test]
fn direct_protocol_rejects_wrong_authored_stage_and_missing_terminal() {
    let source = source();
    let wrong_stage =
        cq_stage::pass::<cq_lang_datalog_strata::Program, cq_lang_datalog_incremental::Program>(
            quote!(pipeline {}; #source),
            |source, _| cq_datalog_incremental::compile(source),
        );
    assert!(wrong_stage.to_string().contains("compile_error"));
    let missing_terminal = cq_stage::pass::<
        cq_lang_datalog_source::Program,
        cq_lang_datalog_strata::Program,
    >(quote!(pipeline {}; #source), |source, _| {
        let target = cq_datalog_stratify::compile(source)?;
        cq_datalog_stratify::check(source, &target)?;
        Ok(target)
    });
    assert!(
        missing_terminal
            .to_string()
            .contains("pipeline ended before its Rust printer")
    );
}

#[test]
fn default_pass_arguments_are_not_silently_ignored() {
    assert!(no_arguments(quote!()).is_ok());
    assert!(no_arguments(quote!(unexpected)).is_err());
    assert!(expand(quote!(pipeline { plan = planner; } struct Q;)).is_err());
}

#[test]
fn fork_rejects_an_incompatible_cq_terminal_printer() {
    let source: cq_lang_datalog_source::Program = syn::parse2(source()).unwrap();
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let config: DatalogConfiguration = syn::parse_quote! { pipeline {
        cq = pipeline { print = incompatible::push; };
    } };
    let queue = config.resolve(DatalogConfiguration::defaults()).unwrap();
    let fork = queue
        .passes
        .iter()
        .find(|pass| pass.path.segments.last().unwrap().ident == "fork")
        .unwrap();
    assert!(cq_datalog_fork::compile_with(&storage, fork.arguments.clone()).is_err());
}
