//! Independently check RA versions, physical fields, native Access operations,
//! resource dominance and lexical configuration. Provider laws and arbitrary
//! Rust bodies remain conditional; this is a wiring correspondence audit.
use super::{ObservedOptions, RelationOptions};
use cq_ir::{native as provider, relational_plan as ra, rust_access_plan as access, symbol_name};
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

struct View {
    name: syn::Ident,
    provider: syn::Path,
    columns: Vec<String>,
    position: usize,
}
enum Binding {
    Local(syn::Expr),
    Materialized(syn::Path, Box<provider::Operation>),
}
fn numbered(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|s| s.parse::<usize>().is_ok())
}
fn reserved(name: &str) -> bool {
    ["__ml_view", "__ml_owner", "__ml_resource", "__ml_input"]
        .iter()
        .any(|p| numbered(name, p))
}

pub fn check_program(
    source: &ra::Module,
    target: &syn::File,
    options: &RelationOptions,
) -> syn::Result<()> {
    check_method(source, target, options, None)
}
pub fn check_observed_program(
    source: &ra::Module,
    target: &syn::File,
    options: &ObservedOptions,
) -> syn::Result<()> {
    check_method(
        source,
        target,
        &options.relations,
        Some(&options.counter_storage),
    )
}
pub fn check_query(
    source: &ra::Module,
    target: &syn::Expr,
    options: &RelationOptions,
) -> syn::Result<()> {
    check_closure(source, target, options, None)
}
pub fn check_observed_query(
    source: &ra::Module,
    target: &syn::Expr,
    options: &ObservedOptions,
) -> syn::Result<()> {
    check_closure(
        source,
        target,
        &options.relations,
        Some(&options.counter_storage),
    )
}
fn check_closure(
    source: &ra::Module,
    target: &syn::Expr,
    options: &RelationOptions,
    storage: Option<&syn::Path>,
) -> syn::Result<()> {
    let syn::Expr::Closure(closure) = target else {
        return Err(syn::Error::new_spanned(
            target,
            "expected physical callable",
        ));
    };
    if closure.capture.is_none() || closure.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            target,
            "physical callable must capture ordinary Rust configuration",
        ));
    }
    check_parameters(source, closure.inputs.iter(), options)?;
    let syn::Expr::Block(body) = closure.body.as_ref() else {
        return Err(syn::Error::new_spanned(target, "expected callable body"));
    };
    let initializer = storage.map(|storage| syn::parse_quote! {
        ({
            #[derive(Debug)]
            struct __MlStatistics<W> {
                pub steps: ::std::vec::Vec<(&'static ::core::primitive::str, ::core::primitive::usize, W)>,
                pub output: W,
            }
            |output| __MlStatistics { steps: ::std::vec::Vec::new(), output }
        })(<#storage::Work as ::core::default::Default>::default())
    });
    if storage.is_some() && closure.output != syn::ReturnType::Default {
        return Err(syn::Error::new_spanned(
            closure,
            "observed callable has an inferred local statistics type",
        ));
    }
    check_body(source, &body.block, options, storage, initializer.as_ref())
}
fn check_method(
    source: &ra::Module,
    target: &syn::File,
    options: &RelationOptions,
    storage: Option<&syn::Path>,
) -> syn::Result<()> {
    let program = &source.program.name;
    let ty: syn::Type = syn::parse_quote!(#program);
    let methods: Vec<_> = target
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item) if *item.self_ty == ty => Some(item),
            _ => None,
        })
        .flat_map(|item| &item.items)
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method) => Some(method),
            _ => None,
        })
        .collect();
    let [method] = methods.as_slice() else {
        return Err(syn::Error::new_spanned(
            target,
            "expected one physical run method",
        ));
    };
    if method.sig.ident != "run" {
        return Err(syn::Error::new_spanned(
            method,
            "expected physical run method",
        ));
    }
    let parameters = method
        .sig
        .inputs
        .iter()
        .map(|argument| match argument {
            syn::FnArg::Typed(argument) => Ok(syn::Pat::Type(argument.clone())),
            _ => Err(syn::Error::new_spanned(
                argument,
                "physical inputs are borrowed explicit parameters",
            )),
        })
        .collect::<syn::Result<Vec<_>>>()?;
    check_parameters(source, parameters.iter(), options)?;
    let initializer = if let Some(storage) = storage {
        let name = format_ident!("{}Stats", symbol_name(program));
        let visibility = &source.program.visibility;
        let expected: syn::ItemStruct = syn::parse_quote! {
            #[derive(Debug,Default)]
            #visibility struct #name {
                pub steps: ::std::vec::Vec<(&'static ::core::primitive::str, ::core::primitive::usize, #storage::Work)>,
                pub output: #storage::Work,
            }
        };
        let actual: Vec<_> = target
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Struct(item) if item.ident == name => Some(item),
                _ => None,
            })
            .collect();
        if actual.as_slice() != [&expected] {
            return Err(syn::Error::new_spanned(
                target,
                "observed item statistics scaffold changed",
            ));
        }
        Some(syn::parse_quote!(#name::default()))
    } else {
        None
    };
    check_body(
        source,
        &method.block,
        options,
        storage,
        initializer.as_ref(),
    )
}
fn check_parameters<'a>(
    source: &ra::Module,
    actual: impl Iterator<Item = &'a syn::Pat>,
    options: &RelationOptions,
) -> syn::Result<()> {
    ra::contract::check(source)?;
    options.validate(source)?;
    let actual: Vec<_> = actual.collect();
    if actual.len() != source.program.inputs().count() {
        return Err(syn::Error::new_spanned(
            source,
            "physical host input arity changed",
        ));
    }
    for ((index, input), actual) in source.program.inputs().enumerate().zip(actual) {
        let name = format_ident!("__ml_input{index}");
        let types = input
            .columns
            .iter()
            .map(|column| crate::rust_bindings::qualify_type(&column.ty));
        let provider = &options.inputs[&symbol_name(&input.name)];
        let expected: syn::FnArg = syn::parse_quote!(#name: &#provider! { type (#(#types,)*) });
        let syn::FnArg::Typed(expected) = expected else {
            unreachable!()
        };
        let expected = syn::Pat::Type(expected);
        if actual != &expected {
            return Err(syn::Error::new_spanned(
                actual,
                "physical host input provider or ownership changed",
            ));
        }
    }
    Ok(())
}
fn check_body(
    source: &ra::Module,
    body: &syn::Block,
    options: &RelationOptions,
    storage: Option<&syn::Path>,
    initializer: Option<&syn::Expr>,
) -> syn::Result<()> {
    ra::contract::check(source)?;
    options.validate(source)?;
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
    let fail = || {
        syn::Error::new_spanned(
            body,
            "physical Access wiring violated: provider, current version, fields, resource or definition order changed",
        )
    };
    let scope = Scope::new(body, options, storage, initializer)?;
    // Every host-owned statement must discharge one checked scaffold obligation.
    // Configured macro bodies remain opaque; extra host effects are not admitted.
    let mut checked_positions: BTreeSet<usize> = scope
        .macros
        .values()
        .map(|(position, _)| *position)
        .collect();
    if storage.is_some() {
        checked_positions.extend([0, 1]);
    }
    let mut bindings = BTreeMap::new();
    let mut owners = BTreeSet::new();
    for (position, stmt) in body.stmts.iter().enumerate() {
        match stmt {
            syn::Stmt::Local(local) => {
                if !cq_ir::cq::rust::bindings(&local.pat)
                    .iter()
                    .any(|id| reserved(&symbol_name(id)))
                {
                    continue;
                }
                let syn::Pat::Ident(pat) = &local.pat else {
                    return Err(fail());
                };
                let name = symbol_name(&pat.ident);
                if pat.mutability.is_some()
                    || pat.by_ref.is_some()
                    || pat.subpat.is_some()
                    || numbered(&name, "__ml_owner")
                    || numbered(&name, "__ml_input")
                {
                    return Err(fail());
                }
                let init = local.init.as_ref().ok_or_else(fail)?;
                if init.diverge.is_some()
                    || bindings
                        .insert(name, (position, Binding::Local(*init.expr.clone())))
                        .is_some()
                {
                    return Err(fail());
                }
            }
            syn::Stmt::Macro(stmt) => {
                let invocation: provider::Operation =
                    syn::parse2(stmt.mac.tokens.clone()).map_err(|_| fail())?;
                let (owner, value) = match &invocation {
                    provider::Operation::Materialize { owner, value, .. }
                    | provider::Operation::Produce { owner, value, .. } => (owner, value),
                    _ => return Err(fail()),
                };
                if !owners.insert(symbol_name(owner))
                    || bindings
                        .insert(
                            symbol_name(value),
                            (
                                position,
                                Binding::Materialized(stmt.mac.path.clone(), Box::new(invocation)),
                            ),
                        )
                        .is_some()
                {
                    return Err(fail());
                }
            }
            _ => {}
        }
    }
    let mut views = BTreeMap::<String, View>::new();
    let mut demands = BTreeMap::<(String, Vec<usize>, String), usize>::new();
    let mut previous = None;
    let mut materializations = 0;
    for (index, definition) in source.plan.definitions.iter().enumerate() {
        let result = symbol_name(&definition.result);
        let name = format_ident!("__ml_view{index}");
        let (position, binding) = bindings.get(&name.to_string()).ok_or_else(fail)?;
        checked_positions.insert(*position);
        if previous.is_some_and(|p| p >= *position) {
            return Err(fail());
        }
        let columns;
        let provider;
        match &definition.operator {
            ra::Operator::Rename(rename) => {
                let (input_index, schema) = source
                    .program
                    .inputs()
                    .enumerate()
                    .find(|(_, r)| symbol_name(&r.name) == symbol_name(&rename.relation))
                    .ok_or_else(fail)?;
                provider =
                    scope.resolve(&options.inputs[&symbol_name(&rename.relation)], *position)?;
                let input = format_ident!("__ml_input{input_index}");
                expect_local(binding, syn::parse_quote!(#provider! { input (#input) }))?;
                columns = schema
                    .columns
                    .iter()
                    .map(|c| {
                        rename
                            .mapping
                            .iter()
                            .find(|m| symbol_name(&m.source) == symbol_name(&c.name))
                            .map(|m| symbol_name(&m.target))
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(fail)?;
            }
            ra::Operator::Select(op) => {
                let input = views.get(&symbol_name(&op.input)).ok_or_else(fail)?;
                let input_info = &info[&symbol_name(&op.input)];
                let Binding::Local(syn::Expr::Block(block)) = binding else {
                    return Err(fail());
                };
                let Some(syn::Expr::Macro(mac)) = tail_expression(&block.block.stmts) else {
                    return Err(fail());
                };
                let provider::Operation::Select { rows, .. } = syn::parse2(mac.mac.tokens.clone())?
                else {
                    return Err(fail());
                };
                let actual_plan = access_argument(&rows, options.access_for(&definition.result)?)
                    .ok_or_else(fail)?;
                let Some(access::Clause {
                    access: access::RustAccess::If { condition, .. },
                    ..
                }) = actual_plan.body.get(1)
                else {
                    return Err(fail());
                };
                let syn::Expr::Block(guard) = condition else {
                    return Err(fail());
                };
                let Some(syn::Stmt::Expr(actual, None)) = guard.block.stmts.last() else {
                    return Err(fail());
                };
                check_expression(
                    actual,
                    &op.condition,
                    input,
                    input_info,
                    &syn::parse_quote!(__cq_row),
                    true,
                )?;
                let p = &input.provider;
                let v = &input.name;
                let next = options.access_for(&definition.result)?;
                let predicate = quote!({ let __cq_row = #p! { row (__cq_handle) }; #actual });
                let expected: access::Plan = syn::parse_quote! {
                    out(__cq_handle) => (__cq_handle) :-
                    Rows(__cq_handle) => for (__cq_handle,) in (#p! { rows (__cq_input) }.map(|handle| (handle,))),
                    if #predicate => if (#predicate);
                };
                access::contract::check(&expected)?;
                expect_local(
                    binding,
                    syn::parse_quote!({
                        let __cq_input = &#v;
                        #p! { select (__cq_input) (#next! { #expected }) }
                    }),
                )?;
                provider = input.provider.clone();
                columns = input.columns.clone();
            }
            ra::Operator::Semijoin(op) => {
                let left = views.get(&symbol_name(&op.left)).ok_or_else(fail)?;
                let right = views.get(&symbol_name(&op.right)).ok_or_else(fail)?;
                let shared: Vec<_> = info[&symbol_name(&op.left)]
                    .heading
                    .intersection(&info[&symbol_name(&op.right)].heading)
                    .cloned()
                    .collect();
                let selected_keys = options.keys_for(&definition.result)?;
                let demand = (
                    symbol_name(&op.right),
                    positions(&right.columns, &shared).ok_or_else(fail)?,
                    selected_keys.to_token_stream().to_string(),
                );
                let new = !demands.contains_key(&demand);
                let next = demands.len();
                let id = *demands.entry(demand).or_insert(next);
                let resource = format_ident!("__ml_resource{id}");
                let (rp, rb) = bindings.get(&resource.to_string()).ok_or_else(fail)?;
                checked_positions.insert(*rp);
                if *rp <= right.position
                    || *rp >= *position
                    || (new && previous.is_some_and(|p| *rp <= p))
                    || (!new && previous.is_some_and(|p| *rp >= p))
                {
                    return Err(fail());
                }
                let key_macro = scope.resolve(selected_keys, *rp)?;
                if new {
                    let fields = positions(&right.columns, &shared).ok_or_else(fail)?;
                    let row_provider = &right.provider;
                    let input = &right.name;
                    let values = fields.iter().map(|&column| {
                        let column = syn::Index::from(column);
                        quote!(&(#row_provider! { field (__row) (#column) }))
                    });
                    let plan: access::Plan = syn::parse_quote! {
                        out(__row) => (#key_macro! { key (#(#values,)*) }) :-
                        Rows(__row) => for (__row,) in (#row_provider! { scan (__cq_right) }.map(|row| (row,)));
                    };
                    access::contract::check(&plan)?;
                    expect_local(
                        rb,
                        syn::parse_quote!({let __cq_right = &#input; #key_macro! { build { #plan } }}),
                    )?;
                }
                let fields = positions(&left.columns, &shared).ok_or_else(fail)?;
                let scan = scope.resolve(options.candidates_for(&definition.result)?, *position)?;
                let lp = &left.provider;
                let l = &left.name;
                let next = options.access_for(&definition.result)?;
                // The exact key decodes a handle once when fields are needed.
                // An empty separator depends only on RHS existence.
                let key = match fields.as_slice() {
                    [] => quote!(#key_macro! { key () }),
                    fields => {
                        let values = fields.iter().map(|&column| {
                            let column = syn::Index::from(column);
                            quote!(&(#lp! { field (__cq_row) (#column) }))
                        });
                        quote!({
                            let __cq_row = #lp! { row (__cq_handle) };
                            #key_macro! { key (#(#values,)*) }
                        })
                    }
                };
                let pred = quote!(#key_macro! { contains (__cq_resource) (#key) });
                let plan: access::Plan = syn::parse_quote! {
                    out(__cq_handle) => (__cq_handle) :-
                    Candidates(__cq_handle) => for (__cq_handle,) in (
                        #scan! { candidates #lp { __cq_left }
                            (move |bounds| #key_macro! { may_match (__cq_resource) (bounds) (&[#(#fields),*]) })
                        }.map(|handle| (handle,))
                    ),
                    if #pred => if (#pred);
                };
                access::contract::check(&plan)?;
                expect_local(
                    binding,
                    syn::parse_quote!({
                        let __cq_left = &#l;
                        let __cq_resource = &#resource;
                        #lp! { select (__cq_left) (#next! { #plan }) }
                    }),
                )?;
                provider = left.provider.clone();
                columns = left.columns.clone();
            }
            ra::Operator::NaturalJoin(op) => {
                materializations += 1;
                columns = info[&result].heading.iter().cloned().collect();
                provider =
                    scope.resolve(options.result_rows_for(&definition.result)?, *position)?;
                let Binding::Materialized(actual, operation) = binding else {
                    return Err(fail());
                };
                if *actual != provider {
                    return Err(fail());
                }
                let left = views.get(&symbol_name(&op.left)).ok_or_else(fail)?;
                let right = views.get(&symbol_name(&op.right)).ok_or_else(fail)?;
                let key: Vec<_> = info[&symbol_name(&op.left)]
                    .heading
                    .intersection(&info[&symbol_name(&op.right)].heading)
                    .cloned()
                    .collect();
                let lk = positions(&left.columns, &key).ok_or_else(fail)?;
                let rk = positions(&right.columns, &key).ok_or_else(fail)?;
                let owner = format_ident!("__ml_owner{index}");
                let left_binding = format_ident!("__ml_view{index}_input0");
                let right_binding = format_ident!("__ml_view{index}_input1");
                let (lp, rp, l, r) = (&left.provider, &right.provider, &left.name, &right.name);
                let algorithm = scope.resolve(options.pairs_for(&definition.result)?, *position)?;
                let left_key = lk
                    .iter()
                    .map(|&i| syn::Index::from(i))
                    .map(|i| quote!(&(#lp! { field (__left_row) (#i) })));
                let right_key = rk
                    .iter()
                    .map(|&i| syn::Index::from(i))
                    .map(|i| quote!(&(#rp! { field (__right_row) (#i) })));
                let fields = columns.iter().map(|column| {
                    if let Some(i) = left.columns.iter().position(|v| v == column) {
                        let i = syn::Index::from(i);
                        quote!(::core::clone::Clone::clone(&(#lp! { field (__left_row) (#i) })))
                    } else {
                        let i = right.columns.iter().position(|v| v == column).unwrap();
                        let i = syn::Index::from(i);
                        quote!(::core::clone::Clone::clone(&(#rp! { field (__right_row) (#i) })))
                    }
                });
                let expected: provider::Operation = syn::parse_quote! {
                    produce #owner #name via __emit {
                        #algorithm! {
                            pairs __left_row in (#lp! { scan (#left_binding) }) by ((#(#left_key,)*)),
                                  __right_row in (#rp! { scan (#right_binding) }) by ((#(#right_key,)*)) {
                                __emit(#provider! { record (#(#fields,)*) });
                            }
                        };
                    }
                };
                if **operation != expected {
                    return Err(fail());
                }
                let setup: syn::Block = syn::parse_quote!({
                    let #left_binding = &#l;
                    let #right_binding = &#r;
                });
                if *position < 2 || body.stmts[position - 2..*position] != setup.stmts {
                    return Err(fail());
                }
                checked_positions.extend(position - 2..*position);
            }
            ra::Operator::Unit(_) | ra::Operator::Project(_) | ra::Operator::Extend(_) => {
                materializations += 1;
                columns = info[&result].heading.iter().cloned().collect();
                provider =
                    scope.resolve(options.result_rows_for(&definition.result)?, *position)?;
                let Binding::Materialized(actual, operation) = binding else {
                    return Err(fail());
                };
                let provider::Operation::Materialize {
                    owner, value, rows, ..
                } = &**operation
                else {
                    return Err(fail());
                };
                if *actual != provider
                    || *owner != format_ident!("__ml_owner{index}")
                    || *value != name
                {
                    return Err(fail());
                }
                match &definition.operator {
                    ra::Operator::Unit(_) => {
                        if **rows != syn::parse_quote!(::core::iter::once(#provider! { record () }))
                        {
                            return Err(fail());
                        }
                    }
                    ra::Operator::Project(op) => {
                        let input = views.get(&symbol_name(&op.input)).ok_or_else(fail)?;
                        let fields = positions(&input.columns, &columns).ok_or_else(fail)?;
                        let (p, v, next) = (
                            &input.provider,
                            &input.name,
                            options.access_for(&definition.result)?,
                        );
                        let input_binding = format_ident!("__ml_view{index}_input0");
                        let setup: syn::Stmt = syn::parse_quote!(let #input_binding = &#v;);
                        if *position == 0 || body.stmts[position - 1] != setup {
                            return Err(fail());
                        }
                        checked_positions.insert(position - 1);
                        let values = fields.iter().map(|&column| {
                            let column = syn::Index::from(column);
                            quote!(::core::clone::Clone::clone(&(#p! { field (__row) (#column) })))
                        });
                        let plan: access::Plan = syn::parse_quote! {
                            out(__row) => (#provider! { record (#(#values,)*) }) :-
                            Rows(__row) => for (__row,) in (#p! { scan (#input_binding) }.map(|row| (row,)));
                        };
                        access::contract::check(&plan)?;
                        if **rows != syn::parse_quote!(#next! { #plan }) {
                            return Err(fail());
                        }
                    }
                    ra::Operator::Extend(op) => {
                        let input = views.get(&symbol_name(&op.input)).ok_or_else(fail)?;
                        let input_info = &info[&symbol_name(&op.input)];
                        let actual_plan =
                            access_argument(rows, options.access_for(&definition.result)?)
                                .ok_or_else(fail)?;
                        let syn::Expr::Block(output) = &actual_plan.head.output else {
                            return Err(fail());
                        };
                        let Some(syn::Stmt::Local(local)) = output.block.stmts.first() else {
                            return Err(fail());
                        };
                        let actual = &local.init.as_ref().ok_or_else(fail)?.expr;
                        check_expression(
                            actual,
                            &op.value,
                            input,
                            input_info,
                            &syn::parse_quote!(__row),
                            false,
                        )?;
                        let (p, v, next) = (
                            &input.provider,
                            &input.name,
                            options.access_for(&definition.result)?,
                        );
                        let input_binding = format_ident!("__ml_view{index}_input0");
                        let setup: syn::Stmt = syn::parse_quote!(let #input_binding = &#v;);
                        if *position == 0 || body.stmts[position - 1] != setup {
                            return Err(fail());
                        }
                        checked_positions.insert(position - 1);
                        let types = op
                            .columns
                            .iter()
                            .map(|column| crate::rust_bindings::qualify_type(&column.ty));
                        let fields = columns.iter().map(|name| {
                            if let Some(column) = input.columns.iter().position(|c| c == name) {
                                let column = syn::Index::from(column);
                                quote!(::core::clone::Clone::clone(&(#p! { field (__row) (#column) })))
                            } else {
                                let column = syn::Index::from(op.columns.iter().position(|c| symbol_name(&c.name) == *name).unwrap());
                                quote!(__cq_value.#column)
                            }
                        });
                        let expected: access::Plan = syn::parse_quote! {
                            out(__row) => ({
                                let __cq_value: (#(#types,)*) = #actual;
                                #provider! { record (#(#fields,)*) }
                            }) :-
                            Rows(__row) => for (__row,) in (#p! { scan (#input_binding) }.map(|row| (row,)));
                        };
                        access::contract::check(&expected)?;
                        if **rows != syn::parse_quote!(#next! { #expected }) {
                            return Err(fail());
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        if storage.is_some() {
            let expected: syn::Stmt = syn::parse_quote! {
                __ml_stats.steps.push((#result, #provider! { len (#name) }, __ml_work.take()));
            };
            if body.stmts.get(position + 1) != Some(&expected) {
                return Err(fail());
            }
            checked_positions.insert(position + 1);
        }
        views.insert(
            result,
            View {
                name,
                provider,
                columns,
                position: *position,
            },
        );
        previous = Some(*position);
    }
    if bindings.len() != source.plan.definitions.len() + demands.len()
        || owners.len() != materializations
    {
        return Err(fail());
    }
    let output = &views[&symbol_name(&source.plan.output.input)];
    let output_info = &info[&symbol_name(&source.plan.output.input)];
    let types = ra::output_types(&source.program, output_info)?
        .iter()
        .map(crate::rust_bindings::qualify_type)
        .collect::<Vec<_>>();
    let tail_len = if storage.is_some() { 3 } else { 2 };
    let answer_position = body.stmts.len().checked_sub(tail_len).ok_or_else(fail)?;
    let syn::Stmt::Local(answer_local) = &body.stmts[answer_position] else {
        return Err(fail());
    };
    let syn::Expr::MethodCall(collect) = answer_local.init.as_ref().ok_or_else(fail)?.expr.as_ref()
    else {
        return Err(fail());
    };
    let syn::Expr::MethodCall(map) = collect.receiver.as_ref() else {
        return Err(fail());
    };
    let Some(syn::Expr::Closure(mapper)) = map.args.first() else {
        return Err(fail());
    };
    let syn::Expr::Tuple(tuple) = mapper.body.as_ref() else {
        return Err(fail());
    };
    if tuple.elems.len() != source.plan.output.head.args.len() {
        return Err(fail());
    }
    let provider = &output.provider;
    for (actual, original) in tuple.elems.iter().zip(&source.plan.output.head.args) {
        if let Some(variable) = cq_ir::cq::rust::variable(original)
            .filter(|v| output_info.heading.contains(&symbol_name(v)))
        {
            let column = syn::Index::from(
                output
                    .columns
                    .iter()
                    .position(|v| *v == symbol_name(variable))
                    .ok_or_else(fail)?,
            );
            let expected: syn::Expr = syn::parse_quote!(::core::clone::Clone::clone(&(#provider! { field (__ml_row) (#column) })));
            if actual != &expected {
                return Err(fail());
            }
        } else {
            check_expression(
                actual,
                original,
                output,
                output_info,
                &syn::parse_quote!(__ml_row),
                false,
            )?;
        }
    }
    let fields = &tuple.elems;
    let output_name = &output.name;
    let answer: syn::Stmt = syn::parse_quote! {
        let __ml_answer: ::std::collections::BTreeSet<(#(#types,)*)> = #provider! { scan (&#output_name) }
            .map(|__ml_row| (#fields)) .collect();
    };
    let mut tail = vec![answer];
    if storage.is_some() {
        tail.push(syn::parse_quote!(__ml_stats.output = __ml_work.take();));
        tail.push(syn::Stmt::Expr(
            syn::parse_quote!((__ml_answer.into_iter().collect::<::std::vec::Vec<(#(#types,)*)>>(), __ml_stats)),
            None,
        ));
    } else {
        tail.push(syn::Stmt::Expr(
            syn::parse_quote!(__ml_answer.into_iter().collect()),
            None,
        ));
    }
    if body.stmts.len() < tail.len() || body.stmts[body.stmts.len() - tail.len()..] != tail {
        return Err(fail());
    }
    checked_positions.extend(body.stmts.len() - tail.len()..body.stmts.len());
    if checked_positions.len() != body.stmts.len() {
        return Err(syn::Error::new_spanned(
            body,
            "unaccounted statement in physical host scaffold",
        ));
    }
    scope.finish()
}
fn expect_local(binding: &Binding, expected: syn::Expr) -> syn::Result<()> {
    if matches!(binding,Binding::Local(actual) if *actual == expected) {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            expected,
            "native Access expression changed its operands, fields or residual",
        ))
    }
}

fn tail_expression(statements: &[syn::Stmt]) -> Option<syn::Expr> {
    match statements.last()? {
        syn::Stmt::Expr(expression, None) => Some(expression.clone()),
        syn::Stmt::Macro(statement) if statement.semi_token.is_none() => {
            Some(syn::Expr::Macro(syn::ExprMacro {
                attrs: statement.attrs.clone(),
                mac: statement.mac.clone(),
            }))
        }
        _ => None,
    }
}

fn access_argument(expression: &syn::Expr, next: &syn::Path) -> Option<access::Plan> {
    let syn::Expr::Macro(mac) = expression else {
        return None;
    };
    (mac.mac.path == *next)
        .then(|| syn::parse2(mac.mac.tokens.clone()).ok())
        .flatten()
}

/// Observe the actual lexical aliases and unchanged Rust leaf. No emitter is
/// called: every alias must refer to its original logical field, with exactly
/// the source binding's borrowed/owned convention.
fn check_expression(
    actual: &syn::Expr,
    original: &syn::Expr,
    view: &View,
    info: &ra::RelationInfo,
    row: &syn::Expr,
    condition: bool,
) -> syn::Result<()> {
    let fail = || syn::Error::new_spanned(actual, "Rust leaf or logical field binding changed");
    let provider = &view.provider;
    let value = |name: &str| -> syn::Expr {
        let column = syn::Index::from(view.columns.iter().position(|v| v == name).unwrap());
        syn::parse_quote!(&(#provider! { field (#row) (#column) }))
    };
    if condition {
        if let Some(predicate) = cq_ir::cq::Predicate::from_expr(original)
            .filter(|p| p.variables().iter().all(|name| info.heading.contains(name)))
        {
            let term = |term: &cq_ir::cq::Term| -> syn::Expr {
                match term {
                    cq_ir::cq::Term::Variable(name) => value(&symbol_name(name)),
                    cq_ir::cq::Term::Integer(number) => syn::parse_quote!(&#number),
                }
            };
            let (left, right, comparison) = (
                term(&predicate.left),
                term(&predicate.right),
                &predicate.comparison,
            );
            return (actual == &syn::parse_quote!(#left #comparison #right))
                .then_some(())
                .ok_or_else(fail);
        }
    }
    let syn::Expr::Block(block) = actual else {
        return Err(fail());
    };
    let Some((last, aliases)) = block.block.stmts.split_last() else {
        return Err(fail());
    };
    if tail_expression(std::slice::from_ref(last)).as_ref() != Some(original) {
        return Err(fail());
    }
    let used = cq_ir::cq::rust::free_variables(original).dependencies(&info.heading);
    let mut bound = BTreeSet::new();
    for statement in aliases {
        let syn::Stmt::Local(local) = statement else {
            return Err(fail());
        };
        let syn::Pat::Ident(pattern) = &local.pat else {
            return Err(fail());
        };
        let name = symbol_name(&pattern.ident);
        if pattern.by_ref.is_some()
            || pattern.mutability.is_some()
            || pattern.subpat.is_some()
            || !used.contains(&name)
        {
            return Err(fail());
        }
        let expected = value(&name);
        let initializer = local.init.as_ref().ok_or_else(fail)?;
        if initializer.diverge.is_some() || *initializer.expr != expected {
            return Err(fail());
        }
        bound.insert(name);
    }
    used.is_subset(&bound).then_some(()).ok_or_else(fail)
}

fn positions(layout: &[String], columns: &[String]) -> Option<Vec<usize>> {
    columns
        .iter()
        .map(|column| layout.iter().position(|c| c == column))
        .collect()
}

/// Inspect actual Rust macro bindings. This checks the lexical configuration at
/// each use, rather than treating an alias name as a trusted provider catalogue.
struct Scope<'a> {
    macros: BTreeMap<String, (usize, &'a syn::ItemMacro)>,
    used: std::cell::RefCell<BTreeSet<String>>,
    storage: Option<&'a syn::Path>,
}
impl<'a> Scope<'a> {
    fn new(
        block: &'a syn::Block,
        options: &RelationOptions,
        storage: Option<&'a syn::Path>,
        initializer: Option<&syn::Expr>,
    ) -> syn::Result<Self> {
        if let Some(storage) = storage {
            let initializer = initializer.ok_or_else(|| {
                syn::Error::new_spanned(block, "missing expected statistics initializer")
            })?;
            let work: syn::Stmt =
                syn::parse_quote!(let __ml_work = &#storage::Counters::default(););
            let statistics: syn::Stmt = syn::parse_quote!(let mut __ml_stats = #initializer;);
            if block.stmts.first() != Some(&work) || block.stmts.get(1) != Some(&statistics) {
                return Err(syn::Error::new_spanned(
                    block,
                    "observed host must initialize fresh counters and empty statistics exactly once before operations",
                ));
            }
        }
        let mut macros = BTreeMap::new();
        let mut work_position = None;
        for (position, stmt) in block.stmts.iter().enumerate() {
            if let syn::Stmt::Local(local) = stmt {
                if cq_ir::cq::rust::bindings(&local.pat)
                    .iter()
                    .any(|id| symbol_name(id) == "__ml_stats")
                    && (storage.is_none() || position != 1)
                {
                    return Err(syn::Error::new_spanned(
                        stmt,
                        "statistics must have one unshadowed host initialization",
                    ));
                }
                if cq_ir::cq::rust::bindings(&local.pat)
                    .iter()
                    .any(|id| symbol_name(id) == "__ml_work")
                {
                    let storage = storage.ok_or_else(|| {
                        syn::Error::new_spanned(stmt, "plain hosts cannot bind counters")
                    })?;
                    let expected: syn::Stmt =
                        syn::parse_quote!(let __ml_work = &#storage::Counters::default(););
                    if work_position.replace(position).is_some() || stmt != &expected {
                        return Err(syn::Error::new_spanned(
                            stmt,
                            "invalid instrumentation binding",
                        ));
                    }
                }
            }
            if let syn::Stmt::Item(syn::Item::Macro(item)) = stmt {
                let Some(name) = &item.ident else {
                    return Err(syn::Error::new_spanned(
                        item,
                        "expected configured macro binding",
                    ));
                };
                if item.mac.path != syn::parse_quote!(macro_rules)
                    || (storage.is_some() && work_position.is_none())
                    || macros.insert(symbol_name(name), (position, item)).is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item,
                        "invalid configured macro scope",
                    ));
                }
            }
        }
        if options.access.iter().any(|path| {
            path.get_ident()
                .is_some_and(|name| macros.contains_key(&symbol_name(name)))
        }) {
            return Err(syn::Error::new_spanned(
                block,
                "configured macro shadows the selected Access continuation",
            ));
        }
        if storage.is_some() && work_position.is_none() {
            return Err(syn::Error::new_spanned(
                block,
                "missing instrumentation binding",
            ));
        }
        Ok(Self {
            macros,
            used: Default::default(),
            storage,
        })
    }
    fn check(&self, alias: &syn::Path, selected: &syn::Path, before: usize) -> syn::Result<()> {
        let fail = || {
            syn::Error::new_spanned(
                alias,
                "configured macro changed its implementation, arguments or lexical scope",
            )
        };
        if selected
            .get_ident()
            .is_some_and(|name| self.macros.contains_key(&symbol_name(name)))
        {
            return Err(fail());
        }
        let name = alias.get_ident().ok_or_else(fail)?;
        let (position, actual) = self.macros.get(&symbol_name(name)).ok_or_else(fail)?;
        // Reconstruct the administrative binding contract independently of
        // the emitter's wrapper function and name-allocation table.
        let work = self.storage.map(|_| quote!(work (__ml_work);));
        let expected: syn::ItemMacro = syn::parse_quote! {
            macro_rules! #name {
                ($($source:tt)*) => {
                    #selected! { #work $($source)* }
                };
            }
        };
        if *position >= before
            || actual.to_token_stream().to_string() != expected.to_token_stream().to_string()
        {
            return Err(fail());
        }
        self.used.borrow_mut().insert(symbol_name(name));
        Ok(())
    }
    fn resolve(&self, selected: &syn::Path, before: usize) -> syn::Result<syn::Path> {
        if self.storage.is_none() {
            return Ok(selected.clone());
        }
        let mut candidates = Vec::new();
        for name in self.macros.keys() {
            let alias: syn::Path = syn::parse_str(name)?;
            if self.check(&alias, selected, before).is_ok() {
                candidates.push(alias);
            }
        }
        if candidates.len() != 1 {
            return Err(syn::Error::new_spanned(
                selected,
                "expected exactly one lexically configured implementation",
            ));
        }
        Ok(candidates.remove(0))
    }
    fn finish(&self) -> syn::Result<()> {
        if self.used.borrow().len() != self.macros.len() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "unreferenced configured macro binding",
            ));
        }
        Ok(())
    }
}
