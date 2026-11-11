//! GPU-owned equality-component seam data; no new Datalog or CQ grammar.
use cq_lang_datalog_storage::incremental::strata::source::{BodyItem, Program};
use cq_syntax::{rust, symbol_name};
use std::collections::BTreeMap;
use quote::quote;

/// Emit load-time relation membership assignments and creating-head constants.
/// The same source rules feed execution and this schema callback.
pub fn schema(input: proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    struct Input { symbols: crate::storage::Symbols, source: Program }
    impl syn::parse::Parse for Input {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let name: syn::Ident = input.parse()?;
            if name != "symbols" { return Err(syn::Error::new_spanned(name, "expected symbols(...) before domain schema")); }
            let content;
            syn::parenthesized!(content in input);
            let tokens: proc_macro2::TokenStream = content.parse()?;
            let symbols = syn::parse2(quote!(symbols(#tokens)))?;
            input.parse::<syn::Token![;]>()?;
            Ok(Self { symbols, source: input.parse()? })
        }
    }
    let Input { mut symbols, source } = syn::parse2(input)?;
    let mut declarations = source.declarations.clone();
    let mut advice: cq_syntax::RelationDecl = syn::parse_quote!(relation __cq_domain_symbols(););
    for symbol in &symbols.0 {
        let ty = &symbol.logical;
        let mut column: cq_syntax::ColumnDecl = syn::parse_quote!(__cq_symbol: #ty);
        column.logical_type_id = symbol.logical_type_id;
        advice.columns.push(column);
    }
    declarations.push(advice);
    cq_syntax::intern_logical_types(&mut declarations)?;
    for (symbol, column) in symbols.0.iter_mut().zip(&declarations.last().unwrap().columns) {
        symbol.logical_type_id = column.logical_type_id;
    }
    let components = DomainComponents::infer(&source)?;
    let entries = source.declarations.iter().enumerate().map(|(index, relation)| {
        let name = symbol_name(&relation.name);
        let ids = &components.columns[index];
        quote!((#name, &[#(#ids),*] as &[u32]))
    });
    let positions: BTreeMap<_, _> = source.declarations.iter().enumerate()
        .map(|(index, relation)| (symbol_name(&relation.name), index)).collect();
    let mut required = Vec::new();
    for rule in &source.rules {
        let relation = positions[&symbol_name(&rule.head.relation)];
        for (column, value) in rule.head.args.iter().enumerate() {
            if rust::variable(value).is_some() { continue; }
            let declared = &source.declarations[relation].columns[column];
            if let Some(codec) = symbols.codec(declared.logical_type_id) {
                let ty = &declared.ty;
                let domain = components.columns[relation][column];
                required.push(quote!((#domain, <#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::code(&(#value)))));
            }
        }
    }
    Ok(quote!((&[#(#entries),*] as &[(&str, &[u32])], || ::std::vec![#(#required),*])))
}

/// One component for every declaration column, including native numeric fields.
/// IDs are deterministic in declaration/column order and contain no data bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainComponents {
    pub columns: Vec<Vec<u32>>,
    pub count: usize,
}

fn requirements(source: &Program) -> syn::Result<(Vec<usize>, Vec<(usize, usize)>)> {
    let positions: BTreeMap<_, _> = source.declarations.iter().enumerate()
        .map(|(i, declaration)| (symbol_name(&declaration.name), i)).collect();
    let mut offsets = Vec::with_capacity(source.declarations.len() + 1);
    offsets.push(0);
    for declaration in &source.declarations {
        offsets.push(offsets.last().unwrap() + declaration.arity());
    }
    let mut edges = Vec::new();
    for rule in &source.rules {
        let mut variables = BTreeMap::<String, usize>::new();
        let mut atom = |atom: &cq_syntax::Atom| -> syn::Result<()> {
            let relation = *positions.get(&symbol_name(&atom.relation))
                .ok_or_else(|| syn::Error::new_spanned(atom, "domain atom names an undeclared relation"))?;
            if atom.args.len() != source.declarations[relation].arity() {
                return Err(syn::Error::new_spanned(atom, "domain atom arity differs from declaration"));
            }
            for (column, argument) in atom.args.iter().enumerate() {
                if let Some(variable) = rust::variable(argument) {
                    let node = offsets[relation] + column;
                    if let Some(&first) = variables.get(&symbol_name(variable)) {
                        edges.push((first, node));
                    } else {
                        variables.insert(symbol_name(variable), node);
                    }
                }
            }
            Ok(())
        };
        atom(&rule.head)?;
        for item in &rule.body {
            match item {
                BodyItem::Positive { atom: input } => atom(input)?,
                BodyItem::Negation(negative) => atom(&negative.atom)?,
                _ => {}
            }
        }
        fn compare(condition: &syn::Expr, variables: &BTreeMap<String, usize>, edges: &mut Vec<(usize, usize)>) {
            if let syn::Expr::Binary(binary) = rust::unparen(condition) {
                if matches!(binary.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_)
                    | syn::BinOp::Lt(_) | syn::BinOp::Le(_) | syn::BinOp::Gt(_) | syn::BinOp::Ge(_))
                    && let (Some(left), Some(right)) = (rust::variable(&binary.left), rust::variable(&binary.right))
                    && let (Some(&left), Some(&right)) = (variables.get(&symbol_name(left)), variables.get(&symbol_name(right))) {
                    edges.push((left, right));
                }
                compare(&binary.left, variables, edges);
                compare(&binary.right, variables, edges);
            }
        }
        fn relational(condition: &cq_syntax::Constraint, variables: &BTreeMap<String, usize>, edges: &mut Vec<(usize, usize)>) {
            if let cq_syntax::Constraint::Or(branches) = condition {
                for condition in branches.iter().flatten() { relational(condition, variables, edges); }
            } else {
                let mut nodes = condition.variables().filter_map(|variable| variables.get(&symbol_name(variable)));
                if let Some(&first) = nodes.next() { edges.extend(nodes.map(|&next| (first, next))); }
            }
        }
        for item in &rule.body {
            match item {
                BodyItem::Filter(filter) => compare(&filter.condition, &variables, &mut edges),
                BodyItem::Constraint(condition) => relational(condition, &variables, &mut edges),
                _ => {}
            }
        }
    }
    Ok((offsets, edges))
}

impl DomainComponents {
    pub fn infer(source: &Program) -> syn::Result<Self> {
        let (offsets, edges) = requirements(source)?;
        let mut parent: Vec<usize> = (0..*offsets.last().unwrap()).collect();
        fn root(parent: &mut [usize], mut node: usize) -> usize {
            while parent[node] != node {
                parent[node] = parent[parent[node]];
                node = parent[node];
            }
            node
        }
        for (left, right) in edges {
            let left = root(&mut parent, left);
            let right = root(&mut parent, right);
            parent[left.max(right)] = left.min(right);
        }
        let mut ids = BTreeMap::new();
        let mut columns = Vec::with_capacity(source.declarations.len());
        for range in offsets.windows(2) {
            let mut relation = Vec::with_capacity(range[1] - range[0]);
            for node in range[0]..range[1] {
                let root = root(&mut parent, node);
                let next = u32::try_from(ids.len()).map_err(|_| syn::Error::new_spanned(source, "too many domain components"))?;
                relation.push(*ids.entry(root).or_insert(next));
            }
            columns.push(relation);
        }
        let result = Self { columns, count: ids.len() };
        result.check(source)?;
        Ok(result)
    }

    /// Validate connectivity independently by graph traversal, rather than
    /// trusting the union-find producer or merely permitting arbitrary mergers.
    pub fn check(&self, source: &Program) -> syn::Result<()> {
        let error = || syn::Error::new_spanned(source, "domain component table differs from rule equality components");
        let (offsets, edges) = requirements(source)?;
        if self.columns.len() != source.declarations.len()
            || self.columns.iter().zip(&source.declarations).any(|(columns, declaration)| columns.len() != declaration.arity()) {
            return Err(error());
        }
        let actual: Vec<_> = self.columns.iter().flatten().copied().collect();
        let mut adjacency = vec![Vec::new(); actual.len()];
        for (left, right) in edges {
            adjacency[left].push(right);
            adjacency[right].push(left);
        }
        let mut visited = vec![false; actual.len()];
        let mut component = 0u32;
        for node in 0..*offsets.last().unwrap() {
            if visited[node] { continue; }
            let mut stack = vec![node];
            visited[node] = true;
            while let Some(current) = stack.pop() {
                if actual[current] != component { return Err(error()); }
                for &next in &adjacency[current] {
                    if !visited[next] { visited[next] = true; stack.push(next); }
                }
            }
            component += 1;
        }
        if component as usize != self.count { return Err(error()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_variables_heads_and_negation_record_exact_components() {
        let source: Program = syn::parse_quote! {
            struct Domains;
            relation E(a:u32,b:u32) = (edges);
            relation N(a:u32) = (negative);
            relation Out(a:u32,b:u32);
            output Out;
            Out(x,y) :- E(x,y), !N(x);
        };
        let table = DomainComponents::infer(&source).unwrap();
        assert_eq!(table.columns, [vec![0,1], vec![0], vec![0,1]]);
        assert_eq!(table.count, 2);
        let mut bad = table.clone();
        bad.columns[1][0] = 1;
        assert!(bad.check(&source).is_err());
        let mut merged = table;
        for id in merged.columns.iter_mut().flatten() { *id = 0; }
        merged.count = 1;
        assert!(merged.check(&source).is_err());
    }

    #[test]
    fn comparisons_unify_columns_but_equal_constants_do_not() {
        let source: Program = syn::parse_quote! {
            struct Domains;
            relation E(a:u32,b:u32,c:u32) = (edges);
            relation Out(a:u32);
            output Out;
            Out(x) :- E(x,y,7), where x != y;
        };
        let table = DomainComponents::infer(&source).unwrap();
        assert_eq!(table.columns, [vec![0,0,1], vec![0]]);
    }
}
