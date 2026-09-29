//! Reusable row layouts, host and built access, and expression printing helpers.
//!
//! [`Context`] records checked layouts and generated bindings. Printers choose
//! their control flow and state sinks; this crate selects neither a planner
//! nor a runtime dependency. Runtime names occur only in emitted Rust tokens.

use cq_lang_structures as structures;
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeMap;
use structures::physical::{self, Operator};
use structures::syntax::symbol_name;
use syn::{Expr, Ident, Stmt};

pub mod access;
pub mod expressions;

/// A host relation's value: its name in scope, declared column order, and
/// key columns in the order the Structures entry lists them.
pub struct Host {
    pub value: Ident,
    pub layout: Vec<String>,
    pub key: Option<Vec<String>>,
}

/// Where a result's rows live once printed.
pub enum Repr {
    /// A rename read in place through its host.
    Alias,
    /// A stream, inlined into its single reader.
    Pipe(Option<TokenStream>),
    /// Materialized: collected rows and the structures over them, each
    /// keyed on sorted column names.
    Let {
        rows: Option<Ident>,
        structures: Vec<(Vec<Vec<String>>, Ident)>,
    },
}

pub struct Context<'a> {
    pub analysis: &'a structures::Analysis,
    pub physical_analysis: &'a physical::Analysis,
    pub infos: BTreeMap<String, physical::Row>,
    pub physical: BTreeMap<String, &'a physical::Definition>,
    pub hosts: BTreeMap<String, Host>,
    /// The identifier every result binds each of its columns to.
    pub idents: BTreeMap<String, BTreeMap<String, Ident>>,
    pub reprs: BTreeMap<String, Repr>,
    pub statements: Vec<Stmt>,
    /// Whether a statement binds a value the iterator borrows.
    pub bound: bool,
}

pub fn error<T>(spanned: &impl ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

pub fn sorted(columns: &cq_lang_structures::syntax::CommaList<Ident>) -> Vec<String> {
    let mut names: Vec<String> = columns.iter().map(symbol_name).collect();
    names.sort();
    names
}

pub fn index(i: usize) -> syn::Index {
    syn::Index::from(i)
}

impl<'a> Context<'a> {
    /// Construct printer state from the complete program and its checked analysis.
    pub fn new(module: &'a structures::Program, analysis: &'a structures::Analysis) -> Self {
        let mut hosts = BTreeMap::new();
        let mut statements = Vec::new();
        let mut bound = false;
        // Reserve names even inside opaque user tokens before introducing leaf bindings.
        fn names(tokens: TokenStream, used: &mut std::collections::BTreeSet<String>) {
            for token in tokens {
                match token {
                    proc_macro2::TokenTree::Ident(i) => {
                        used.insert(symbol_name(&i));
                    }
                    proc_macro2::TokenTree::Group(g) => names(g.stream(), used),
                    _ => {}
                }
            }
        }
        let mut used = std::collections::BTreeSet::new();
        names(module.to_token_stream(), &mut used);
        for input in module.physical.inputs() {
            let facts = input.host_facts().expect("checked declaration");
            let value = if let Some(leaf) = &input.value {
                // Formal rule IF-Leaf: Bind each leaf once in its enclosing Rust context.
                let mut n = hosts.len();
                while used.contains(&format!("__cq_leaf_{n}")) {
                    n += 1;
                }
                let value = format_ident!("__cq_leaf_{n}", span = proc_macro2::Span::mixed_site());
                used.insert(value.to_string());
                let expression = &leaf.expression;
                statements.push(syn::parse_quote! { let #value = &(#expression); });
                bound = true;
                value
            } else {
                input.name.clone()
            };
            let name = symbol_name(&input.name);
            hosts.insert(
                name.clone(),
                Host {
                    value,
                    layout: analysis.host_layouts[&name].clone(),
                    key: facts
                        .key
                        .map(|k| k.columns.iter().map(symbol_name).collect()),
                },
            );
        }
        // Keep named-host guard order identical to the authored Structures order.
        // Facts themselves come only from the checked declarations.
        for entry in &module.structures.entries {
            if !matches!(entry.structure, structures::Structure::Host(_)) {
                continue;
            }
            let input = module
                .physical
                .inputs()
                .find(|d| symbol_name(&d.name) == symbol_name(&entry.name))
                .expect("checked host");
            let facts = input.host_facts().expect("checked declaration");
            let value = &hosts[&symbol_name(&input.name)].value;
            // Formal rule IF-Facts: rustc checks every declared fact, even on unused access paths.
            // Formal rule IF-Canon: Require the canonical marker.
            if facts.canon.is_some() {
                statements.push(syn::parse_quote! { ::cq_storage::require::canonical(&#value); });
            }
            // Formal rule IF-Keyed: Require keyed read at the declared key tuple type.
            if let Some(key) = &facts.key {
                let types = key.columns.iter().map(|c| {
                    &input
                        .columns
                        .iter()
                        .find(|d| symbol_name(&d.name) == symbol_name(c))
                        .expect("checked column")
                        .ty
                });
                statements.push(syn::parse_quote! { ::cq_storage::require::keyed::<(#(#types,)*), _>(&#value); });
                // Formal rule IF-Ordered: Require ordered read even when only scanned.
                if key.ordered.is_some() {
                    statements.push(syn::parse_quote! { ::cq_storage::require::ordered(&#value); });
                }
            }
        }
        Context {
            physical_analysis: &analysis.physical,
            analysis,
            infos: analysis.physical.rows.clone(),
            physical: module
                .physical
                .program
                .definitions
                .iter()
                .map(|d| (symbol_name(&d.name), d))
                .collect(),
            hosts,
            idents: BTreeMap::new(),
            reprs: BTreeMap::new(),
            statements,
            bound,
        }
    }

    /// The sorted logical heading of a result.
    pub fn heading(&self, result: &str) -> &'a [String] {
        &self.physical_analysis.headings[result]
    }

    /// The Rust binding assigned to one logical column.
    pub fn ident(&self, result: &str, column: &str) -> &Ident {
        &self.idents[result][column]
    }

    /// Tuple pattern binding all columns in sorted heading order.
    pub fn pattern(&self, result: &str) -> TokenStream {
        let columns = self.heading(result).iter().map(|c| self.ident(result, c));
        quote!((#(#columns,)*))
    }

    /// Construct a heading-ordered tuple using a caller-provided field expression.
    pub fn row(&self, result: &str, expression: impl Fn(&str) -> TokenStream) -> TokenStream {
        let fields = self.heading(result).iter().map(|c| expression(c));
        quote!((#(#fields,)*))
    }

    /// Bind a borrowed row, borrowing owned computed columns as needed.
    pub fn borrowed_pattern(&self, result: &str) -> TokenStream {
        let computed = &self.infos[result].computed;
        let binders = self.heading(result).iter().map(|c| {
            let ident = self.ident(result, c);
            if computed.contains(c) {
                quote!(ref #ident)
            } else {
                ident.to_token_stream()
            }
        });
        quote!(&(#(#binders,)*))
    }

    /// Borrow computed bindings before evaluating embedded Rust expressions.
    pub fn shadows(&self, result: &str) -> Vec<TokenStream> {
        self.infos[result]
            .computed
            .iter()
            .map(|c| {
                let ident = self.ident(result, c);
                quote!(#[allow(unused_variables)] let #ident = &#ident;)
            })
            .collect()
    }

    /// Propagate Rust identifiers through the logical operation defining a result.
    pub fn register(&mut self, name: &str) {
        self.idents
            .insert(name.to_owned(), self.infos[name].bindings.clone());
    }

    /// Build the selected structures over a result and append their statements.
    pub fn materialize(&mut self, name: &str, canon: bool, source: TokenStream) -> Repr {
        let heading = self.heading(name);
        let scanned = self.physical_analysis.scanned.contains(name);
        let mut structures: Vec<(Vec<Vec<String>>, syn::Type)> =
            self.analysis.built.get(name).cloned().unwrap_or_default();
        if canon
            && !structures
                .iter()
                .any(|(c, _)| c.len() == 1 && c[0] == heading)
        {
            structures.push((
                vec![heading.to_vec()],
                syn::parse_quote!(::std::collections::HashSet<_>),
            ));
        }
        // Rows are kept only when scanned as a bag (a `canon` scan reads the
        // set instead) or when more than one structure is built from them.
        let rows = ((scanned && !canon) || structures.len() > 1)
            .then(|| format_ident!("__cq_{}_rows", name));
        let mut statements: Vec<Stmt> = Vec::new();
        let source = match &rows {
            Some(rows) => {
                statements.push(syn::parse_quote! {
                    let #rows: &::std::vec::Vec<_> = &::core::iter::Iterator::collect(#source);
                });
                quote!(::core::iter::Iterator::copied(#rows.iter()))
            }
            None => source,
        };
        let pattern = self.pattern(name);
        let mut built = Vec::new();
        for (levels, ty) in &structures {
            let columns: Vec<String> = levels.concat();
            let payload = heading
                .iter()
                .filter(|c| !columns.contains(c))
                .map(|c| self.ident(name, c));
            // A curried value is built from nested pairs, one per level:
            // `((k₁,), ((k₂,), (payload,)))`.
            let mut nested = quote!((#(#payload,)*));
            for level in levels.iter().rev() {
                let key = level.iter().map(|c| self.ident(name, c));
                nested = quote!(((#(#key,)*), #nested));
            }
            let structure = format_ident!(
                "__cq_{}_on_{}",
                name,
                levels
                    .iter()
                    .map(|l| l.join("_"))
                    .collect::<Vec<_>>()
                    .join("__")
            );
            let pairs = quote! {
                ::core::iter::Iterator::map(#source, |#pattern| #nested)
            };
            // A bare path lets Rust infer the type's generics from the pairs;
            // written arguments (or holes) select the qualified form.
            let bare = matches!(ty, syn::Type::Path(path)
                if path.qself.is_none() && path.path.segments.iter().all(|s| s.arguments.is_none()));
            let build = if bare {
                quote!({ use ::cq_storage::Build as _; #ty::build(#pairs) })
            } else {
                quote!(<#ty as ::cq_storage::Build<_>>::build(#pairs))
            };
            statements.push(syn::parse_quote! { let #structure = &#build; });
            // A type lacking a trait a read uses on it fails at a one-line
            // call naming the trait, rather than inside a closure.
            for guard in self.guards(name, &columns, canon && columns == heading) {
                statements.push(syn::parse_quote! { ::cq_storage::require::#guard(#structure); });
            }
            built.push((levels.clone(), structure));
        }
        self.bound |= !statements.is_empty();
        self.statements.extend(statements);
        Repr::Let {
            rows,
            structures: built,
        }
    }

    /// Runtime trait checks required for reads through a particular key.
    pub fn guards(&self, result: &str, columns: &[String], set: bool) -> Vec<Ident> {
        let physical = &self.physical_analysis;
        let mut guards: Vec<Ident> = Vec::new();
        let mut require = |guard: &str| {
            if !guards.iter().any(|g| g == guard) {
                guards.push(format_ident!("{guard}"));
            }
        };
        for (reader, read) in physical.key_reads.get(result).into_iter().flatten() {
            if read == columns {
                match &self.physical[reader].operator {
                    Operator::Probe(..) => require("seek"),
                    Operator::Semi(..) => require("contains"),
                    _ => unreachable!("key reads come from probes and semijoins"),
                }
            }
        }
        for (_, read) in physical.merge_reads.get(result).into_iter().flatten() {
            if read == columns {
                require("ordered");
            }
        }
        for (_, column) in physical.range_reads.get(result).into_iter().flatten() {
            if columns == std::slice::from_ref(column) {
                require("ordered");
            }
        }
        if set && physical.scanned.contains(result) {
            require("canonical");
        }
        guards
    }

    /// Consume an inline stream exactly once.
    pub fn take_pipe(&mut self, name: &str) -> Option<TokenStream> {
        match self.reprs.get_mut(name) {
            Some(Repr::Pipe(rows)) => rows.take(),
            _ => None,
        }
    }
    /// The levels a built structure of `result` was declared with.
    pub fn levels_of(&self, result: &str, structure: &Ident) -> Vec<Vec<String>> {
        let Some(Repr::Let { structures, .. }) = self.reprs.get(result) else {
            unreachable!("a built structure belongs to a let");
        };
        structures
            .iter()
            .find(|(_, s)| s == structure)
            .map(|(l, _)| l.clone())
            .expect("the structure is declared")
    }

    /// Apply every level of a curried structure in turn to the bound columns
    /// of `left`; the items of the last application are the payloads.
    pub fn chained_lookup(
        &self,
        left: &str,
        structure: &Ident,
        right: &str,
        columns: &[String],
    ) -> TokenStream {
        let levels = self.levels_of(right, structure);
        debug_assert_eq!(
            structures::prefix_depth(&levels, columns),
            Some(levels.len())
        );
        let key = |level: &Vec<String>| {
            let key = level.iter().map(|c| self.ident(left, c));
            quote!(&(#(#key,)*))
        };
        let first = key(&levels[0]);
        let mut chain = quote!(::cq_storage::IndexRead::lookup(&#structure, #first));
        for level in &levels[1..] {
            let key = key(level);
            chain = quote! {
                ::core::iter::Iterator::flat_map(#chain, move |__cq_level| {
                    ::cq_storage::IndexRead::lookup(&__cq_level, #key)
                })
            };
        }
        chain
    }

    /// Membership on a prefix of a curried structure: every level but the
    /// last is applied, the last is tested.
    pub fn chained_contains(
        &self,
        left: &str,
        structure: &Ident,
        levels: &[Vec<String>],
    ) -> TokenStream {
        let key = |level: &Vec<String>| {
            let key = level.iter().map(|c| self.ident(left, c));
            quote!(&(#(#key,)*))
        };
        let (last, init) = levels.split_last().expect("at least one level");
        let last = key(last);
        if init.is_empty() {
            return quote!(::cq_storage::ExactMembership::contains(&#structure, #last));
        }
        let first = key(&init[0]);
        let mut chain = quote!(::cq_storage::IndexRead::lookup(&#structure, #first));
        for level in &init[1..] {
            let key = key(level);
            chain = quote! {
                ::core::iter::Iterator::flat_map(#chain, move |__cq_level| {
                    ::cq_storage::IndexRead::lookup(&__cq_level, #key)
                })
            };
        }
        quote! {
            ::core::iter::Iterator::any(&mut #chain, |__cq_level| {
                ::cq_storage::ExactMembership::contains(__cq_level, #last)
            })
        }
    }
}
