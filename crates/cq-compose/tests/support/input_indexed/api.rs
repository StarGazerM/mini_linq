use super::*;
pub(super) fn assert_staged_api(
    file: &syn::File,
    source: &index_requirements::Module,
    fixture: &PhysicalFixture,
) {
    let storage_name = format!("{}Storage", fixture.program_name);
    assert_eq!(
        file.items.len(),
        4,
        "{} must emit only Program, Storage, and their two inherent impls",
        fixture.program_name,
    );
    let program = exactly_one_struct(file, fixture.program_name);
    assert_eq!(program.vis, source.relational_plan.program.visibility);
    assert!(matches!(program.fields, syn::Fields::Unit));
    let storage = exactly_one_struct(file, &storage_name);
    assert_eq!(storage.vis, source.relational_plan.program.visibility);
    assert_storage_fields_are_private(storage, fixture.program_name);

    let program_methods = inherent_methods(file, fixture.program_name);
    assert_eq!(
        program_methods.len(),
        2,
        "{} must expose exactly `build` and `run`",
        fixture.program_name,
    );
    let build = exactly_one_method(&program_methods, "build", fixture.program_name);
    let run = exactly_one_method(&program_methods, "run", fixture.program_name);
    assert_eq!(build.vis, source.relational_plan.program.visibility);
    assert_eq!(run.vis, source.relational_plan.program.visibility);

    let storage_methods = inherent_methods(file, &storage_name);
    assert_eq!(
        storage_methods.len(),
        2,
        "{storage_name} must define exactly `query` and `materialize`",
    );
    let query = exactly_one_method(&storage_methods, "query", &storage_name);
    let materialize = exactly_one_method(&storage_methods, "materialize", &storage_name);

    let output_type = result_row_type(source);
    let expected_materialized_output: syn::ReturnType = syn::parse2(quote!(
        -> ::std::vec::Vec<#output_type>
    ))
    .unwrap();
    assert_eq!(run.sig.output, expected_materialized_output);
    assert_eq!(materialize.sig.output, expected_materialized_output);

    let expected_storage_output: syn::ReturnType =
        syn::parse_str(&format!("-> {storage_name}")).unwrap();
    assert_eq!(build.sig.output, expected_storage_output);

    assert_no_method_generics(build, fixture.program_name);
    assert_no_method_generics(run, fixture.program_name);
    assert_no_method_generics(query, &storage_name);
    assert_no_method_generics(materialize, &storage_name);

    assert_normalized_inputs(build, source, fixture);
    assert_indexes_derive_from_normalized_inputs(build, source, fixture);
    assert_full_key_indexes_supply_scans_without_row_copies(build, source, fixture.program_name);
    assert_normalized_relations_move_into_their_final_consumer(build, source, fixture.program_name);
    assert_build_constructs_storage(build, &storage_name, fixture.program_name);
    assert_run_delegates_to_build(run, source, fixture.program_name);
    assert_query_api(
        query,
        &output_type,
        &source.relational_plan.program.visibility,
        fixture.program_name,
    );
    assert_materialize_api(
        materialize,
        &source.relational_plan.program.visibility,
        fixture.program_name,
    );
}

pub(super) fn assert_storage_fields_are_private(storage: &syn::ItemStruct, program_name: &str) {
    let syn::Fields::Named(fields) = &storage.fields else {
        panic!("{program_name}Storage must be a named-field storage struct")
    };
    assert!(
        !fields.named.is_empty(),
        "{program_name}Storage must own at least one concrete storage object",
    );
    for field in &fields.named {
        assert!(
            matches!(field.vis, syn::Visibility::Inherited),
            "{program_name}Storage fields must remain private",
        );
    }
}

pub(super) fn assert_no_method_generics(method: &syn::ImplItemFn, owner: &str) {
    assert!(
        method.sig.generics.params.is_empty() && method.sig.generics.where_clause.is_none(),
        "{owner}::{} must not introduce named or const generic parameters",
        method.sig.ident,
    );
    assert!(
        method.sig.asyncness.is_none(),
        "{owner}::{} must remain synchronous: its in-memory sources never await",
        method.sig.ident,
    );
}

pub(super) fn assert_build_constructs_storage(
    build: &syn::ImplItemFn,
    storage_name: &str,
    program_name: &str,
) {
    let Some(syn::Stmt::Expr(syn::Expr::Struct(construction), None)) = build.block.stmts.last()
    else {
        panic!("{program_name}::build must finish with a {storage_name} struct expression")
    };
    assert!(
        construction.qself.is_none()
            && construction.path.segments.len() == 1
            && construction.path.segments[0].ident == storage_name,
        "{program_name}::build must construct {storage_name} directly",
    );
    assert!(
        construction.rest.is_none(),
        "{program_name}::build must initialize every storage field explicitly",
    );
}

pub(super) fn assert_run_delegates_to_build(
    run: &syn::ImplItemFn,
    source: &index_requirements::Module,
    program_name: &str,
) {
    let inputs = (0..source.relational_plan.program.inputs().count())
        .map(|number| format_ident!("input{number}", span = Span::call_site()))
        .collect::<Vec<_>>();
    let expected: syn::Block = syn::parse2(quote!({
        Self::build(#(#inputs),*).materialize()
    }))
    .unwrap();
    assert_eq!(
        run.block, expected,
        "{program_name}::run must be only the eager convenience path `build(...).materialize()`",
    );
}

pub(super) fn assert_query_api(
    query: &syn::ImplItemFn,
    output_type: &syn::Type,
    expected_visibility: &syn::Visibility,
    program_name: &str,
) {
    assert_eq!(query.vis, *expected_visibility);
    assert_shared_self_receiver(query, program_name);
    assert_iterator_output(query, output_type, program_name);
    assert!(
        matches!(
            query.block.stmts.as_slice(),
            [syn::Stmt::Macro(statement)]
                if statement.semi_token.is_none()
                    && is_absolute_mini_linq_pull(&statement.mac.path)
        ),
        "{program_name}Storage::query must be exactly one `::mini_linq::pull!` expression",
    );
}

pub(super) fn assert_materialize_api(
    materialize: &syn::ImplItemFn,
    expected_visibility: &syn::Visibility,
    program_name: &str,
) {
    assert_eq!(materialize.vis, *expected_visibility);
    assert_shared_self_receiver(materialize, program_name);

    let expected: syn::Block = syn::parse2(quote!({
        let mut result = self.query().collect::<::std::vec::Vec<_>>();
        result.sort_unstable();
        result
    }))
    .unwrap();
    assert_eq!(
        materialize.block, expected,
        "{program_name}Storage::materialize must drain the unique query cursor and then sort only",
    );
}

pub(super) fn assert_shared_self_receiver(method: &syn::ImplItemFn, owner: &str) {
    assert_eq!(
        method.sig.inputs.len(),
        1,
        "{owner}::{} must take only `&self`",
        method.sig.ident,
    );
    assert!(
        matches!(
            method.sig.inputs.first(),
            Some(syn::FnArg::Receiver(receiver))
                if receiver.reference.is_some()
                    && receiver.mutability.is_none()
                    && receiver.colon_token.is_none()
        ),
        "{owner}::{} must take shared `&self`",
        method.sig.ident,
    );
}

pub(super) fn assert_iterator_output(
    method: &syn::ImplItemFn,
    output_type: &syn::Type,
    owner: &str,
) {
    let expected: syn::ReturnType = syn::parse2(quote!(
        -> impl ::std::iter::Iterator<
            Item = #output_type,
        > + '_
    ))
    .unwrap();
    assert_eq!(
        method.sig.output, expected,
        "{owner}::{} must return one lazy iterator for the whole query",
        method.sig.ident,
    );
}
