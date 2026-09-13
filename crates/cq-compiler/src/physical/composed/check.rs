//! Independent audit of composition wiring before provider expansion.
//! Reconstructs versions and field positions from RA, then checks actual macro
//! invocations. Opaque provider laws and Rust type checking remain separate.

use super::PhysicalOptions;
use cq_ir::{
    cq, physical_context as context, physical_provider as provider, relational_plan as ra,
    rust_access_plan as access, symbol_name,
};
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

struct Witness {
    name: syn::Ident,
    provider: Option<syn::Path>,
    columns: Vec<String>,
    position: usize,
}

enum Binding {
    Local(syn::Expr),
    Materialized(syn::Path, Box<provider::Operation>),
}

fn numbered(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|suffix| suffix.parse::<usize>().is_ok())
}

fn reserved(name: &str) -> bool {
    ["__ml_view", "__ml_owner", "__ml_resource", "__ml_input"]
        .iter()
        .any(|prefix| numbered(name, prefix))
}

/// Check selected providers, source versions, projections, resource sharing and
/// definition order in the emitted invocation graph. This is not a verifier of
/// arbitrary provider expansion or the complete generated query API.
pub fn check_physical(
    source: &ra::Module,
    target: &syn::File,
    options: &PhysicalOptions,
) -> syn::Result<()> {
    check_method(source, target, options, false)?;
    check_method(source, target, options, true)
}
fn check_method(
    source: &ra::Module,
    target: &syn::File,
    options: &PhysicalOptions,
    observed: bool,
) -> syn::Result<()> {
    ra::contract::check(source)?;
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
    let fail = || {
        syn::Error::new_spanned(
            target,
            "physical composition wiring violated: provider, operand version, key projection, resource pairing or definition order changed",
        )
    };
    let inputs: BTreeSet<_> = source
        .program
        .inputs()
        .map(|r| symbol_name(&r.name))
        .collect();
    let mut materialized = BTreeSet::new();
    let mut semijoins = BTreeSet::new();
    let mut joins = BTreeSet::new();
    for definition in &source.plan.definitions {
        let name = symbol_name(&definition.result);
        match &definition.operator {
            ra::Operator::Semijoin(_) => {
                semijoins.insert(name);
            }
            ra::Operator::NaturalJoin(_) => {
                joins.insert(name.clone());
                materialized.insert(name);
            }
            ra::Operator::Project(_) | ra::Operator::Unit(_) => {
                materialized.insert(name);
            }
            ra::Operator::Rename(_) => {}
            _ => return Err(fail()),
        }
    }
    if options.inputs.keys().cloned().collect::<BTreeSet<_>>() != inputs
        || !options
            .results
            .keys()
            .all(|name| materialized.contains(name))
        || !options
            .semijoins
            .keys()
            .all(|name| semijoins.contains(name))
        || !options.key_sets.keys().all(|name| semijoins.contains(name))
        || !options.joins.keys().all(|name| joins.contains(name))
    {
        return Err(fail());
    }
    let program = &source.program.name;
    let program_type: syn::Type = syn::parse_quote!(#program);
    let methods: Vec<_> = target
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(i) if *i.self_ty == program_type => Some(i),
            _ => None,
        })
        .flat_map(|i| &i.items)
        .filter_map(|item| match item {
            syn::ImplItem::Fn(f)
                if f.sig.ident == if observed { "run_with_stats" } else { "run" } =>
            {
                Some(f)
            }
            _ => None,
        })
        .collect();
    let [method] = methods.as_slice() else {
        return Err(fail());
    };
    let scope = Scope::new(&method.block, options, observed)?;
    let mut bindings = BTreeMap::new();
    let mut owners = BTreeSet::new();
    for (position, statement) in method.block.stmts.iter().enumerate() {
        match statement {
            syn::Stmt::Local(local) => {
                let names = cq_ir::cq::rust::bindings(&local.pat);
                if !names.iter().any(|id| reserved(&symbol_name(id))) {
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
            syn::Stmt::Macro(statement) => {
                let invocation: provider::Operation =
                    syn::parse2(statement.mac.tokens.clone()).map_err(|_| fail())?;
                let provider::Operation::Materialize { owner, value, .. } = &invocation else {
                    return Err(fail());
                };
                if !owners.insert(symbol_name(owner))
                    || bindings
                        .insert(
                            symbol_name(value),
                            (
                                position,
                                Binding::Materialized(
                                    statement.mac.path.clone(),
                                    Box::new(invocation),
                                ),
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
    // Reuse requires identical current logical version, separator, recipe and
    // key implementation. This does not consult the emitter's resource table.
    let mut demands = BTreeMap::<(String, Vec<String>, String, String), usize>::new();
    let mut views = BTreeMap::<String, Witness>::new();
    let mut occupied: BTreeSet<_> = source
        .program
        .inputs()
        .map(|r| symbol_name(&r.name))
        .chain(
            source
                .plan
                .definitions
                .iter()
                .map(|d| symbol_name(&d.result)),
        )
        .collect();
    let mut projections = BTreeMap::<usize, syn::Ident>::new();
    let mut previous = None;
    for (index, definition) in source.plan.definitions.iter().enumerate() {
        let result = symbol_name(&definition.result);
        let name = format_ident!("__ml_view{index}");
        let (position, binding) = bindings.get(&name.to_string()).ok_or_else(fail)?;
        if previous.is_some_and(|p| *position <= p) {
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
                    .find(|(_, input)| symbol_name(&input.name) == symbol_name(&rename.relation))
                    .ok_or_else(fail)?;
                let original_provider = &options.inputs[&symbol_name(&rename.relation)];
                let Binding::Local(syn::Expr::Macro(actual)) = binding else {
                    return Err(fail());
                };
                scope.check(&actual.mac.path, original_provider, None, false, *position)?;
                provider = Some(actual.mac.path.clone());
                let configured = &actual.mac.path;
                let input = format_ident!("__ml_input{input_index}");
                let expected: syn::Expr = syn::parse_quote!(#configured! { input (#input) });
                if !local_is(binding, &expected) {
                    return Err(fail());
                }
                columns = schema
                    .columns
                    .iter()
                    .map(|column| {
                        rename
                            .mapping
                            .iter()
                            .find(|mapping| {
                                symbol_name(&mapping.source) == symbol_name(&column.name)
                            })
                            .map(|mapping| symbol_name(&mapping.target))
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(fail)?;
            }
            ra::Operator::Semijoin(op) => {
                let right = views.get(&symbol_name(&op.right)).ok_or_else(fail)?;
                let shared: Vec<_> = info[&symbol_name(&op.left)]
                    .heading
                    .intersection(&info[&symbol_name(&op.right)].heading)
                    .cloned()
                    .collect();
                let recipe = options.semijoins.get(&result).unwrap_or(&options.semijoin);
                let keys = options.key_sets.get(&result).unwrap_or(&options.keys);
                let key = (
                    symbol_name(&op.right),
                    shared.clone(),
                    recipe.to_token_stream().to_string(),
                    keys.to_token_stream().to_string(),
                );
                let new = !demands.contains_key(&key);
                let next = demands.len();
                let resource_id = *demands.entry(key).or_insert(next);
                let resource = format_ident!("__ml_resource{resource_id}");
                let (resource_position, resource_binding) =
                    bindings.get(&resource.to_string()).ok_or_else(fail)?;
                if *resource_position <= right.position
                    || *resource_position >= *position
                    || (new && previous.is_some_and(|p| *resource_position <= p))
                    || (!new && previous.is_some_and(|p| *resource_position >= p))
                {
                    return Err(fail());
                }
                if new {
                    let build = operator_call(
                        resource_binding,
                        recipe,
                        Some(keys),
                        &scope,
                        *resource_position,
                    )?;
                    context::contract::check_project(&build)?;
                    let ra::Operator::Project(project) = &build.source.operator else {
                        return Err(fail());
                    };
                    let projected = &build.source.result;
                    if !occupied.insert(symbol_name(projected))
                        || project.input != op.right
                        || project
                            .attributes
                            .iter()
                            .map(symbol_name)
                            .collect::<BTreeSet<_>>()
                            != shared.iter().cloned().collect()
                    {
                        return Err(fail());
                    }
                    check_context(&build, &build.source, &views, *resource_position)?;
                    projections.insert(resource_id, projected.clone());
                    views.insert(
                        symbol_name(projected),
                        Witness {
                            name: resource.clone(),
                            provider: None,
                            columns: shared.clone(),
                            position: *resource_position,
                        },
                    );
                }
                let projected = projections.get(&resource_id).ok_or_else(fail)?;
                let left_id = &op.left;
                let result_id = &definition.result;
                let expected: ra::Definition =
                    syn::parse_quote!(#result_id = semijoin #left_id with #projected;);
                let reduce = operator_call(binding, recipe, Some(keys), &scope, *position)?;
                context::contract::check_semijoin(&reduce)?;
                check_context(&reduce, &expected, &views, *position)?;
                let left = views.get(&symbol_name(&op.left)).ok_or_else(fail)?;
                provider = left.provider.clone();
                columns = left.columns.clone();
            }
            ra::Operator::Unit(_) | ra::Operator::Project(_) | ra::Operator::NaturalJoin(_) => {
                columns = info[&result].heading.iter().cloned().collect();
                let original_provider = options
                    .results
                    .get(&result)
                    .unwrap_or(&options.materialize)
                    .clone();
                let Binding::Materialized(actual_provider, materialization) = binding else {
                    return Err(fail());
                };
                let provider::Operation::Materialize {
                    owner,
                    value,
                    rows,
                    block_size,
                    ..
                } = &**materialization
                else {
                    return Err(fail());
                };
                scope.check(actual_provider, &original_provider, None, false, *position)?;
                provider = Some(actual_provider.clone());
                if *owner != format_ident!("__ml_owner{index}")
                    || *value != name
                    || **block_size != syn::parse_quote!(__ml_block_rows)
                {
                    return Err(fail());
                }
                match &definition.operator {
                    ra::Operator::Unit(_) => {
                        if !unit_rows_are(rows) {
                            return Err(fail());
                        }
                    }
                    ra::Operator::NaturalJoin(_) => {
                        let recipe = options.joins.get(&result).unwrap_or(&options.join);
                        let syn::Expr::Macro(invocation) = rows.as_ref() else {
                            return Err(fail());
                        };
                        scope.check(&invocation.mac.path, recipe, None, true, *position)?;
                        let context: context::Context = syn::parse2(invocation.mac.tokens.clone())?;
                        context::contract::check_join(&context)?;
                        check_context(&context, definition, &views, *position)?;
                    }
                    ra::Operator::Project(op) => {
                        let input = views.get(&symbol_name(&op.input)).ok_or_else(fail)?;
                        check_project(rows, input, &columns, options)?;
                    }
                    _ => unreachable!(),
                }
            }
            _ => return Err(fail()),
        }
        views.insert(
            result,
            Witness {
                name,
                provider,
                columns,
                position: *position,
            },
        );
        previous = Some(*position);
    }
    if bindings.len() != source.plan.definitions.len() + demands.len()
        || owners.len() != materialized.len()
    {
        return Err(fail());
    }
    scope.finish()?;
    Ok(())
}

/// Check contextual assumptions against independently reconstructed whole-source
/// bindings. The local IR contract alone cannot establish these Rust meanings.
fn check_context(
    actual: &context::Context,
    definition: &ra::Definition,
    views: &BTreeMap<String, Witness>,
    before: usize,
) -> syn::Result<()> {
    let fail = || {
        syn::Error::new_spanned(
            actual,
            "physical context changed its source definition or binding assumptions",
        )
    };
    if actual.source != *definition {
        return Err(fail());
    }
    for binding in &actual.bindings {
        let witness = views
            .get(&symbol_name(&binding.atom.relation))
            .ok_or_else(fail)?;
        let columns: Option<Vec<_>> = binding
            .atom
            .args
            .iter()
            .map(|arg| cq::rust::variable(arg).map(symbol_name))
            .collect();
        let name = &witness.name;
        if columns.as_ref() != Some(&witness.columns)
            || binding.provider.as_ref().map(|p| &p.path) != witness.provider.as_ref()
            || witness.position >= before
            || binding.value != syn::parse_quote!(#name)
        {
            return Err(fail());
        }
    }
    Ok(())
}

fn operator_call(
    binding: &Binding,
    recipe: &syn::Path,
    keys: Option<&syn::Path>,
    scope: &Scope<'_>,
    before: usize,
) -> syn::Result<context::Context> {
    if let Binding::Local(syn::Expr::Macro(invocation)) = binding {
        scope.check(&invocation.mac.path, recipe, keys, true, before)?;
        return syn::parse2(invocation.mac.tokens.clone());
    }
    Err(syn::Error::new_spanned(
        recipe,
        "expected selected contextual operator invocation",
    ))
}

fn local_is(binding: &Binding, expected: &syn::Expr) -> bool {
    matches!(binding, Binding::Local(actual) if actual == expected)
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

fn check_project(
    actual: &syn::Expr,
    input: &Witness,
    columns: &[String],
    options: &PhysicalOptions,
) -> syn::Result<()> {
    let fields = positions(&input.columns, columns)
        .ok_or_else(|| syn::Error::new_spanned(actual, "projection field absent"))?;
    let width = columns.len();
    let (provider, name, access) = (
        input
            .provider
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(actual, "projection requires rows"))?,
        &input.name,
        &options.access,
    );
    let syn::Expr::Macro(actual) = actual else {
        return Err(syn::Error::new_spanned(
            actual,
            "expected Access projection",
        ));
    };
    if actual.mac.path != *access {
        return Err(syn::Error::new_spanned(
            actual,
            "projection continuation changed",
        ));
    }
    let plan: access::Plan = syn::parse2(actual.mac.tokens.clone())?;
    let expected: access::Plan = syn::parse_quote! {
        out(__row) => ({ let __projected: [::std::primitive::i64; #width] = [#(__row[#fields]),*]; __projected }) :-
        Rows(__row) => for (__row,) in (#provider! { scan (&#name) }.map(|row| (row,)));
    };
    access::contract::check(&plan)?;
    if plan == expected {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            actual,
            "projection scan or fields changed",
        ))
    }
}

/// Inspect actual Rust macro bindings. This checks the lexical configuration at
/// each use, rather than treating an alias name as a trusted provider catalogue.
struct Scope<'a> {
    macros: BTreeMap<String, (usize, &'a syn::ItemMacro)>,
    used: std::cell::RefCell<BTreeSet<String>>,
    options: &'a PhysicalOptions,
    observed: bool,
}
impl<'a> Scope<'a> {
    fn new(
        block: &'a syn::Block,
        options: &'a PhysicalOptions,
        observed: bool,
    ) -> syn::Result<Self> {
        let mut macros = BTreeMap::new();
        let mut work_position = None;
        for (position, stmt) in block.stmts.iter().enumerate() {
            if let syn::Stmt::Local(local) = stmt {
                if cq_ir::cq::rust::bindings(&local.pat)
                    .iter()
                    .any(|id| symbol_name(id) == "__ml_work")
                {
                    let storage = &options.storage;
                    let expected: syn::Stmt =
                        syn::parse_quote!(let __ml_work = &#storage::Counters::default(););
                    if !observed || work_position.replace(position).is_some() || stmt != &expected {
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
                    || (observed && work_position.is_none())
                    || macros.insert(symbol_name(name), (position, item)).is_some()
                {
                    return Err(syn::Error::new_spanned(
                        item,
                        "invalid configured macro scope",
                    ));
                }
            }
        }
        if options
            .access
            .get_ident()
            .is_some_and(|name| macros.contains_key(&symbol_name(name)))
        {
            return Err(syn::Error::new_spanned(
                block,
                "configured macro shadows the selected Access continuation",
            ));
        }
        if observed && work_position.is_none() {
            return Err(syn::Error::new_spanned(
                block,
                "missing instrumentation binding",
            ));
        }
        Ok(Self {
            macros,
            used: Default::default(),
            options,
            observed,
        })
    }
    fn check(
        &self,
        alias: &syn::Path,
        selected: &syn::Path,
        keys: Option<&syn::Path>,
        recipe: bool,
        before: usize,
    ) -> syn::Result<()> {
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
        let storage = &self.options.storage;
        let access = recipe.then(|| {
            let access = &self.options.access;
            quote!(access #access;)
        });
        let keys = keys.map(|keys| quote!(keys #keys;));
        let work = self.observed.then(|| quote!(work (__ml_work);));
        let expected: syn::ItemMacro = syn::parse_quote! {
            macro_rules! #name {
                ($($source:tt)*) => {
                    #selected! { storage #storage; #access #keys #work $($source)* }
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
