//! Fuse the canonical left-deep RA into the existing local pull language.
//! Build owns only relation rows and indexes; Rust clauses run on cursor demand.

mod check;
pub use check::check as check_lazy_indexed;
mod coverage;
mod domain;
use crate::rust_bindings::{expression_aliases, local};
use cq_ir::index_requirements::signature;
use cq_ir::{
    cq, index_requirements as ir, relational_plan as ra, rust_access_plan as access, symbol_name,
};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

fn supports_lazy_indexed(source: &ir::Module) -> bool {
    let definitions = &source.relational_plan.plan.definitions;
    domain::is_linear_prefix(&source.relational_plan)
        && source.indexes.iter().all(|key| {
            key.result_token.is_some()
                && definitions.iter().any(|d| {
                    symbol_name(&d.result) == symbol_name(&key.relation)
                        && matches!(d.operator, ra::Operator::Rename(_))
                })
        })
}

/// Check the admitted plan, index namespace, and required lookup coverage.
pub fn check_domain(source: &ir::Module) -> syn::Result<()> {
    ir::contract::check(source)?;
    if !supports_lazy_indexed(source) {
        return Err(syn::Error::new_spanned(
            source,
            "lazy expression lowering requires indexes on direct renamed inputs",
        ));
    }
    coverage::check_coverage(source)
}

pub fn compile_lazy_indexed(source: &ir::Module, access: &syn::Path) -> syn::Result<syn::File> {
    check_domain(source)?;
    let module = &source.relational_plan;
    let info = ra::check_in(module.program.inputs(), &module.plan)?;
    let mut statements = Vec::new();
    let mut fields = Vec::new();
    let mut field_names = Vec::new();
    let mut query_bindings = Vec::new();
    let mut sources = BTreeMap::new();
    let all: BTreeSet<_> = info
        .values()
        .flat_map(|r| r.heading.iter().cloned())
        .collect();
    let vars: BTreeMap<_, _> = all
        .into_iter()
        .enumerate()
        .map(|(i, name)| (name, local("v", i)))
        .collect();
    let tuple_vars =
        |heading: &BTreeSet<String>| heading.iter().map(|v| vars[v].clone()).collect::<Vec<_>>();
    let input_names: Vec<_> = (0..module.program.inputs().count())
        .map(|i| local("input", i))
        .collect();
    let parameters: Vec<_> = module
        .program
        .inputs()
        .zip(&input_names)
        .map(|(decl, id)| {
            let types = decl.columns.iter().map(|c| &c.ty);
            quote!(#id: impl ::core::iter::IntoIterator<Item = (#(#types,)*)>)
        })
        .collect();
    let used: BTreeSet<_> = module
        .plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Rename(r) => Some(symbol_name(&r.relation)),
            _ => None,
        })
        .collect();
    for (decl, id) in module.program.inputs().zip(&input_names) {
        if used.contains(&symbol_name(&decl.name)) {
            let types = decl.columns.iter().map(|c| &c.ty);
            statements.push(quote!(let #id: ::std::collections::HashSet<(#(#types,)*)> = #id.into_iter().collect();));
        }
    }
    for (i, d) in module.plan.definitions.iter().enumerate() {
        let ra::Operator::Rename(rename) = &d.operator else {
            continue;
        };
        let id = local("rows", i);
        let name = symbol_name(&d.result);
        let relation = &info[&name];
        let (input_i, decl) = module
            .program
            .inputs()
            .enumerate()
            .find(|(_, d)| symbol_name(&d.name) == symbol_name(&rename.relation))
            .unwrap();
        let input = &input_names[input_i];
        let types: Vec<_> = relation
            .heading
            .iter()
            .map(|v| &relation.types[v])
            .collect();
        let values = relation.heading.iter().map(|v| {
            let mapping = rename
                .mapping
                .iter()
                .find(|m| symbol_name(&m.target) == *v)
                .unwrap();
            let index = syn::Index::from(
                decl.columns
                    .iter()
                    .position(|c| symbol_name(&c.name) == symbol_name(&mapping.source))
                    .unwrap(),
            );
            quote!(::core::clone::Clone::clone(&row.#index))
        });
        statements.push(quote!(let #id: ::std::collections::HashSet<(#(#types,)*)> = #input.iter().map(|row| (#(#values,)*)).collect();));
        fields.push(quote!(#id: ::std::collections::HashSet<(#(#types,)*)>));
        field_names.push(id.clone());
        query_bindings.push(quote!(let #id = &self.#id;));
        sources.insert(name, id);
    }
    let mut keys = BTreeMap::new();
    for (i, requirement) in source.indexes.iter().enumerate() {
        let id = local("index", i);
        let key = signature(requirement);
        let relation = &info[&key.1];
        let rows = &sources[&key.1];
        let heading: Vec<_> = relation.heading.iter().collect();
        let key_types: Vec<_> = key.2.iter().map(|&i| &relation.types[heading[i]]).collect();
        let key_values: Vec<_> = key
            .2
            .iter()
            .map(|&i| {
                let i = syn::Index::from(i);
                quote!(::core::clone::Clone::clone(&row.#i))
            })
            .collect();
        let payload: Vec<_> = (0..heading.len()).filter(|i| !key.2.contains(i)).collect();
        if payload.is_empty() {
            statements.push(quote!(let #id: ::std::collections::HashSet<(#(#key_types,)*)> = #rows.iter().map(|row| (#(#key_values,)*)).collect();));
            fields.push(quote!(#id: ::std::collections::HashSet<(#(#key_types,)*)>));
        } else {
            let types: Vec<_> = payload
                .iter()
                .map(|&i| &relation.types[heading[i]])
                .collect();
            let values: Vec<_> = payload
                .iter()
                .map(|&i| {
                    let i = syn::Index::from(i);
                    quote!(::core::clone::Clone::clone(&row.#i))
                })
                .collect();
            let ty = quote!(::std::collections::HashMap<(#(#key_types,)*), ::std::vec::Vec<(#(#types,)*)>>);
            statements.push(quote!(let mut #id: #ty = ::std::collections::HashMap::new(); for row in &#rows { #id.entry((#(#key_values,)*)).or_default().push((#(#values,)*)); }));
            fields.push(quote!(#id: #ty));
        }
        field_names.push(id.clone());
        query_bindings.push(quote!(let #id = &self.#id;));
        keys.insert(key, id);
    }
    let expression = |expr: &syn::Expr, relation: &ra::RelationInfo| {
        if let Some(p) = cq::Predicate::from_expr(expr)
            .filter(|p| p.variables().iter().all(|v| relation.heading.contains(v)))
        {
            let term = |t: &cq::Term| match t {
                cq::Term::Integer(n) => quote!(&#n),
                cq::Term::Variable(v) => {
                    let name = symbol_name(v);
                    let id = &vars[&name];
                    if relation.computed.contains(&name) {
                        quote!(&#id)
                    } else {
                        quote!(#id)
                    }
                }
            };
            let left = term(&p.left);
            let right = term(&p.right);
            let op = &p.comparison;
            return quote!(#left #op #right);
        }
        let aliases = expression_aliases(expr, relation, |name| {
            let id = &vars[name];
            if relation.computed.contains(name) {
                quote!(&#id)
            } else {
                quote!(#id)
            }
        });
        quote!({ #aliases #expr })
    };
    let mut clauses = Vec::<access::Clause>::new();
    let mut current = None::<String>;
    for d in &module.plan.definitions {
        let name = symbol_name(&d.result);
        match &d.operator {
            ra::Operator::Unit(_) if current.is_none() => {
                clauses.push(syn::parse_quote!(unit() => if (true)));
                current = Some(name);
            }
            ra::Operator::Rename(_) if current.is_none() => {
                let rows = &sources[&name];
                let variables = tuple_vars(&info[&name].heading);
                clauses.push(syn::parse_quote!(rows(#(#variables),*) => for (#(#variables,)*) in (#rows.iter())));
                current = Some(name);
            }
            ra::Operator::Rename(_) => (),
            ra::Operator::NaturalJoin(join)
                if current.as_deref() == Some(&symbol_name(&join.left)) =>
            {
                let left = &info[&symbol_name(&join.left)];
                let right = &info[&symbol_name(&join.right)];
                let shared: BTreeSet<_> =
                    left.heading.intersection(&right.heading).cloned().collect();
                let fresh = tuple_vars(&right.heading.difference(&left.heading).cloned().collect());
                let right_vars = tuple_vars(&right.heading);
                if shared.is_empty() {
                    let rows = &sources[&symbol_name(&join.right)];
                    clauses.push(syn::parse_quote!(rows(#(#right_vars),*) => for (#(#fresh,)*) in (#rows.iter())));
                } else {
                    let index = &keys[&(
                        true,
                        symbol_name(&join.right),
                        ra::shared_columns(
                            &info[&symbol_name(&join.left)],
                            &info[&symbol_name(&join.right)],
                        ),
                    )];
                    let values: Vec<_> = shared
                        .iter()
                        .map(|v| {
                            let id = &vars[v];
                            if left.computed.contains(v) {
                                quote!(#id.clone())
                            } else {
                                quote!(::core::clone::Clone::clone(#id))
                            }
                        })
                        .collect();
                    if fresh.is_empty() {
                        clauses.push(syn::parse_quote!(key(#(#right_vars),*) => if (#index.contains(&(#(#values,)*)))));
                    } else {
                        clauses.push(syn::parse_quote!(rows(#(#right_vars),*) => for (#(#fresh,)*) in (#index.get(&(#(#values,)*)).into_iter().flatten())));
                    }
                }
                current = Some(name);
            }
            ra::Operator::Select(select)
                if current.as_deref() == Some(&symbol_name(&select.input)) =>
            {
                let expr = expression(&select.condition, &info[&symbol_name(&select.input)]);
                clauses.push(syn::parse_quote!(if (#expr) => if (#expr)));
                current = Some(name);
            }
            ra::Operator::Extend(extend)
                if current.as_deref() == Some(&symbol_name(&extend.input)) =>
            {
                let expr = expression(&extend.value, &info[&symbol_name(&extend.input)]);
                let fresh: Vec<_> = extend
                    .columns
                    .iter()
                    .map(|c| &vars[&symbol_name(&c.name)])
                    .collect();
                clauses.push(syn::parse_quote!(let (#(#fresh,)*) = (#expr) => for (#(#fresh,)*) in (::core::iter::once(#expr))));
                current = Some(name);
            }
            ra::Operator::Project(p)
                if current.as_deref() == Some(&symbol_name(&p.input))
                    && d.result == module.plan.output.input =>
            {
                current = Some(name);
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    d,
                    "lazy expression lowering requires a linear prefix and final projection",
                ));
            }
        }
    }
    let final_info = &info[&symbol_name(&module.plan.output.input)];
    let head_vars = tuple_vars(&final_info.heading);
    let output_fields: Vec<_> = module
        .plan
        .output
        .head
        .args
        .iter()
        .map(|e| {
            if let Some(v) =
                cq::rust::variable(e).filter(|v| final_info.heading.contains(&symbol_name(v)))
            {
                let id = &vars[&symbol_name(v)];
                quote!(#id.clone())
            } else {
                expression(e, final_info)
            }
        })
        .collect();
    let plan: access::Plan =
        syn::parse2(quote!(output(#(#head_vars),*) => ((#(#output_fields,)*)) :- #(#clauses),*;))?;
    access::contract::check(&plan)?;
    let output_types = ra::output_types(&module.program, final_info)?;
    let row = quote!((#(#output_types,)*));
    let visibility = &module.program.visibility;
    let program = &module.program.name;
    let storage = format_ident!("{}Storage", program);
    let target = syn::parse2(quote! {
        #visibility struct #program;
        #visibility struct #storage { #(#fields,)* }
        impl #program {
            pub fn build(#(#parameters),*) -> #storage { #(#statements)* #storage { #(#field_names,)* } }
            pub fn run(#(#parameters),*) -> ::std::vec::Vec<#row> { Self::build(#(#input_names),*).materialize() }
        }
        impl #storage {
            pub fn query(&self) -> impl ::core::iter::Iterator<Item = #row> + '_ {
                #(#query_bindings)* #access! { #plan }
            }
            pub fn materialize(&self) -> ::std::vec::Vec<#row> { let mut rows: ::std::vec::Vec<_> = self.query().collect(); rows.sort_unstable(); rows }
        }
    })?;
    check_lazy_indexed(source, &target, access)?;
    Ok(target)
}

#[cfg(test)]
mod tests;
