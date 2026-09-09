use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use cq_compiler::{
    compile_cq, compile_index_requirements, compile_iterator_pipeline, compile_pull,
    compile_relational_plan,
};
use cq_ir::{cq, index_requirements, iterator_pipeline, relational_plan, rust_access_plan};
use proc_macro2::Span;
use quote::{ToTokens, format_ident, quote};
use syn::visit::Visit;
use syn::visit_mut::{self, VisitMut};

use super::fixtures::{self, LoweringFixture, PhysicalFixture};

pub fn assert_cq_to_relational_plan(fixture: LoweringFixture) {
    let source: cq::Module = syn::parse2(fixture.source).unwrap();
    cq::contract::check(&source).unwrap();
    let actual_logical = compile_cq(&source).unwrap();
    relational_plan::contract::check(&actual_logical).unwrap();
    let expected: index_requirements::Module = syn::parse2(fixture.expected).unwrap();
    index_requirements::contract::check(&expected).unwrap();
    assert_eq!(actual_logical, expected.relational_plan);
}

pub fn assert_relational_plan_to_index_requirements(fixture: LoweringFixture) {
    let expected: index_requirements::Module = syn::parse2(fixture.expected).unwrap();
    index_requirements::contract::check(&expected).unwrap();
    let actual = compile_relational_plan(&expected.relational_plan).unwrap();
    assert_eq!(actual, expected);
}

pub fn assert_index_requirements_to_staged_rust(fixture: PhysicalFixture) {
    let source: index_requirements::Module = syn::parse2(fixture.source.clone()).unwrap();
    index_requirements::contract::check(&source).unwrap();
    let file = compile_index_requirements(&source).unwrap();
    let text = file.to_token_stream().to_string();

    assert_eq!(source.relational_plan.program.name, fixture.program_name);
    assert_staged_api(&file, &source, &fixture);
    assert_unique_pull_plan(&file, &fixture);

    for required in fixture.required_rust {
        assert!(
            text.contains(required),
            "{} is missing staged-Rust fragment: {required}",
            fixture.program_name,
        );
    }

    compile_and_run_staged_rust(file, &fixture);
}

pub fn assert_missing_required_index_rejected(tokens: proc_macro2::TokenStream) {
    let source: index_requirements::Module = syn::parse2(tokens.clone()).unwrap();
    index_requirements::contract::check(&source).unwrap();

    let error = compile_index_requirements(&source).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("Edge"));
    assert!(message.contains('0'));
}

fn assert_unique_pull_plan(file: &syn::File, fixture: &PhysicalFixture) {
    let mut collector = PullMacroCollector::default();
    collector.visit_file(file);
    assert_eq!(
        collector.macros.len(),
        1,
        "{} must contain exactly one macro named `pull`",
        fixture.program_name,
    );

    let pull = collector.macros[0];
    assert!(
        is_absolute_mini_linq_pull(&pull.path),
        "{} must invoke the exact path `::mini_linq::pull!`",
        fixture.program_name,
    );

    let actual: rust_access_plan::Plan = syn::parse2(pull.tokens.clone()).unwrap();
    rust_access_plan::contract::check(&actual).unwrap();
    assert_query_plan_borrows_stored_tuples(&actual, fixture.program_name);
    let expected: rust_access_plan::Plan = syn::parse2(fixture.rust_access_plan.clone()).unwrap();
    assert_eq!(
        actual, expected,
        "{} must emit the exact occurrence-annotated Rust access plan",
        fixture.program_name,
    );

    let actual_pipeline = compile_pull(&actual);
    iterator_pipeline::contract::check(&actual_pipeline).unwrap();
    let expected_pipeline: iterator_pipeline::Pipeline =
        syn::parse2(fixture.iterator_pipeline.clone()).unwrap();
    iterator_pipeline::contract::check(&expected_pipeline).unwrap();
    assert_eq!(
        actual_pipeline, expected_pipeline,
        "{} must expose every binary iterator boundary and intermediate binding",
        fixture.program_name,
    );

    let _: syn::Expr = compile_iterator_pipeline(&actual_pipeline);
}

fn assert_query_plan_borrows_stored_tuples(plan: &rust_access_plan::Plan, program_name: &str) {
    let mut copies = QueryTupleCopyAdapters::default();
    copies.visit_expr(&plan.head.output);
    for clause in &plan.body {
        match &clause.access {
            rust_access_plan::RustAccess::For { source, .. } => copies.visit_expr(source),
            rust_access_plan::RustAccess::If { condition, .. } => copies.visit_expr(condition),
        }
    }

    assert_eq!(
        copies.count, 0,
        "{program_name}'s query plan must borrow scans and index payload tuples; `.copied()`/`.cloned()` adapters belong only to eager physical construction",
    );
}

#[derive(Default)]
struct QueryTupleCopyAdapters {
    count: usize,
}

impl<'ast> Visit<'ast> for QueryTupleCopyAdapters {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "copied" || call.method == "cloned" {
            self.count += 1;
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

pub fn assert_pull_pipeline_lowers_to_iterator_expression(source: proc_macro2::TokenStream) {
    let source: rust_access_plan::Plan = syn::parse2(source).unwrap();
    rust_access_plan::contract::check(&source).unwrap();
    let pipeline = compile_pull(&source);
    iterator_pipeline::contract::check(&pipeline).unwrap();
    let _: syn::Expr = compile_iterator_pipeline(&pipeline);
}

fn assert_staged_api(
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

fn assert_storage_fields_are_private(storage: &syn::ItemStruct, program_name: &str) {
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

fn assert_no_method_generics(method: &syn::ImplItemFn, owner: &str) {
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

fn assert_build_constructs_storage(
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

fn assert_run_delegates_to_build(
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

fn assert_query_api(
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

fn assert_materialize_api(
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

fn assert_shared_self_receiver(method: &syn::ImplItemFn, owner: &str) {
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

fn assert_iterator_output(method: &syn::ImplItemFn, output_type: &syn::Type, owner: &str) {
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

fn assert_indexes_derive_from_normalized_inputs(
    build: &syn::ImplItemFn,
    source: &index_requirements::Module,
    fixture: &PhysicalFixture,
) {
    for (index_number, index) in source.indexes.iter().enumerate() {
        let index_name = format!("index{index_number}");
        let matching_locals = build
            .block
            .stmts
            .iter()
            .enumerate()
            .filter_map(|(position, statement)| match statement {
                syn::Stmt::Local(local)
                    if local_binding_name(statement).as_deref() == Some(&index_name) =>
                {
                    Some((position, local))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            matching_locals.len(),
            1,
            "{} must build written requirement {index_number} exactly once as {index_name}",
            fixture.program_name,
        );

        let relation_number = source
            .relational_plan
            .program
            .inputs()
            .position(|input| {
                cq_ir::symbol_name(&input.name) == cq_ir::symbol_name(&index.relation)
            })
            .expect("well-formed index relation must name a declared input");
        let normalized_relation = format!("relation{relation_number}");
        let (local_position, local) = matching_locals[0];
        let initializer = local.init.as_ref().unwrap_or_else(|| {
            panic!(
                "{}.build must initialize {index_name}",
                fixture.program_name,
            )
        });
        let initializer_uses_relation =
            expression_identifier_occurrences(&initializer.expr, &normalized_relation) >= 1;
        let later_builder_uses_both =
            build.block.stmts[local_position + 1..]
                .iter()
                .any(|statement| {
                    statement_identifier_occurrences(statement, &normalized_relation) >= 1
                        && statement_identifier_occurrences(statement, &index_name) >= 1
                });
        assert!(
            initializer_uses_relation || later_builder_uses_both,
            "{}.build must derive {index_name} from {normalized_relation}",
            fixture.program_name,
        );

        if index.key_columns.len()
            == source
                .relational_plan
                .program
                .inputs()
                .nth(relation_number)
                .unwrap()
                .arity()
        {
            let normalized_ident =
                format_ident!("relation{relation_number}", span = Span::call_site());
            let expected: syn::Expr = syn::parse2(quote!(#normalized_ident)).unwrap();
            assert_eq!(
                *initializer.expr, expected,
                "{}.build must move {normalized_relation} directly into full-key {index_name} without collecting a duplicate HashSet",
                fixture.program_name,
            );
        }
    }
}

fn assert_full_key_indexes_supply_scans_without_row_copies(
    build: &syn::ImplItemFn,
    source: &index_requirements::Module,
    program_name: &str,
) {
    let mut bound = Vec::<String>::new();
    let mut coalesced_inputs = Vec::<usize>::new();

    for item in &source.relational_plan.program.query.body {
        let atom = item.positive_atom().expect("positive R1/reference fixture");
        let is_empty_key_scan = atom
            .variables()
            .all(|variable| !bound.contains(&cq_ir::symbol_name(variable)));
        let relation_name = cq_ir::symbol_name(&atom.relation);
        let input_number = source
            .relational_plan
            .program
            .inputs()
            .position(|input| cq_ir::symbol_name(&input.name) == relation_name)
            .expect("well-formed body relation must name a declared input");
        let arity = source
            .relational_plan
            .program
            .inputs()
            .nth(input_number)
            .unwrap()
            .arity();
        let has_full_key_index = source.indexes.iter().any(|index| {
            cq_ir::symbol_name(&index.relation) == relation_name && index.key_columns.len() == arity
        });

        if is_empty_key_scan && has_full_key_index && !coalesced_inputs.contains(&input_number) {
            coalesced_inputs.push(input_number);
        }
        for variable in atom.variables() {
            let variable = cq_ir::symbol_name(variable);
            if !bound.contains(&variable) {
                bound.push(variable);
            }
        }
    }

    for input_number in coalesced_inputs {
        let relation_name = format!("relation{input_number}");
        let copied_row_builders = build
            .block
            .stmts
            .iter()
            .filter(|statement| {
                local_binding_name(statement).is_some_and(|name| name.starts_with("rows"))
                    && statement_identifier_occurrences(statement, &relation_name) > 0
            })
            .count();
        assert_eq!(
            copied_row_builders, 0,
            "{program_name}.build must reuse the full-key HashSet to scan input{input_number}, not copy it into a rows Vec",
        );
    }
}

fn assert_normalized_relations_move_into_their_final_consumer(
    build: &syn::ImplItemFn,
    source: &index_requirements::Module,
    program_name: &str,
) {
    for relation_number in 0..source.relational_plan.program.inputs().count() {
        let relation_name = format!("relation{relation_number}");
        if !input_has_physical_consumer(source, relation_number) {
            assert_eq!(
                identifier_occurrences(&build.block, &relation_name),
                0,
                "{}.build must not normalize unused input{relation_number} as {relation_name}",
                program_name,
            );
            continue;
        }
        let uses = build
            .block
            .stmts
            .iter()
            .filter(|statement| local_binding_name(statement).as_deref() != Some(&relation_name))
            .filter(|statement| statement_identifier_occurrences(statement, &relation_name) > 0)
            .map(|statement| {
                (
                    classify_relation_use(statement, &relation_name),
                    statement.to_token_stream().to_string(),
                )
            })
            .collect::<Vec<_>>();

        assert!(
            !uses.is_empty(),
            "{}.build normalizes {relation_name}, so one physical builder must eventually take ownership of it",
            program_name,
        );

        let final_use = uses.len() - 1;
        for (use_number, (kind, statement)) in uses.iter().enumerate() {
            if use_number == final_use {
                assert!(
                    matches!(kind, RelationUse::IntoIterMove | RelationUse::DirectMove),
                    "{}.build must move {relation_name} into its final physical consumer; found `{statement}`",
                    program_name,
                );
            } else {
                assert_eq!(
                    *kind,
                    RelationUse::BorrowedCopiedRows,
                    "{}.build may only borrow {relation_name} before its final consuming builder; found `{statement}`",
                    program_name,
                );
            }
        }

        let consuming_uses = uses
            .iter()
            .filter(|(kind, _)| matches!(kind, RelationUse::IntoIterMove | RelationUse::DirectMove))
            .count();
        assert_eq!(
            consuming_uses, 1,
            "{}.build must have exactly one ownership-taking consumer of {relation_name}",
            program_name,
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelationUse {
    /// A non-final builder clones the selected tuple fields from a shared
    /// borrow because its index owns keys and payloads.
    BorrowedCopiedRows,
    /// `relationN.into_iter()` moves rows into a final collection builder.
    IntoIterMove,
    /// A bare `relationN` either moves the whole set or is the iterable of a
    /// consuming `for` loop.
    DirectMove,
}

fn classify_relation_use(statement: &syn::Stmt, relation_name: &str) -> RelationUse {
    let mut classifier = RelationUseClassifier {
        relation_name,
        borrowed_copied_rows: 0,
        into_iter_moves: 0,
        direct_moves: 0,
        unsupported_uses: 0,
    };
    classifier.visit_stmt(statement);

    match (
        classifier.borrowed_copied_rows,
        classifier.into_iter_moves,
        classifier.direct_moves,
        classifier.unsupported_uses,
    ) {
        (1, 0, 0, 0) => RelationUse::BorrowedCopiedRows,
        (0, 1, 0, 0) => RelationUse::IntoIterMove,
        (0, 0, 1, 0) => RelationUse::DirectMove,
        profile => panic!(
            "a physical builder must use {relation_name} exactly once as `.iter()`, `.into_iter()`, or a direct move; got ownership profile {profile:?} in `{}`",
            statement.to_token_stream(),
        ),
    }
}

struct RelationUseClassifier<'a> {
    relation_name: &'a str,
    borrowed_copied_rows: usize,
    into_iter_moves: usize,
    direct_moves: usize,
    unsupported_uses: usize,
}

impl<'ast> Visit<'ast> for RelationUseClassifier<'_> {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "iter"
            && call.args.is_empty()
            && expression_is_identifier(&call.receiver, self.relation_name)
        {
            self.borrowed_copied_rows += 1;
            for argument in &call.args {
                self.visit_expr(argument);
            }
            return;
        }

        if expression_is_identifier(&call.receiver, self.relation_name) {
            if call.method == "into_iter" {
                self.into_iter_moves += 1;
            } else {
                self.unsupported_uses += 1;
            }
            for argument in &call.args {
                self.visit_expr(argument);
            }
            return;
        }

        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path.qself.is_none()
            && path.path.segments.len() == 1
            && cq_ir::symbol_name(&path.path.segments[0].ident) == self.relation_name
        {
            self.direct_moves += 1;
            return;
        }
        syn::visit::visit_expr_path(self, path);
    }
}

fn expression_is_identifier(expression: &syn::Expr, name: &str) -> bool {
    matches!(
        unparenthesized_expression(expression),
        syn::Expr::Path(path)
            if path.qself.is_none()
                && path.path.segments.len() == 1
                && cq_ir::symbol_name(&path.path.segments[0].ident) == name
    )
}

fn unparenthesized_expression(mut expression: &syn::Expr) -> &syn::Expr {
    loop {
        expression = match expression {
            syn::Expr::Group(group) => &group.expr,
            syn::Expr::Paren(parenthesized) => &parenthesized.expr,
            _ => return expression,
        };
    }
}

fn assert_normalized_inputs(
    build: &syn::ImplItemFn,
    source: &index_requirements::Module,
    fixture: &PhysicalFixture,
) {
    for (input_index, input) in source.relational_plan.program.inputs().enumerate() {
        let input_name = format_ident!("input{input_index}", span = Span::call_site());
        let relation_name = format_ident!("relation{input_index}", span = Span::call_site());
        let relation_spelling = relation_name.to_string();
        let matching_locals = build
            .block
            .stmts
            .iter()
            .filter_map(|statement| match statement {
                syn::Stmt::Local(local)
                    if local_binding_name(statement).as_deref() == Some(&relation_spelling) =>
                {
                    Some(local)
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        if !input_has_physical_consumer(source, input_index) {
            assert!(
                matching_locals.is_empty(),
                "{}.build must not normalize unused input{input_index}",
                fixture.program_name,
            );
            let expected_discard: syn::Stmt = syn::parse2(quote!(let _ = #input_name;)).unwrap();
            assert_eq!(
                build
                    .block
                    .stmts
                    .iter()
                    .filter(|statement| **statement == expected_discard)
                    .count(),
                1,
                "{}.build must discard unused input{input_index} without iterating it",
                fixture.program_name,
            );
            continue;
        }

        assert_eq!(
            matching_locals.len(),
            1,
            "{} must normalize input{input_index} exactly once as relation{input_index}",
            fixture.program_name,
        );

        let local = matching_locals[0];
        let syn::Pat::Type(typed_pattern) = &local.pat else {
            panic!(
                "{}.build must give relation{input_index} an explicit HashSet row type",
                fixture.program_name,
            );
        };
        assert!(
            is_hash_set_row_type(&typed_pattern.ty, input),
            "{}.build must type relation{input_index} as HashSet of the declared row tuple with {} columns",
            fixture.program_name,
            input.arity(),
        );

        let initializer = local.init.as_ref().unwrap_or_else(|| {
            panic!(
                "{}.build must initialize relation{input_index}",
                fixture.program_name,
            )
        });
        let expected_initializer: syn::Expr =
            syn::parse2(quote!(#input_name.into_iter().collect())).unwrap();
        assert_eq!(
            *initializer.expr, expected_initializer,
            "{}.build must normalize input{input_index} with `input{input_index}.into_iter().collect()`",
            fixture.program_name,
        );

        assert_eq!(
            identifier_occurrences(&build.block, &input_name.to_string()),
            1,
            "{}.build must consume input{input_index} only in its normalization",
            fixture.program_name,
        );
    }

    for &input_index in fixture.reused_normalized_inputs {
        assert!(input_index < source.relational_plan.program.inputs().count());
        let relation_name = format!("relation{input_index}");
        assert!(
            identifier_occurrences(&build.block, &relation_name) >= 3,
            "{}.build must derive multiple physical structures from relation{input_index}",
            fixture.program_name,
        );

        let copied_builders = build
            .block
            .stmts
            .iter()
            .filter(|statement| local_binding_name(statement).as_deref() != Some(&relation_name))
            .filter(|statement| statement_identifier_occurrences(statement, &relation_name) > 0)
            .filter(|statement| {
                classify_relation_use(statement, &relation_name) == RelationUse::BorrowedCopiedRows
            })
            .count();
        assert!(
            copied_builders >= 1,
            "{}.build may and must copy normalized input{input_index} while constructing multiple owned physical structures; the no-copy rule starts at query traversal",
            fixture.program_name,
        );
    }
}

fn input_has_physical_consumer(source: &index_requirements::Module, input_index: usize) -> bool {
    let relation_name = cq_ir::symbol_name(
        &source
            .relational_plan
            .program
            .inputs()
            .nth(input_index)
            .unwrap()
            .name,
    );
    source
        .relational_plan
        .program
        .query
        .body
        .iter()
        .any(|item| match item {
            cq::BodyItem::Positive { atom } => cq_ir::symbol_name(&atom.relation) == relation_name,
            _ => false,
        })
        || source
            .indexes
            .iter()
            .any(|index| cq_ir::symbol_name(&index.relation) == relation_name)
}

fn is_hash_set_row_type(ty: &syn::Type, input: &cq::RelationDecl) -> bool {
    is_set_row_type(ty, "HashSet", input)
}

fn is_set_row_type(ty: &syn::Type, collection: &str, input: &cq::RelationDecl) -> bool {
    let syn::Type::Path(type_path) = ty else {
        return false;
    };
    let Some(segment) = type_path.path.segments.last() else {
        return false;
    };
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    let column_types = input.columns.iter().map(|column| &column.ty);
    let expected_row: syn::Type = syn::parse2(quote!((#(#column_types,)*))).unwrap();
    segment.ident == collection
        && arguments.args.len() == 1
        && matches!(arguments.args.first(), Some(syn::GenericArgument::Type(row)) if row == &expected_row)
}

fn result_row_type(source: &index_requirements::Module) -> syn::Type {
    let program = &source.relational_plan.program;
    let relations = program
        .inputs()
        .map(|input| (cq_ir::symbol_name(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let mut variables = BTreeMap::<String, syn::Type>::new();
    for item in &program.query.body {
        let atom = item.positive_atom().expect("positive R1/reference fixture");
        let input = relations
            .get(&cq_ir::symbol_name(&atom.relation))
            .expect("well-formed body relation is declared");
        for (variable, column) in atom.variables().zip(&input.columns) {
            variables
                .entry(cq_ir::symbol_name(variable))
                .or_insert_with(|| column.ty.clone());
        }
    }
    let output_types = program.query.head.variables().map(|variable| {
        variables
            .get(&cq_ir::symbol_name(variable))
            .expect("well-formed result variable is bound")
    });
    syn::parse2(quote!((#(#output_types,)*))).unwrap()
}

fn identifier_occurrences(node: &syn::Block, name: &str) -> usize {
    let mut counter = IdentifierCounter { name, count: 0 };
    counter.visit_block(node);
    counter.count
}

fn expression_identifier_occurrences(node: &syn::Expr, name: &str) -> usize {
    let mut counter = IdentifierCounter { name, count: 0 };
    counter.visit_expr(node);
    counter.count
}

fn statement_identifier_occurrences(node: &syn::Stmt, name: &str) -> usize {
    let mut counter = IdentifierCounter { name, count: 0 };
    counter.visit_stmt(node);
    counter.count
}

struct IdentifierCounter<'a> {
    name: &'a str,
    count: usize,
}

impl<'ast> Visit<'ast> for IdentifierCounter<'_> {
    fn visit_ident(&mut self, ident: &'ast syn::Ident) {
        if cq_ir::symbol_name(ident) == self.name {
            self.count += 1;
        }
    }
}

#[derive(Default)]
struct PullMacroCollector<'ast> {
    macros: Vec<&'ast syn::Macro>,
}

impl<'ast> Visit<'ast> for PullMacroCollector<'ast> {
    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        if invocation
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "pull")
        {
            self.macros.push(invocation);
        }
        syn::visit::visit_macro(self, invocation);
    }
}

fn is_absolute_mini_linq_pull(path: &syn::Path) -> bool {
    path.leading_colon.is_some()
        && path.segments.len() == 2
        && path.segments[0].ident == "mini_linq"
        && path.segments[1].ident == "pull"
}

fn is_absolute_mini_linq_iterator_pipeline(path: &syn::Path) -> bool {
    path.leading_colon.is_some()
        && path.segments.len() == 2
        && path.segments[0].ident == "mini_linq"
        && path.segments[1].ident == "iterator_pipeline"
}

fn exactly_one_struct<'a>(file: &'a syn::File, name: &str) -> &'a syn::ItemStruct {
    let matches = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item) if item.ident == name => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected exactly one struct `{name}`");
    matches[0]
}

fn inherent_methods<'a>(file: &'a syn::File, program_name: &str) -> Vec<&'a syn::ImplItemFn> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item)
                if item.trait_.is_none() && type_is_named(&item.self_ty, program_name) =>
            {
                Some(item)
            }
            _ => None,
        })
        .flat_map(|item| &item.items)
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method) => Some(method),
            _ => None,
        })
        .collect()
}

fn exactly_one_method<'a>(
    methods: &'a [&syn::ImplItemFn],
    name: &str,
    program_name: &str,
) -> &'a syn::ImplItemFn {
    let matches = methods
        .iter()
        .copied()
        .filter(|method| method.sig.ident == name)
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "{program_name} must define exactly one `{name}` method",
    );
    matches[0]
}

fn type_is_named(ty: &syn::Type, name: &str) -> bool {
    matches!(
        ty,
        syn::Type::Path(path)
            if path.qself.is_none()
                && path.path.segments.len() == 1
                && path.path.segments[0].ident == name
    )
}

fn local_binding_name(statement: &syn::Stmt) -> Option<String> {
    let syn::Stmt::Local(local) = statement else {
        return None;
    };
    pattern_identifier(&local.pat).map(ToString::to_string)
}

fn pattern_identifier(pattern: &syn::Pat) -> Option<&syn::Ident> {
    match pattern {
        syn::Pat::Ident(pattern) => Some(&pattern.ident),
        syn::Pat::Paren(pattern) => pattern_identifier(&pattern.pat),
        syn::Pat::Type(pattern) => pattern_identifier(&pattern.pat),
        _ => None,
    }
}

fn compile_and_run_staged_rust(mut file: syn::File, fixture: &PhysicalFixture) {
    let mut expander = IteratorPipelineExpander::default();
    expander.visit_file_mut(&mut file);
    if let Some(error) = expander.error {
        panic!(
            "{} emitted a Rust access plan that the supplied pull lowering rejected: {error}",
            fixture.program_name,
        );
    }
    assert_eq!(
        expander.pull_expansions, 1,
        "{} must lower one RustAccessPlan",
        fixture.program_name
    );
    assert_eq!(
        expander.pipeline_expansions, 1,
        "{} must lower the resulting iterator pipeline exactly once",
        fixture.program_name,
    );

    // Generated collection paths must not rely on the caller's prelude
    // bindings. Declared column types themselves intentionally remain exactly
    // the Rust types written by the caller.
    let shadows: syn::File = syn::parse2(quote! {
        #[allow(dead_code)]
        struct Vec;
    })
    .unwrap();
    file.items.splice(0..0, shadows.items);

    let runtime_assertions = &fixture.runtime_assertions;
    let main: syn::ItemFn = syn::parse2(quote! {
        fn main() {
            #runtime_assertions
        }
    })
    .unwrap();
    file.items.push(syn::Item::Fn(main));

    let temporary = TemporaryRustProgram::new(fixture.program_name);
    let source_path = temporary.directory.join("program.rs");
    let executable_path = temporary.directory.join("program");
    let source_text = file.to_token_stream().to_string();
    fs::write(&source_path, &source_text).unwrap_or_else(|error| {
        panic!(
            "failed to write the {} generated-Rust check to {}: {error}",
            fixture.program_name,
            source_path.display(),
        )
    });

    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let crate_name = format!("{}_generated", fixture.program_name.to_ascii_lowercase());
    let compilation = Command::new(&rustc)
        .arg("--edition=2024")
        .arg("--crate-name")
        .arg(&crate_name)
        .arg(&source_path)
        .arg("-o")
        .arg(&executable_path)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "failed to invoke {:?} for {}: {error}",
                rustc, fixture.program_name,
            )
        });
    assert!(
        compilation.status.success(),
        "{} did not compile as standalone Rust\nstdout:\n{}\nstderr:\n{}\ngenerated source:\n{}",
        fixture.program_name,
        String::from_utf8_lossy(&compilation.stdout),
        String::from_utf8_lossy(&compilation.stderr),
        source_text,
    );

    let execution = Command::new(&executable_path)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "failed to execute generated program {} for {}: {error}",
                executable_path.display(),
                fixture.program_name,
            )
        });
    assert!(
        execution.status.success(),
        "{} compiled but its runtime assertions failed\nstdout:\n{}\nstderr:\n{}\ngenerated source:\n{}",
        fixture.program_name,
        String::from_utf8_lossy(&execution.stdout),
        String::from_utf8_lossy(&execution.stderr),
        source_text,
    );
}

#[derive(Default)]
struct IteratorPipelineExpander {
    pull_expansions: usize,
    pipeline_expansions: usize,
    error: Option<syn::Error>,
}

impl VisitMut for IteratorPipelineExpander {
    fn visit_stmt_mut(&mut self, statement: &mut syn::Stmt) {
        let macro_invocation = match statement {
            syn::Stmt::Macro(statement) if statement.semi_token.is_none() => {
                Some(statement.mac.clone())
            }
            _ => None,
        };
        let Some(macro_invocation) = macro_invocation else {
            visit_mut::visit_stmt_mut(self, statement);
            return;
        };

        if is_absolute_mini_linq_pull(&macro_invocation.path) {
            self.pull_expansions += 1;
            let lowered =
                syn::parse2::<rust_access_plan::Plan>(macro_invocation.tokens).and_then(|plan| {
                    rust_access_plan::contract::check(&plan)?;
                    let target = compile_pull(&plan);
                    iterator_pipeline::contract::check(&target)?;
                    syn::parse2(quote! {
                        ::mini_linq::iterator_pipeline! { #target }
                    })
                });
            match lowered {
                Ok(lowered) => {
                    *statement = lowered;
                    self.visit_stmt_mut(statement);
                }
                Err(error) => self.error = Some(error),
            }
        } else if is_absolute_mini_linq_iterator_pipeline(&macro_invocation.path) {
            self.pipeline_expansions += 1;
            let lowered = syn::parse2::<iterator_pipeline::Pipeline>(macro_invocation.tokens)
                .and_then(|plan| {
                    iterator_pipeline::contract::check(&plan)?;
                    Ok(compile_iterator_pipeline(&plan))
                });
            match lowered {
                Ok(lowered) => *statement = syn::Stmt::Expr(lowered, None),
                Err(error) => self.error = Some(error),
            }
        } else {
            visit_mut::visit_stmt_mut(self, statement);
        }
    }
}

static NEXT_TEMPORARY_PROGRAM: AtomicU64 = AtomicU64::new(0);

struct TemporaryRustProgram {
    directory: PathBuf,
}

impl TemporaryRustProgram {
    fn new(label: &str) -> Self {
        loop {
            let nonce = NEXT_TEMPORARY_PROGRAM.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "mini-linq-generated-{}-{nonce}-{}",
                std::process::id(),
                label.to_ascii_lowercase(),
            ));
            match fs::create_dir(&directory) {
                Ok(()) => return Self { directory },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!(
                    "failed to create generated-Rust test directory {}: {error}",
                    directory.display(),
                ),
            }
        }
    }
}

impl Drop for TemporaryRustProgram {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

pub fn assert_semantically_invalid_sources_fail_local_contracts() {
    let invalid = fixtures::invalid_sources();

    let cq: cq::Module = syn::parse2(invalid.cq).unwrap();
    assert!(
        cq::contract::check(&cq).is_err(),
        "the parsed CQ fixture must fail its local semantic contract",
    );
    assert!(!cq::contract::well_formed(&cq));

    let indexes: index_requirements::Module = syn::parse2(invalid.index_requirements).unwrap();
    assert!(
        index_requirements::contract::check(&indexes).is_err(),
        "the parsed IndexRequirements fixture must fail its local semantic contract",
    );
    assert!(!index_requirements::contract::well_formed(&indexes));

    let access_plan: rust_access_plan::Plan = syn::parse2(invalid.rust_access_plan).unwrap();
    assert!(
        rust_access_plan::contract::check(&access_plan).is_err(),
        "the parsed RustAccessPlan fixture must fail its local semantic contract",
    );
    assert!(!rust_access_plan::contract::well_formed(&access_plan));
}

pub fn assert_malformed_sources_fail_direct_syn_parsing() {
    let cq = quote! { struct MissingProgramBody; };
    assert!(
        syn::parse2::<cq::Module>(cq).is_err(),
        "the malformed CQ fixture must fail direct syn parsing",
    );

    let indexes = quote! { indexes {} };
    assert!(
        syn::parse2::<index_requirements::Module>(indexes).is_err(),
        "the malformed IndexRequirements fixture must fail direct syn parsing",
    );

    let access_plan = quote! { answer(x) => ([x]) :- };
    assert!(
        syn::parse2::<rust_access_plan::Plan>(access_plan).is_err(),
        "the malformed RustAccessPlan fixture must fail direct syn parsing",
    );
}

#[cfg(debug_assertions)]
pub fn assert_pull_debug_precondition_rejects_invalid_typed_source() {
    let source: rust_access_plan::Plan =
        syn::parse2(fixtures::invalid_sources().rust_access_plan).unwrap();
    assert!(rust_access_plan::contract::check(&source).is_err());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = compile_pull(&source);
    }));
    assert!(
        result.is_err(),
        "compile_pull must enforce its semantic precondition when debug contracts are enabled",
    );
}

#[cfg(not(debug_assertions))]
pub fn assert_pull_debug_precondition_is_disabled() {
    let source: rust_access_plan::Plan =
        syn::parse2(fixtures::invalid_sources().rust_access_plan).unwrap();
    assert!(rust_access_plan::contract::check(&source).is_err());

    let _: iterator_pipeline::Pipeline = compile_pull(&source);
}

pub fn assert_fixtures_well_formed() {
    for fixture in fixtures::cq_to_relational_plan_and_indexes() {
        let source: cq::Module = syn::parse2(fixture.source).unwrap();
        cq::contract::check(&source).unwrap();
        let expected: index_requirements::Module = syn::parse2(fixture.expected).unwrap();
        index_requirements::contract::check(&expected).unwrap();
    }

    for fixture in fixtures::index_requirements_to_staged_rust() {
        let source: index_requirements::Module = syn::parse2(fixture.source).unwrap();
        index_requirements::contract::check(&source).unwrap();
        let access_plan: rust_access_plan::Plan =
            syn::parse2(fixture.rust_access_plan.clone()).unwrap();
        rust_access_plan::contract::check(&access_plan).unwrap();
        let pipeline: iterator_pipeline::Pipeline =
            syn::parse2(fixture.iterator_pipeline.clone()).unwrap();
        iterator_pipeline::contract::check(&pipeline).unwrap();
        assert_eq!(compile_pull(&access_plan), pipeline);
        let _: syn::Expr = compile_iterator_pipeline(&pipeline);
        assert_pull_pipeline_lowers_to_iterator_expression(fixture.rust_access_plan);
        let runtime_assertions = fixture.runtime_assertions;
        let _: syn::Block = syn::parse2(quote!({ #runtime_assertions })).unwrap();
    }
}
