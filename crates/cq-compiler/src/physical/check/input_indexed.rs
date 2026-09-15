//! The base-index route's storage and Access relation. Occurrences are recovered
//! from RA dataflow; storage statements and lookup leaves are checked separately.
use super::*;

pub(crate) fn check(source: &ir::Module, target: &syn::File) -> syn::Result<()> {
    ir::contract::check(source)?;
    let accepted = supported(source) && check_storage_and_access(source, target).is_some();
    if accepted {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            target,
            "base-index storage/access source-target relation violated",
        ))
    }
}

fn check_storage_and_access(source: &ir::Module, target: &syn::File) -> Option<()> {
    let module = &source.relational_plan;
    let inputs: Vec<_> = module.program.inputs().collect();
    let info = ra::check_in(module.program.inputs(), &module.plan).ok()?;
    let definitions: BTreeMap<_, _> = module
        .plan
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.result), &definition.operator))
        .collect();
    let ra::Operator::Project(project) =
        definitions.get(&symbol_name(&module.plan.output.input))?
    else {
        return None;
    };

    // Follow the selected dataflow backwards, independently of emission order.
    let mut occurrences = Vec::new();
    let mut current = &project.input;
    loop {
        match definitions.get(&symbol_name(current))? {
            ra::Operator::NaturalJoin(join) => {
                let ra::Operator::Rename(rename) = definitions.get(&symbol_name(&join.right))?
                else {
                    return None;
                };
                occurrences.push((rename, info.get(&symbol_name(&join.left))?.heading.clone()));
                current = &join.left;
            }
            ra::Operator::Rename(rename) => {
                occurrences.push((rename, BTreeSet::new()));
                break;
            }
            _ => return None,
        }
    }
    occurrences.reverse();
    let indexes: Vec<_> = source
        .indexes
        .iter()
        .map(|index| {
            let input = inputs
                .iter()
                .position(|input| symbol_name(&input.name) == symbol_name(&index.relation))?;
            Some((
                input,
                index
                    .key_columns
                    .iter()
                    .map(|column| column.index as usize)
                    .collect::<Vec<_>>(),
            ))
        })
        .collect::<Option<_>>()?;
    let full: BTreeMap<_, _> = indexes
        .iter()
        .enumerate()
        .filter(|(_, (input, keys))| keys.len() == inputs[*input].arity())
        .map(|(number, (input, _))| (*input, number))
        .collect();
    let storage = format_ident!("{}Storage", module.program.name);
    let query = method(target, &storage, "query")?;
    let [statement] = query.block.stmts.as_slice() else {
        return None;
    };
    let plan = match statement {
        Stmt::Expr(expression, None) => pull(expression)?,
        Stmt::Macro(mac) if mac.semi_token.is_none() => pull_macro(&mac.mac)?,
        _ => return None,
    };
    ac::contract::check(&plan).ok()?;
    let mut clauses = plan.body.iter();
    let mut rows = Vec::new();
    for (rename, bound) in occurrences {
        let input = inputs
            .iter()
            .position(|input| symbol_name(&input.name) == symbol_name(&rename.relation))?;
        let declaration = inputs[input];
        let variables: Vec<_> = declaration
            .columns
            .iter()
            .map(|column| {
                rename
                    .mapping
                    .iter()
                    .find(|mapping| symbol_name(&mapping.source) == symbol_name(&column.name))
                    .map(|mapping| mapping.target.clone())
            })
            .collect::<Option<_>>()?;
        let relation = &rename.relation;
        let clause = clauses.next()?;
        let cq::BodyItem::Positive { atom } = &clause.item else {
            return None;
        };
        if atom != &syn::parse_quote!(#relation(#(#variables),*)) {
            return None;
        }
        let key: Vec<_> = variables
            .iter()
            .enumerate()
            .filter_map(|(column, variable)| {
                bound.contains(&symbol_name(variable)).then_some(column)
            })
            .collect();
        if key.is_empty() {
            let store = if let Some(number) = full.get(&input) {
                format_ident!("index{number}")
            } else {
                let number = match rows.iter().position(|stored| *stored == input) {
                    Some(number) => number,
                    None => {
                        rows.push(input);
                        rows.len() - 1
                    }
                };
                format_ident!("rows{number}")
            };
            check_for(clause, &variables, &syn::parse_quote!(self.#store.iter()))?;
        } else {
            let number = indexes
                .iter()
                .position(|(stored, columns)| *stored == input && *columns == key)?;
            let index = format_ident!("index{number}");
            let key_variables: Vec<_> = key
                .iter()
                .map(|column| variables[*column].clone())
                .collect();
            let value = cloned_names(&key_variables);
            if key.len() == variables.len() {
                check_if(clause, &syn::parse_quote!(self.#index.contains(&#value)))?;
            } else {
                let payload: Vec<_> = variables
                    .iter()
                    .enumerate()
                    .filter(|(column, _)| !key.contains(column))
                    .map(|(_, variable)| variable.clone())
                    .collect();
                check_for(
                    clause,
                    &payload,
                    &syn::parse_quote!(self.#index.get(&#value).into_iter().flatten()),
                )?;
            }
        }
    }
    if clauses.next().is_some() || plan.head.atom != module.plan.output.head {
        return None;
    }
    equal(
        &plan.head.output,
        &cloned_names(
            &module
                .plan
                .output
                .head
                .variables()
                .cloned()
                .collect::<Vec<_>>(),
        ),
    )?;

    // Full-key sets own their normalized relation. Otherwise a row store owns
    // it, or the last written proper index does. All other consumers borrow.
    let moving: BTreeSet<_> = inputs
        .iter()
        .enumerate()
        .filter_map(|(input, _)| {
            full.get(&input).copied().or_else(|| {
                (!rows.contains(&input))
                    .then(|| indexes.iter().rposition(|(stored, _)| *stored == input))
                    .flatten()
            })
        })
        .collect();
    let build = method(target, &module.program.name, "build")?;
    let mut statements = build.block.stmts.iter();
    for (number, declaration) in inputs.iter().enumerate() {
        let input = format_ident!("input{number}");
        if rows.contains(&number) || indexes.iter().any(|(stored, _)| *stored == number) {
            let relation = format_ident!("relation{number}");
            let types: Vec<_> = declaration
                .columns
                .iter()
                .map(|column| &column.ty)
                .collect();
            expect_statement(
                &mut statements,
                &syn::parse_quote!(let #relation: ::std::collections::HashSet<(#(#types,)*)> = #input.into_iter().collect();),
            )?;
        } else {
            expect_statement(&mut statements, &syn::parse_quote!(let _ = #input;))?;
        }
    }
    for (number, (input, keys)) in indexes
        .iter()
        .enumerate()
        .filter(|(number, _)| !moving.contains(number))
    {
        check_index_builder(&mut statements, number, *input, inputs[*input], keys, false)?;
    }
    for (number, (input, keys)) in indexes
        .iter()
        .enumerate()
        .filter(|(number, (input, keys))| {
            moving.contains(number) && keys.len() < inputs[*input].arity()
        })
    {
        check_index_builder(&mut statements, number, *input, inputs[*input], keys, true)?;
    }
    for (number, input) in rows.iter().enumerate() {
        let row_store = format_ident!("rows{number}");
        let relation = format_ident!("relation{input}");
        let types: Vec<_> = inputs[*input]
            .columns
            .iter()
            .map(|column| &column.ty)
            .collect();
        expect_statement(
            &mut statements,
            &syn::parse_quote!(let #row_store: ::std::vec::Vec<(#(#types,)*)> = #relation.into_iter().collect();),
        )?;
    }
    for (number, (input, keys)) in indexes
        .iter()
        .enumerate()
        .filter(|(number, (input, keys))| {
            moving.contains(number) && keys.len() == inputs[*input].arity()
        })
    {
        check_index_builder(&mut statements, number, *input, inputs[*input], keys, true)?;
    }
    let fields: Vec<_> = (0..rows.len())
        .map(|number| format_ident!("rows{number}"))
        .chain((0..indexes.len()).map(|number| format_ident!("index{number}")))
        .collect();
    expect_statement(
        &mut statements,
        &Stmt::Expr(syn::parse_quote!(#storage { #(#fields),* }), None),
    )?;
    if statements.next().is_some() {
        return None;
    }
    Some(())
}

fn check_index_builder(
    statements: &mut std::slice::Iter<'_, Stmt>,
    number: usize,
    input: usize,
    declaration: &cq::RelationDecl,
    key: &[usize],
    moving: bool,
) -> Option<()> {
    let index = format_ident!("index{number}");
    let relation = format_ident!("relation{input}");
    let types: Vec<_> = declaration
        .columns
        .iter()
        .map(|column| &column.ty)
        .collect();
    if key.len() == declaration.arity() {
        if !moving {
            return None;
        }
        return expect_statement(
            statements,
            &syn::parse_quote!(let #index: ::std::collections::HashSet<(#(#types,)*)> = #relation;),
        );
    }
    let local = next_local(statements, &index)?;
    let key_types: Vec<_> = key.iter().map(|column| types[*column]).collect();
    let complement: Vec<_> = (0..types.len())
        .filter(|column| !key.contains(column))
        .collect();
    let payload_types: Vec<_> = complement.iter().map(|column| types[*column]).collect();
    let ty: syn::Type = syn::parse_quote!(::std::collections::HashMap<(#(#key_types,)*), ::std::vec::Vec<(#(#payload_types,)*)>>);
    let syn::Pat::Type(pattern) = &local.pat else {
        return None;
    };
    if !local.attrs.is_empty()
        || !pattern.attrs.is_empty()
        || pattern.pat.as_ref() != &syn::parse_quote!(#index)
        || pattern.ty.as_ref() != &ty
    {
        return None;
    }
    let Expr::Block(block) = initializer(local)? else {
        return None;
    };
    if block.label.is_some() || !block.attrs.is_empty() {
        return None;
    }
    let mut body = block.block.stmts.iter();
    expect_statement(
        &mut body,
        &syn::parse_quote!(let mut #index: #ty = ::std::collections::HashMap::new();),
    )?;
    let Stmt::Expr(Expr::ForLoop(loop_), None) = body.next()? else {
        return None;
    };
    let columns: Vec<_> = (0..types.len())
        .map(|column| format_ident!("index{number}_column{column}"))
        .collect();
    let iteration: Expr = if moving {
        syn::parse_quote!(#relation.into_iter())
    } else {
        syn::parse_quote!(#relation.iter())
    };
    if !loop_.attrs.is_empty()
        || loop_.label.is_some()
        || loop_.pat.as_ref() != &syn::parse_quote!((#(#columns,)*))
    {
        return None;
    }
    equal(&loop_.expr, &iteration)?;
    let field = |column: &usize| {
        let variable = &columns[*column];
        if moving {
            quote!(#variable)
        } else {
            quote!(::core::clone::Clone::clone(#variable))
        }
    };
    let keys = key.iter().map(field);
    let payload = complement.iter().map(field);
    if loop_.body.stmts
        != vec![syn::parse_quote!(#index.entry((#(#keys,)*)).or_default().push((#(#payload,)*));)]
    {
        return None;
    }
    expect_statement(&mut body, &Stmt::Expr(syn::parse_quote!(#index), None))?;
    body.next().is_none().then_some(())
}

pub(crate) fn supported(source: &ir::Module) -> bool {
    source.indexes.iter().all(|i| i.result_token.is_none())
        && crate::relational_to_indexes::check::supported_linear_plan(&source.relational_plan)
        && crate::relational_to_indexes::check::covers_required_indexes(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::visit_mut::{self, VisitMut};

    fn fixture() -> ir::Module {
        let source: cq::Module = syn::parse_quote! {
            struct BasicContract;
            relation R(a:i32,b:i32);
            relation S(a:i32,b:i32);
            relation T(a:i32,b:i32);
            relation Unused(a:i32,b:i32);
            answer(x,y,z) :- R(x,y), S(y,z), T(z,x);
        };
        let ra = crate::compile_cq(&source).unwrap();
        let mut indexed = crate::compile_relational_plan(&ra).unwrap();
        for extra in [
            syn::parse_quote!(R[0]),
            syn::parse_quote!(R[1]),
            syn::parse_quote!(S[1]),
        ] {
            indexed.indexes.push(extra);
        }
        indexed
    }

    fn replace_expression(target: &mut syn::File, original: Expr, replacement: Expr) {
        struct Replace {
            original: Expr,
            replacement: Expr,
            count: usize,
        }
        impl VisitMut for Replace {
            fn visit_expr_mut(&mut self, expression: &mut Expr) {
                if *expression == self.original {
                    *expression = self.replacement.clone();
                    self.count += 1;
                } else {
                    visit_mut::visit_expr_mut(self, expression);
                }
            }
        }
        let mut visitor = Replace {
            original,
            replacement,
            count: 0,
        };
        visitor.visit_file_mut(target);
        assert!(
            visitor.count > 0,
            "mutation did not reach a generated expression"
        );
    }

    fn mutate_access(target: &mut syn::File, change: impl FnOnce(&mut ac::Plan)) {
        for item in &mut target.items {
            let syn::Item::Impl(item) = item else {
                continue;
            };
            for method in &mut item.items {
                let syn::ImplItem::Fn(method) = method else {
                    continue;
                };
                if method.sig.ident != "query" {
                    continue;
                }
                let mac = match method.block.stmts.first_mut().unwrap() {
                    Stmt::Expr(Expr::Macro(mac), None) => &mut mac.mac,
                    Stmt::Macro(mac) => &mut mac.mac,
                    _ => panic!("query lost its Access region"),
                };
                let mut plan = pull_macro(mac).unwrap();
                change(&mut plan);
                mac.tokens = quote!(#plan);
                return;
            }
        }
        panic!("missing query method");
    }

    #[test]
    fn base_storage_contract_checks_keys_payloads_and_ownership_schedule() {
        let source = fixture();
        let target = crate::compile_index_requirements(&source).unwrap();
        check(&source, &target).unwrap();
        for (original, replacement) in [
            (
                syn::parse_quote!(input1.into_iter().collect()),
                syn::parse_quote!(input0.into_iter().collect()),
            ),
            (
                syn::parse_quote!(::core::clone::Clone::clone(index0_column0)),
                syn::parse_quote!(::core::clone::Clone::clone(index0_column1)),
            ),
            (
                syn::parse_quote!(::core::clone::Clone::clone(index0_column1)),
                syn::parse_quote!(::core::clone::Clone::clone(index0_column0)),
            ),
            (
                syn::parse_quote!(relation1.iter()),
                syn::parse_quote!(relation1.into_iter()),
            ),
            (
                syn::parse_quote!(relation1.into_iter()),
                syn::parse_quote!(relation1.iter()),
            ),
            (
                syn::parse_quote!(BasicContractStorage {
                    rows0,
                    index0,
                    index1,
                    index2,
                    index3,
                    index4
                }),
                syn::parse_quote!(BasicContractStorage {
                    rows0,
                    index0: index4,
                    index1,
                    index2,
                    index3,
                    index4
                }),
            ),
        ] {
            let mut invalid = target.clone();
            replace_expression(&mut invalid, original, replacement);
            assert!(check(&source, &invalid).is_err());
        }
        let mut full = source;
        full.indexes.push(syn::parse_quote!(R[0,1]));
        let target = crate::compile_index_requirements(&full).unwrap();
        check(&full, &target).unwrap();
    }

    #[test]
    fn base_access_contract_rejects_wrong_stores_keys_and_missing_occurrences() {
        let source = fixture();
        let target = crate::compile_index_requirements(&source).unwrap();
        for mutation in 0..4 {
            let mut invalid = target.clone();
            mutate_access(&mut invalid, |plan| match mutation {
                0 => {
                    let ac::RustAccess::For { source, .. } = &mut plan.body[0].access else {
                        unreachable!()
                    };
                    *source = syn::parse_quote!(self.index1.iter());
                }
                1 => {
                    let ac::RustAccess::For { source, .. } = &mut plan.body[1].access else {
                        unreachable!()
                    };
                    *source = syn::parse_quote!(
                        self.index0
                            .get(&(::core::clone::Clone::clone(x),))
                            .into_iter()
                            .flatten()
                    );
                }
                2 => {
                    plan.body = plan
                        .body
                        .iter()
                        .take(plan.body.len() - 1)
                        .cloned()
                        .collect();
                }
                _ => {
                    plan.head.output = syn::parse_quote!((
                        ::core::clone::Clone::clone(x),
                        ::core::clone::Clone::clone(x),
                        ::core::clone::Clone::clone(z),
                    ));
                }
            });
            ac::contract::check(&pull_from_file(&invalid)).unwrap();
            assert!(check(&source, &invalid).is_err());
        }
    }

    fn pull_from_file(target: &syn::File) -> ac::Plan {
        let query = method(target, &syn::parse_quote!(BasicContractStorage), "query").unwrap();
        match query.block.stmts.first().unwrap() {
            Stmt::Expr(expression, None) => pull(expression).unwrap(),
            Stmt::Macro(mac) => pull_macro(&mac.mac).unwrap(),
            _ => panic!("missing Access region"),
        }
    }
}
