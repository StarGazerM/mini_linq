//! Independently check RA versions, physical fields, native Access operations,
//! resource dominance and lexical configuration. Provider laws and arbitrary
//! Rust bodies remain conditional; this is a wiring correspondence audit.
use super::PhysicalOptions;
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
    ra::contract::check(source)?;
    options.validate(source)?;
    check_method(source, target, options, false)?;
    check_method(source, target, options, true)
}
fn check_method(
    source: &ra::Module,
    target: &syn::File,
    options: &PhysicalOptions,
    observed: bool,
) -> syn::Result<()> {
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
    let fail = || {
        syn::Error::new_spanned(
            target,
            "physical Access wiring violated: provider, current version, fields, resource or definition order changed",
        )
    };
    let program = &source.program.name;
    let ty: syn::Type = syn::parse_quote!(#program);
    let methods: Vec<_> = target
        .items
        .iter()
        .filter_map(|i| match i {
            syn::Item::Impl(i) if *i.self_ty == ty => Some(i),
            _ => None,
        })
        .flat_map(|i| &i.items)
        .filter_map(|i| match i {
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
    for (position, stmt) in method.block.stmts.iter().enumerate() {
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
    let mut demands = BTreeMap::<(String, Vec<String>, String, String), usize>::new();
    let mut previous = None;
    let mut materializations = 0;
    for (index, definition) in source.plan.definitions.iter().enumerate() {
        let result = symbol_name(&definition.result);
        let name = format_ident!("__ml_view{index}");
        let (position, binding) = bindings.get(&name.to_string()).ok_or_else(fail)?;
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
                provider = scope.resolve(
                    &options.inputs[&symbol_name(&rename.relation)],
                    None,
                    false,
                    *position,
                )?;
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
                let selected_keys = options
                    .key_providers
                    .get(&result)
                    .unwrap_or(&options.key_provider);
                let keys_type = options.key_sets.get(&result).unwrap_or(&options.keys);
                let demand = (
                    symbol_name(&op.right),
                    shared.clone(),
                    selected_keys.to_token_stream().to_string(),
                    keys_type.to_token_stream().to_string(),
                );
                let new = !demands.contains_key(&demand);
                let next = demands.len();
                let id = *demands.entry(demand).or_insert(next);
                let resource = format_ident!("__ml_resource{id}");
                let (rp, rb) = bindings.get(&resource.to_string()).ok_or_else(fail)?;
                if *rp <= right.position
                    || *rp >= *position
                    || (new && previous.is_some_and(|p| *rp <= p))
                    || (!new && previous.is_some_and(|p| *rp >= p))
                {
                    return Err(fail());
                }
                let key_macro = scope.resolve(selected_keys, Some(keys_type), true, *rp)?;
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
                    None,
                    true,
                    *position,
                )?;
                let lp = &left.provider;
                let l = &left.name;
                let next = &options.iterator;
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
                    None,
                    false,
                    *position,
                )?;
                let Binding::Materialized(actual, operation) = binding else {
                    return Err(fail());
                };
                let provider::Operation::Materialize {
                    owner,
                    value,
                    rows,
                    block_size,
                    ..
                } = &**operation
                else {
                    return Err(fail());
                };
                if *actual != provider
                    || *owner != format_ident!("__ml_owner{index}")
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
                    ra::Operator::Project(op) => {
                        let input = views.get(&symbol_name(&op.input)).ok_or_else(fail)?;
                        let fields = positions(&input.columns, &columns).ok_or_else(fail)?;
                        let width = fields.len();
                        let (p, v, next) = (&input.provider, &input.name, &options.access);
                        let plan: access::Plan = syn::parse_quote! {
                            out(__row) => ({ let __projected: [::std::primitive::i64; #width] = [#(__row[#fields]),*]; __projected }) :-
                            Rows(__row) => for (__row,) in (#p! { scan (&#v) }.map(|row| (row,)));
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
                            None,
                            true,
                            *position,
                        )?;
                        let build: access::Plan = syn::parse_quote! {
                            out(__row) => ({ let __key: [::std::primitive::i64; #key_width] = [#(__row[#rk]),*]; (__key, __row) }) :-
                            Rows(__row) => for (__row,) in (#rp! { scan (__join_right) }.map(|row| (row,)));
                        };
                        let plan: access::Plan = syn::parse_quote! {
                            out(__left_row,__right_row) => ({ let __joined: [::std::primitive::i64; #width] = [#(#fields),*]; __joined }) :-
                            Left(__left_row) => for (__left_row,) in (#lp! { scan (__join_left) }.map(|row| (row,))),
                            Right(__right_row) => for (__right_row,) in (#witness! { probe (__join_resource_ref) ([#(__left_row[#lk]),*]) }.map(|row| (row,)));
                        };
                        access::contract::check(&build)?;
                        access::contract::check(&plan)?;
                        let expected: syn::Expr = syn::parse_quote!({
                            let __join_left = &#l; let __join_right = &#r;
                            let __join_resource = #witness! { build { #build } };
                            let __join_resource_ref = &__join_resource;
                            #next! { #plan }
                        });
                        if **rows != expected {
                            return Err(fail());
                        }
                    }
                    _ => unreachable!(),
                }
            }
            _ => return Err(fail()),
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
        if [&options.access, &options.iterator].iter().any(|path| {
            path.get_ident()
                .is_some_and(|name| macros.contains_key(&symbol_name(name)))
        }) {
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
        leaf: bool,
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
        let iterator = leaf.then(|| {
            let iterator = &self.options.iterator;
            quote!(iterator #iterator;)
        });
        let keys = keys.map(|keys| quote!(keys #keys;));
        let work = self.observed.then(|| quote!(work (__ml_work);));
        let expected: syn::ItemMacro = syn::parse_quote! {
            macro_rules! #name {
                ($($source:tt)*) => {
                    #selected! { storage #storage; #iterator #keys #work $($source)* }
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
    fn resolve(
        &self,
        selected: &syn::Path,
        keys: Option<&syn::Path>,
        leaf: bool,
        before: usize,
    ) -> syn::Result<syn::Path> {
        let mut candidates = Vec::new();
        for name in self.macros.keys() {
            let alias: syn::Path = syn::parse_str(name)?;
            if self.check(&alias, selected, keys, leaf, before).is_ok() {
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
