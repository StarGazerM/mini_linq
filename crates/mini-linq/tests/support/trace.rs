//! Shared acceptance for real proc-macro boundary captures.
pub fn verify(
    expected_ra_source: Option<&str>,
    expected_ix_source: &str,
    expected_selection_source: &str,
    expected_rust_source: &str,
) {
    use quote::ToTokens;
    use std::fs;
    let trace = std::path::PathBuf::from(
        std::env::var_os("CQ_REALIZATION_TRACE")
            .expect("set CQ_REALIZATION_TRACE to the current trace artifact"),
    );
    assert!(
        trace.join("verification.txt").is_file(),
        "trace must finish its source-identity verification"
    );
    let mut calls = Vec::new();
    for process in fs::read_dir(trace.join("raw")).unwrap() {
        for file in fs::read_dir(process.unwrap().path()).unwrap() {
            let file = file.unwrap().path();
            if file.to_string_lossy().ends_with(".output.tokens") {
                calls.push(file);
            }
        }
    }
    calls.sort();
    let output = |suffix: &str| {
        let matches: Vec<_> = calls
            .iter()
            .filter(|p| p.file_name().unwrap().to_string_lossy().ends_with(suffix))
            .collect();
        assert_eq!(matches.len(), 1, "expected one captured {suffix}");
        fs::read_to_string(matches[0]).unwrap()
    };
    if let Some(source) = expected_ra_source {
        let planned: syn::ExprMacro =
            syn::parse_str(&output("-cq_to_ra_written_order.output.tokens")).unwrap();
        let actual: cq_ir::relational_plan::Module = syn::parse2(planned.mac.tokens).unwrap();
        let expected: cq_ir::relational_plan::Module = syn::parse_str(source).unwrap();
        assert_eq!(actual, expected);
    }
    let indexed_call: syn::ExprMacro =
        syn::parse_str(&output("-index_demands.output.tokens")).unwrap();
    let indexed: cq_ir::index_requirements::Module = syn::parse2(indexed_call.mac.tokens).unwrap();
    let expected_ix: cq_ir::index_requirements::Module =
        syn::parse_str(expected_ix_source).unwrap();
    assert_eq!(indexed, expected_ix);
    let selected_call: syn::ExprMacro =
        syn::parse_str(&output("-given_physical_bindings.output.tokens")).unwrap();
    let selected: cq_ir::physical::Module = syn::parse2(selected_call.mac.tokens).unwrap();
    let expected: cq_ir::physical::Module = syn::parse_str(&format!(
        "{}\n{}",
        expected_ix_source, expected_selection_source
    ))
    .unwrap();
    assert_eq!(
        selected.to_token_stream().to_string(),
        expected.to_token_stream().to_string()
    );

    // Every realization request must carry the same frozen Physical program.
    for call in &calls {
        if !call
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-resume_physical.output.tokens")
        {
            continue;
        }
        let text = fs::read_to_string(call).unwrap();
        if let Ok(call) = syn::parse_str::<syn::ExprMacro>(&text) {
            let request: cq_ir::provider_protocol::Request = syn::parse2(call.mac.tokens).unwrap();
            assert_eq!(
                request.plan.to_token_stream().to_string(),
                expected.to_token_stream().to_string()
            );
        }
    }
    let realized: Vec<syn::Expr> = calls
        .iter()
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-resume_physical.output.tokens")
        })
        .filter_map(|p| {
            let expression: syn::Expr = syn::parse_str(&fs::read_to_string(p).unwrap()).unwrap();
            matches!(expression, syn::Expr::Block(_)).then_some(expression)
        })
        .collect();
    assert_eq!(realized.len(), 1, "one completed Physical realization");
    let expected: syn::Expr = syn::parse_str(expected_rust_source).unwrap();
    // syn leaves macro bodies opaque. Parse the actual Access payload as well:
    // its separate ':'/'-' grammar tokens print with different punctuation
    // spacing from authored ':-', despite being the same parsed language.
    let normalize = |mut expression: syn::Expr| {
        let syn::Expr::Block(block) = &mut expression else {
            panic!("expected host block")
        };
        let call = match block.block.stmts.last_mut() {
            Some(syn::Stmt::Expr(syn::Expr::Macro(call), None)) => &mut call.mac,
            Some(syn::Stmt::Macro(call)) if call.semi_token.is_none() => &mut call.mac,
            _ => panic!("expected final Access quotation"),
        };
        let access: cq_ir::rust_access_plan::Plan = syn::parse2(call.tokens.clone()).unwrap();
        call.tokens = access.to_token_stream();
        expression.to_token_stream().to_string()
    };
    assert_eq!(normalize(realized[0].clone()), normalize(expected));
}

/// Check the retained pull boundary independently of final iterator code.
#[allow(dead_code)] // Shared by eager trace tests, which do not use Pipeline.
pub fn verify_pipeline(expected_pipeline: &str) {
    let trace = std::path::PathBuf::from(
        std::env::var_os("CQ_REALIZATION_TRACE").expect("set CQ_REALIZATION_TRACE"),
    );
    let mut found = Vec::new();
    for process in std::fs::read_dir(trace.join("raw")).unwrap() {
        for file in std::fs::read_dir(process.unwrap().path()).unwrap() {
            let file = file.unwrap().path();
            if file
                .to_string_lossy()
                .ends_with("-access_to_distinct_iter.output.tokens")
            {
                found.push(file);
            }
        }
    }
    assert_eq!(found.len(), 1, "one Access-to-Pipeline expansion");
    let call: syn::ExprMacro =
        syn::parse_str(&std::fs::read_to_string(&found[0]).unwrap()).unwrap();
    let actual: cq_ir::iterator_pipeline::Pipeline = syn::parse2(call.mac.tokens).unwrap();
    let expected: cq_ir::iterator_pipeline::Pipeline = syn::parse_str(expected_pipeline).unwrap();
    cq_ir::iterator_pipeline::contract::check(&actual).unwrap();
    assert_eq!(actual, expected);
}
