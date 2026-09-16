use super::*;
pub(super) fn assert_indexes_derive_from_normalized_inputs(
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

pub(super) fn assert_full_key_indexes_supply_scans_without_row_copies(
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

pub(super) fn assert_normalized_relations_move_into_their_final_consumer(
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

pub(super) fn expression_is_identifier(expression: &syn::Expr, name: &str) -> bool {
    matches!(
        unparenthesized_expression(expression),
        syn::Expr::Path(path)
            if path.qself.is_none()
                && path.path.segments.len() == 1
                && cq_ir::symbol_name(&path.path.segments[0].ident) == name
    )
}

pub(super) fn unparenthesized_expression(mut expression: &syn::Expr) -> &syn::Expr {
    loop {
        expression = match expression {
            syn::Expr::Group(group) => &group.expr,
            syn::Expr::Paren(parenthesized) => &parenthesized.expr,
            _ => return expression,
        };
    }
}

pub(super) fn assert_normalized_inputs(
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

pub(super) fn input_has_physical_consumer(
    source: &index_requirements::Module,
    input_index: usize,
) -> bool {
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

pub(super) fn is_hash_set_row_type(ty: &syn::Type, input: &cq::RelationDecl) -> bool {
    is_set_row_type(ty, "HashSet", input)
}

pub(super) fn is_set_row_type(ty: &syn::Type, collection: &str, input: &cq::RelationDecl) -> bool {
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
