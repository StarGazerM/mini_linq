//! IndexRequirements -> staged Rust. Storage and accesses are concrete Rust;
//! every named set result is an existing RustAccessPlan hole consumed by pull.
use crate::relational_to_indexes::{self as indexes, Signature};
use cq_ir::{
    cq, index_requirements as ir, relational_plan as ra, rust_access_plan as access, symbol_name,
};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

type Information = BTreeMap<String, ra::RelationInfo>;
type Names = BTreeMap<String, syn::Ident>;

pub(crate) fn compile(source: &ir::Module) -> syn::Result<syn::File> {
    let ra = &source.relational_plan;
    let info = ra::check_in(&ra.program.inputs, &ra.plan)?;
    if info.values().any(|r| r.heading.len() > 12) {
        return Err(syn::Error::new_spanned(
            ra,
            "Rust tuple set storage supports at most 12 attributes per result",
        ));
    }
    let results: Names = ra
        .plan
        .definitions
        .iter()
        .enumerate()
        .map(|(i, d)| (symbol_name(&d.result), local("relation", i)))
        .collect();
    let inputs: Names = ra
        .program
        .inputs
        .iter()
        .enumerate()
        .map(|(i, d)| (symbol_name(&d.name), local("input", i)))
        .collect();
    let mut required: BTreeMap<Signature, (syn::Ident, bool)> = source
        .indexes
        .iter()
        .enumerate()
        .map(|(i, k)| (indexes::signature(k), (local("index", i), false)))
        .collect();
    for d in &ra.plan.definitions {
        if let ra::Operator::NaturalJoin(j) = &d.operator {
            let key = indexes::columns(&info, &j.left, &j.right);
            if !key.is_empty() && key.len() < info[&symbol_name(&j.right)].heading.len() {
                required
                    .get_mut(&(true, symbol_name(&j.right), key))
                    .expect("checked index coverage")
                    .1 = true;
            }
        }
    }
    let mut statements = Vec::new();
    let used: BTreeSet<_> = ra
        .plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Rename(r) => Some(symbol_name(&r.relation)),
            _ => None,
        })
        .chain(
            source
                .indexes
                .iter()
                .filter(|k| k.result_token.is_none())
                .map(|k| symbol_name(&k.relation)),
        )
        .collect();
    for input in &ra.program.inputs {
        let name = symbol_name(&input.name);
        if used.contains(&name) {
            let id = &inputs[&name];
            let types = input.columns.iter().map(|c| &c.ty);
            statements.push(quote! { let #id: ::std::collections::HashSet<(#(#types,)*)> = #id.into_iter().collect(); });
            for (key, (index, payload)) in &required {
                if !key.0 && key.1 == name {
                    statements.push(build_index(index, id, input.arity(), &key.2, *payload));
                }
            }
        }
    }
    for d in &ra.plan.definitions {
        let name = symbol_name(&d.result);
        let id = &results[&name];
        let types = info[&name].heading.iter().map(|v| &info[&name].types[v]);
        let (bindings, plan) = local_plan(d, &info, &results, &inputs, &required, &ra.program)?;
        let arity = info[&name].heading.len();
        statements.push(quote! {
            let #id: ::std::collections::HashSet<(#(#types,)*)> = {
                #bindings
                ::core::iter::Iterator::collect(::mini_linq::pull! { #plan })
            };
            __stats.results.push((#name, #arity, #id.len()));
        });
        for (key, (index, payload)) in &required {
            if key.0 && key.1 == name {
                statements.push(build_index(index, id, arity, &key.2, *payload));
            }
        }
    }
    let output_name = symbol_name(&ra.plan.output.input);
    let output = &results[&output_name];
    let output_info = &info[&output_name];
    let output_types: Vec<_> = ra
        .plan
        .output
        .head
        .variables
        .iter()
        .map(|v| &output_info.types[&symbol_name(v)])
        .collect();
    let fields = ra.plan.output.head.variables.iter().map(|v| {
        let i = syn::Index::from(
            output_info
                .heading
                .iter()
                .position(|x| *x == symbol_name(v))
                .unwrap(),
        );
        quote!(::core::clone::Clone::clone(&__row.#i))
    });
    let parameters: Vec<_> = ra
        .program
        .inputs
        .iter()
        .map(|d| {
            let name = &inputs[&symbol_name(&d.name)];
            let types = d.columns.iter().map(|c| &c.ty);
            quote!(#name: impl ::core::iter::IntoIterator<Item = (#(#types,)*)>)
        })
        .collect();
    let arguments: Vec<_> = ra
        .program
        .inputs
        .iter()
        .map(|d| &inputs[&symbol_name(&d.name)])
        .collect();
    let visibility = &ra.program.visibility;
    let program = &ra.program.name;
    let storage = format_ident!("{}Storage", program, span = Span::mixed_site());
    let stats = format_ident!("{}ExecutionStats", program, span = Span::mixed_site());
    let row = quote!((#(#output_types,)*));
    syn::parse2(quote! {
        #visibility struct #program;
        #visibility struct #storage { answer: ::std::collections::HashSet<#row>, stats: #stats }
        #[derive(Debug, Default)]
        #visibility struct #stats {
            pub results: ::std::vec::Vec<(&'static str, usize, usize)>,
            pub index_rows: usize,
            pub join_pairs: usize,
            pub semijoin_probes: usize,
        }
        impl #program {
            pub fn build(#(#parameters),*) -> #storage {
                let mut __stats = #stats::default();
                let __pairs = ::core::cell::Cell::new(0usize);
                let __probes = ::core::cell::Cell::new(0usize);
                #(#statements)*
                __stats.join_pairs = __pairs.get();
                __stats.semijoin_probes = __probes.get();
                let answer = #output.iter().map(|__row| (#(#fields,)*)).collect();
                #storage { answer, stats: __stats }
            }
            pub fn run(#(#parameters),*) -> ::std::vec::Vec<#row> {
                Self::build(#(#arguments),*).materialize()
            }
            pub fn run_with_stats(#(#parameters),*) -> (::std::vec::Vec<#row>, #stats) {
                let storage = Self::build(#(#arguments),*);
                (storage.materialize(), storage.stats)
            }
        }
        impl #storage {
            pub fn query(&self) -> impl ::core::iter::Iterator<Item = #row> + '_ { self.answer.iter().cloned() }
            pub fn materialize(&self) -> ::std::vec::Vec<#row> {
                let mut rows: ::std::vec::Vec<_> = self.query().collect(); rows.sort_unstable(); rows
            }
            pub fn stats(&self) -> &#stats { &self.stats }
        }
    })
}

fn local(prefix: &str, i: usize) -> syn::Ident {
    format_ident!("{prefix}{i}", span = Span::mixed_site())
}

fn build_index(
    index: &syn::Ident,
    input: &syn::Ident,
    arity: usize,
    key: &[usize],
    payload: bool,
) -> TokenStream {
    if key.len() == arity {
        // The normalized full row set already is this membership index.
        return quote!(let #index = &#input;);
    }
    let fields = key.iter().map(|i| {
        let i = syn::Index::from(*i);
        quote!(::core::clone::Clone::clone(&__row.#i))
    });
    let key_value = quote!((#(#fields,)*));
    if payload {
        let fields = (0..arity).filter(|i| !key.contains(i)).map(|i| {
            let i = syn::Index::from(i);
            quote!(::core::clone::Clone::clone(&__row.#i))
        });
        quote! {
            let mut #index = ::std::collections::HashMap::<_, ::std::vec::Vec<_>>::new();
            for __row in &#input { #index.entry(#key_value).or_default().push((#(#fields,)*)); }
            __stats.index_rows += #input.len();
        }
    } else {
        quote! {
            let #index: ::std::collections::HashSet<_> = #input.iter().map(|__row| #key_value).collect();
            __stats.index_rows += #input.len();
        }
    }
}

/// Each local region is typed and checked before it is quoted into the host.
/// It describes accesses to row sets or projected key sets, not a new CQ plan.
fn local_plan(
    definition: &ra::Definition,
    info: &Information,
    results: &Names,
    inputs: &Names,
    indexes: &BTreeMap<Signature, (syn::Ident, bool)>,
    program: &cq::Program,
) -> syn::Result<(TokenStream, access::Plan)> {
    let all: BTreeSet<_> = info.values().flat_map(|r| r.heading.iter()).collect();
    let variables: Names = all
        .into_iter()
        .enumerate()
        .map(|(i, v)| (v.clone(), local("v", i)))
        .collect();
    let vars = |heading: &BTreeSet<String>| {
        heading
            .iter()
            .map(|v| variables[v].clone())
            .collect::<Vec<_>>()
    };
    let mut bindings = TokenStream::new();
    let mut clauses = Vec::<access::Clause>::new();
    let result_name = symbol_name(&definition.result);
    let head = vars(&info[&result_name].heading);
    let fields = head.iter().map(|v| quote!(::core::clone::Clone::clone(#v)));
    let mut value = quote!((#(#fields,)*));
    match &definition.operator {
        ra::Operator::Unit(_) => clauses.push(syn::parse_quote!(unit() => if (true))),
        ra::Operator::Rename(r) => {
            let input = &inputs[&symbol_name(&r.relation)];
            let decl = program
                .inputs
                .iter()
                .find(|d| symbol_name(&d.name) == symbol_name(&r.relation))
                .unwrap();
            let ordered: Vec<_> = decl
                .columns
                .iter()
                .map(|c| {
                    let m = r
                        .mapping
                        .iter()
                        .find(|m| symbol_name(&m.source) == symbol_name(&c.name))
                        .unwrap();
                    variables[&symbol_name(&m.target)].clone()
                })
                .collect();
            bindings.extend(quote!(let __left = &#input;));
            clauses.push(scan(&ordered, quote!(__left)));
        }
        ra::Operator::Project(p) => {
            let input = &results[&symbol_name(&p.input)];
            bindings.extend(quote!(let __left = &#input;));
            clauses.push(scan(
                &vars(&info[&symbol_name(&p.input)].heading),
                quote!(__left),
            ));
        }
        ra::Operator::Select(s) => {
            let input = &results[&symbol_name(&s.input)];
            bindings.extend(quote!(let __left = &#input;));
            clauses.push(scan(
                &vars(&info[&symbol_name(&s.input)].heading),
                quote!(__left),
            ));
            let mut condition = s.condition.clone();
            for term in [&mut condition.left, &mut condition.right] {
                if let cq::Term::Variable(v) = term {
                    *v = variables[&symbol_name(v)].clone();
                }
            }
            let expression = predicate(&condition);
            clauses.push(syn::parse_quote!(if (#condition) => if (#expression)));
        }
        ra::Operator::NaturalJoin(_) | ra::Operator::Semijoin(_) => {
            let (left, right) = indexes::operands(&definition.operator).unwrap();
            let ln = symbol_name(left);
            let rn = symbol_name(right);
            let l = &results[&ln];
            let r = &results[&rn];
            let lh = &info[&ln].heading;
            let rh = &info[&rn].heading;
            let shared: BTreeSet<_> = lh.intersection(rh).cloned().collect();
            let key = vars(&shared);
            let key_fields = key.iter().map(|v| quote!(::core::clone::Clone::clone(#v)));
            let key_value = quote!((#(#key_fields,)*));
            bindings.extend(quote!(let __left = &#l;));
            if shared.is_empty() {
                bindings.extend(quote!(let __right = &#r;));
            }
            clauses.push(scan(&vars(lh), quote!(__left)));
            let index = if shared.is_empty() {
                None
            } else {
                let requirement = (true, rn.clone(), indexes::columns(info, left, right));
                let (id, payload) = &indexes[&requirement];
                bindings.extend(quote!(let __index = &#id;));
                Some(*payload)
            };
            let exists = match index {
                None => quote!(!__right.is_empty()),
                Some(true) => quote!(__index.contains_key(&#key_value)),
                Some(false) => quote!(__index.contains(&#key_value)),
            };
            if matches!(&definition.operator, ra::Operator::Semijoin(_)) {
                bindings.extend(quote!(let __probes_ref = &__probes;));
                clauses.push(syn::parse_quote!(key(#(#key),*) => if ({ __probes_ref.set(__probes_ref.get()+1); #exists })));
            } else {
                let fresh = vars(&rh.difference(lh).cloned().collect());
                if fresh.is_empty() {
                    clauses.push(syn::parse_quote!(key(#(#key),*) => if (#exists)));
                } else {
                    let right_vars = vars(rh);
                    let source = if index.is_some() {
                        quote!(__index.get(&#key_value).into_iter().flatten())
                    } else {
                        quote!(__right.iter())
                    };
                    clauses.push(
                        syn::parse_quote!(rows(#(#right_vars),*) => for (#(#fresh,)*) in (#source)),
                    );
                }
                bindings.extend(quote!(let __pairs_ref = &__pairs;));
                value = quote!({ __pairs_ref.set(__pairs_ref.get()+1); #value });
            }
        }
    }
    let plan: access::Plan = syn::parse2(quote!(output(#(#head),*) => (#value) :- #(#clauses),*.))?;
    access::contract::check(&plan)?;
    Ok((bindings, plan))
}

fn scan(variables: &[syn::Ident], source: TokenStream) -> access::Clause {
    if variables.is_empty() {
        syn::parse_quote!(rows() => if (!#source.is_empty()))
    } else {
        syn::parse_quote!(rows(#(#variables),*) => for (#(#variables,)*) in (#source.iter()))
    }
}

fn predicate(p: &cq::Predicate) -> TokenStream {
    let term = |t: &cq::Term| match t {
        cq::Term::Integer(n) => quote!(#n),
        cq::Term::Variable(v) => quote!({ let value: ::core::primitive::i32 = *#v; value }),
    };
    let left = term(&p.left);
    let right = term(&p.right);
    let cmp = &p.comparison;
    quote!(#left #cmp #right)
}
