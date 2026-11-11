//! Checked GPU-owned source adaptation, before ordinary CQ normalization.
use cq_lang_datalog_round::cq::{self, BodyItem, Program};
use cq_syntax::{rust, symbol_name, RelationDecl};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;
use syn::{Expr, visit_mut::VisitMut};

/// Adapter arguments are existing relation declarations, not a new IR record.
pub fn compile(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    struct Declarations(Vec<RelationDecl>);
    impl syn::parse::Parse for Declarations {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let mut declarations = Vec::new();
            while !input.is_empty() { declarations.push(input.parse()?); }
            Ok(Self(declarations))
        }
    }
    let Declarations(mut declarations) = syn::parse2(arguments)?;
    if declarations.len() != source.declarations.len() {
        return Err(syn::Error::new_spanned(source, "domain adapter must cover every hole declaration"));
    }
    for (logical, encoded) in source.declarations.iter().zip(&mut declarations) {
        if logical.name != encoded.name || logical.arity() != encoded.arity()
            || logical.columns.iter().zip(&encoded.columns).any(|(left, right)| {
                left.name != right.name || (right.domain.is_none() && left.ty != right.ty)
                    || (right.domain.is_some() && right.ty != syn::parse_quote!(u32))
            }) {
            return Err(syn::Error::new_spanned(encoded, "domain adapter changed the logical heading instead of its representation"));
        }
        encoded.value = logical.value.clone();
        encoded.hosts = logical.hosts.clone();
    }
    let mut target = source.clone();
    target.declarations = declarations;
    // Representation declarations are a fresh logical-type interning boundary.
    for column in target.declarations.iter_mut().flat_map(|declaration| &mut declaration.columns) {
        column.logical_type_id = None;
    }
    cq_syntax::intern_logical_types(&mut target.declarations)?;
    cq_syntax::intern_domains(&mut target.declarations)?;
    let positions: BTreeMap<_, _> = target.declarations.iter().map(|declaration| (symbol_name(&declaration.name), declaration)).collect();
    let mut domains = BTreeMap::new();
    let mut atom = |atom: &cq_syntax::Atom| -> syn::Result<()> {
        let declaration = positions.get(&symbol_name(&atom.relation))
            .ok_or_else(|| syn::Error::new_spanned(atom, "domain adapter atom has no declaration"))?;
        for (argument, column) in atom.args.iter().zip(&declaration.columns) {
            if let (Some(variable), Some(domain)) = (rust::variable(argument), &column.domain) {
                if let Some(previous) = domains.insert(symbol_name(variable), domain.clone())
                    && previous.id != domain.id {
                    return Err(syn::Error::new_spanned(argument, "compared symbol columns belong to different domain components"));
                }
            }
        }
        Ok(())
    };
    atom(&target.query.head)?;
    for item in &target.query.body {
        match item {
            BodyItem::Positive { atom: input } => atom(input)?,
            BodyItem::Negation(negative) => atom(&negative.atom)?,
            _ => {}
        }
    }
    let reads: std::collections::BTreeSet<_> = domains.keys().cloned().collect();
    struct Restore<'a> { domains: &'a BTreeMap<String, cq_syntax::CodeDomain>, reads: std::collections::BTreeSet<String> }
    impl VisitMut for Restore<'_> {
        fn visit_expr_mut(&mut self, expression: &mut Expr) {
            if let Some(variable) = rust::variable(expression)
                && self.reads.contains(&symbol_name(variable)) {
                let domain = &self.domains[&symbol_name(variable)].expression;
                // DECODE R: only free symbol variables actually read by opaque Rust are reconstructed.
                *expression = syn::parse_quote!(&(#domain).decode(*#variable).expect("invalid symbol code at Rust boundary"));
                return;
            }
            syn::visit_mut::visit_expr_mut(self, expression);
        }
    }
    let restore = |expression: &mut Expr| {
        let dependencies = rust::free_variables(expression).dependencies(&reads);
        Restore { domains: &domains, reads: dependencies }.visit_expr_mut(expression);
    };
    for item in &mut target.query.body {
        match item {
            BodyItem::Let(binding) => {
                restore(&mut binding.expression);
                let variables = rust::bindings(&binding.pattern);
                if let [variable] = variables.as_slice()
                    && let Some(domain) = domains.get(&symbol_name(variable)) {
                    let value = &binding.expression;
                    let domain = &domain.expression;
                    binding.expression = syn::parse_quote!((#domain).encode_required(&(#value)).expect("created symbol is absent from its frozen result domain"));
                    if let Some(annotation) = &mut binding.annotation { annotation.ty = syn::parse_quote!(u32); }
                }
            }
            BodyItem::Filter(guard) => restore(&mut guard.condition),
            BodyItem::For(binding) => restore(&mut binding.expression),
            _ => {}
        }
    }
    let output = positions[&symbol_name(&target.query.head.relation)];
    for (value, column) in target.query.head.args.iter_mut().zip(&output.columns) {
        if rust::variable(value).is_none() {
            restore(value);
            if let Some(domain) = &column.domain {
                let domain = &domain.expression;
                *value = syn::parse_quote!((#domain).encode_required(&(#value)).expect("head symbol is absent from its frozen result domain"));
            }
        }
    }
    cq::contract::check(&target)?;
    Ok(target)
}

/// Representation declarations consumed by the adapter and binding checker.
pub fn declarations(
    source: &Program,
    components: &BTreeMap<String, Vec<u32>>,
    symbols: &crate::storage::Symbols,
) -> syn::Result<Vec<RelationDecl>> {
    let mut declarations = source.declarations.clone();
    for declaration in &mut declarations {
        let ids = components.get(&symbol_name(&declaration.name))
            .ok_or_else(|| syn::Error::new_spanned(&*declaration, "missing hole domain components"))?;
        if ids.len() != declaration.arity() { return Err(syn::Error::new_spanned(&*declaration, "hole domain arity differs")); }
        for (column, &id) in declaration.columns.iter_mut().zip(ids) {
            if let Some(codec) = symbols.codec(column.logical_type_id) {
                let logical = &column.ty;
                column.domain = Some(syn::parse2(quote!(in [#id] (__cq_gpu_domains.domain::<#logical, #codec>(#id))))?);
                column.ty = syn::parse_quote!(u32);
            }
            column.logical_type_id = None;
        }
    }
    cq_syntax::intern_logical_types(&mut declarations)?;
    cq_syntax::intern_domains(&mut declarations)?;
    Ok(declarations)
}
