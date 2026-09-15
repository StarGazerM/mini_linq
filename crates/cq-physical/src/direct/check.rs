//! Independently check RA versions, physical fields, native Access operations,
//! resource dominance and lexical configuration. Provider laws and arbitrary
//! Rust bodies remain conditional; this is a wiring correspondence audit.
use super::{ObservedOptions, PhysicalOptions};
use cq_ir::{
    physical_provider as provider, relational_plan as ra, rust_access_plan as access, symbol_name,
};
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

pub fn check_physical(
    source: &ra::Module,
    target: &syn::File,
    options: &PhysicalOptions,
) -> syn::Result<()> {
    check_method(source, target, options, None)
}
pub fn check_observed_physical(
    source: &ra::Module,
    target: &syn::File,
    options: &ObservedOptions,
) -> syn::Result<()> {
    check_method(source, target, &options.physical, Some(&options.storage))
}
pub fn check_physical_query(
    source: &ra::Module,
    target: &syn::Expr,
    options: &PhysicalOptions,
) -> syn::Result<()> {
    check_closure(source, target, options, None)
}
pub fn check_observed_physical_query(
    source: &ra::Module,
    target: &syn::Expr,
    options: &ObservedOptions,
) -> syn::Result<()> {
    check_closure(source, target, &options.physical, Some(&options.storage))
}
fn check_closure(
    source: &ra::Module,
    target: &syn::Expr,
    options: &PhysicalOptions,
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
    options: &PhysicalOptions,
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
    options: &PhysicalOptions,
) -> syn::Result<()> {
    ra::contract::check(source)?;
    options.validate(source)?;
    if source
        .program
        .declarations
        .iter()
        .flat_map(|declaration| &declaration.columns)
        .any(|column| column.ty != syn::parse_quote!(i64))
        || source.plan.output.head.args.len() > 12
    {
        return Err(syn::Error::new_spanned(
            source,
            "array physical domain requires i64 columns and at most twelve output fields",
        ));
    }
    let actual: Vec<_> = actual.collect();
    if actual.len() != source.program.inputs().count() {
        return Err(syn::Error::new_spanned(
            source,
            "physical host input arity changed",
        ));
    }
    for ((index, input), actual) in source.program.inputs().enumerate().zip(actual) {
        let name = format_ident!("__ml_input{index}");
        let width = cq_ir::Column::from(input.arity());
        let provider = &options.inputs[&symbol_name(&input.name)];
        let expected: syn::FnArg = syn::parse_quote!(#name: &#provider! { type #width });
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
    options: &PhysicalOptions,
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
                let provider::Operation::Materialize { owner, value, .. } = &invocation else {
                    return Err(fail());
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
            ra::Operator::Semijoin(op) => {
                let left = views.get(&symbol_name(&op.left)).ok_or_else(fail)?;
                let right = views.get(&symbol_name(&op.right)).ok_or_else(fail)?;
                let shared: Vec<_> = info[&symbol_name(&op.left)]
                    .heading
                    .intersection(&info[&symbol_name(&op.right)].heading)
                    .cloned()
                    .collect();
                let selected_keys = options.key_sets.get(&result).unwrap_or(&options.keys);
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
                    let width = fields.len();
                    let row_provider = &right.provider;
                    let input = &right.name;
                    let plan: access::Plan = syn::parse_quote! {
                        out(__row) => ({ let __key: [::std::primitive::i64; #width] = [#(__row[#fields]),*]; __key }) :-
                        Rows(__row) => for (__row,) in (#row_provider! { scan (__cq_right) }.map(|row| (row,)));
                    };
                    access::contract::check(&plan)?;
                    expect_local(
                        rb,
                        syn::parse_quote!({let __cq_right = &#input; #key_macro! { build { #plan } }}),
                    )?;
                }
                let fields = positions(&left.columns, &shared).ok_or_else(fail)?;
                let scan = scope.resolve(
                    options.scans.get(&result).unwrap_or(&options.scan),
                    *position,
                )?;
                let lp = &left.provider;
                let l = &left.name;
                let next = &options.access;
                // The exact key decodes a handle once when fields are needed.
                // An empty separator depends only on RHS existence.
                let key = match fields.as_slice() {
                    [] => quote!([]),
                    fields => quote!({
                        let __cq_row = #lp! { row (__cq_handle) };
                        [#(__cq_row[#fields]),*]
                    }),
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
            ra::Operator::Unit(_) | ra::Operator::Project(_) | ra::Operator::NaturalJoin(_) => {
                materializations += 1;
                columns = info[&result].heading.iter().cloned().collect();
                provider = scope.resolve(
                    options.results.get(&result).unwrap_or(&options.materialize),
                    *position,
                )?;
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
                        if !unit_rows_are(rows) {
                            return Err(fail());
                        }
                    }
                    ra::Operator::Project(op) => {
                        let input = views.get(&symbol_name(&op.input)).ok_or_else(fail)?;
                        let fields = positions(&input.columns, &columns).ok_or_else(fail)?;
                        let width = fields.len();
                        let (p, v, next) = (&input.provider, &input.name, &options.access);
                        let input_binding = format_ident!("__ml_owner{index}_input");
                        let setup: syn::Stmt = syn::parse_quote!(let #input_binding = &#v;);
                        if *position == 0 || body.stmts[position - 1] != setup {
                            return Err(fail());
                        }
                        checked_positions.insert(position - 1);
                        let plan: access::Plan = syn::parse_quote! {
                            out(__row) => ({ let __projected: [::std::primitive::i64; #width] = [#(__row[#fields]),*]; __projected }) :-
                            Rows(__row) => for (__row,) in (#p! { scan (#input_binding) }.map(|row| (row,)));
                        };
                        access::contract::check(&plan)?;
                        if **rows != syn::parse_quote!(#next! { #plan }) {
                            return Err(fail());
                        }
                    }
                    ra::Operator::NaturalJoin(op) => {
                        let left = views.get(&symbol_name(&op.left)).ok_or_else(fail)?;
                        let right = views.get(&symbol_name(&op.right)).ok_or_else(fail)?;
                        let key: Vec<_> = info[&symbol_name(&op.left)]
                            .heading
                            .intersection(&info[&symbol_name(&op.right)].heading)
                            .cloned()
                            .collect();
                        let lk = positions(&left.columns, &key).ok_or_else(fail)?;
                        let rk = positions(&right.columns, &key).ok_or_else(fail)?;
                        let left_binding = format_ident!("__ml_owner{index}_left");
                        let right_binding = format_ident!("__ml_owner{index}_right");
                        let resource_binding = format_ident!("__ml_owner{index}_witness");
                        let resource_reference = format_ident!("__ml_owner{index}_witness_ref");
                        let key_width = key.len();
                        let width = columns.len();
                        let fields = columns.iter().map(|c| {
                            if let Some(i) = left.columns.iter().position(|v| v == c) {
                                quote!(__left_row[#i])
                            } else {
                                let i = right.columns.iter().position(|v| v == c).unwrap();
                                quote!(__right_row[#i])
                            }
                        });
                        let (lp, rp, l, r, next) = (
                            &left.provider,
                            &right.provider,
                            &left.name,
                            &right.name,
                            &options.access,
                        );
                        let witness = scope.resolve(
                            options.joins.get(&result).unwrap_or(&options.join),
                            *position,
                        )?;
                        let build: access::Plan = syn::parse_quote! {
                            out(__row) => ({ let __key: [::std::primitive::i64; #key_width] = [#(__row[#rk]),*]; (__key, __row) }) :-
                            Rows(__row) => for (__row,) in (#rp! { scan (#right_binding) }.map(|row| (row,)));
                        };
                        let plan: access::Plan = syn::parse_quote! {
                            out(__left_row,__right_row) => ({ let __joined: [::std::primitive::i64; #width] = [#(#fields),*]; __joined }) :-
                            Left(__left_row) => for (__left_row,) in (#lp! { scan (#left_binding) }.map(|row| (row,))),
                            Right(__right_row) => for (__right_row,) in (#witness! { probe (#resource_reference) ([#(__left_row[#lk]),*]) }.map(|row| (row,)));
                        };
                        access::contract::check(&build)?;
                        access::contract::check(&plan)?;
                        let setup: syn::Block = syn::parse_quote!({
                            let #left_binding = &#l;
                            let #right_binding = &#r;
                            let #resource_binding = #witness! { build { #build } };
                            let #resource_reference = &#resource_binding;
                        });
                        if *position < 4 || body.stmts[position - 4..*position] != setup.stmts {
                            return Err(fail());
                        }
                        checked_positions.extend(position - 4..*position);
                        if **rows != syn::parse_quote!(#next! { #plan }) {
                            return Err(fail());
                        }
                    }
                    _ => unreachable!(),
                }
            }
            _ => return Err(fail()),
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
    let fields = source
        .plan
        .output
        .head
        .args
        .iter()
        .map(|argument| {
            let name = cq_ir::cq::rust::variable(argument)?;
            output
                .columns
                .iter()
                .position(|column| *column == symbol_name(name))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(fail)?;
    let output_name = &output.name;
    let provider = &output.provider;
    let types = fields.iter().map(|_| quote!(::std::primitive::i64));
    let answer: syn::Stmt = syn::parse_quote! {
        let __ml_answer: ::std::collections::BTreeSet<(#(#types,)*)> = #provider! { scan (&#output_name) }
            .map(|__ml_row| (#(__ml_row[#fields],)*)) .collect();
    };
    let mut tail = vec![answer];
    if storage.is_some() {
        tail.push(syn::parse_quote!(__ml_stats.output = __ml_work.take();));
        let types = fields.iter().map(|_| quote!(::std::primitive::i64));
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
fn positions(layout: &[String], columns: &[String]) -> Option<Vec<usize>> {
    columns
        .iter()
        .map(|column| layout.iter().position(|c| c == column))
        .collect()
}

fn unit_rows_are(actual: &syn::Expr) -> bool {
    // Rust permits an optional trailing comma in generic arguments. Compare
    // the parsed argument sequence rather than its punctuation state.
    fn normalize(expression: &mut syn::Expr) {
        let syn::Expr::Call(call) = expression else {
            return;
        };
        let syn::Expr::Path(path) = call.func.as_mut() else {
            return;
        };
        for segment in &mut path.path.segments {
            if let syn::PathArguments::AngleBracketed(arguments) = &mut segment.arguments {
                arguments.args.pop_punct();
            }
        }
    }
    let mut actual = actual.clone();
    let mut expected: syn::Expr =
        syn::parse_quote!(::std::collections::HashSet::<[::std::primitive::i64; 0]>::from([[]]));
    normalize(&mut actual);
    normalize(&mut expected);
    actual == expected
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
        options: &PhysicalOptions,
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
        if [&options.access].iter().any(|path| {
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
