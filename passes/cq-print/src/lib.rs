//! Reusable row layouts, host and built access, and expression printing helpers.
//!
//! Queries denote `bottom + Set`: one panic aborts evaluation. Opaque Rust
//! binds and guards keep their written chain; relational steps may move only
//! earlier across them. Their free variables determine their partial-function
//! meaning; I/O, mutation and evaluation counts are outside that meaning.
//! Relational `==` decides Leibniz identity. `Eq` and the selected storage's
//! `Hash` or `Ord` bounds are necessary checks, not a proof of admissibility.
//!
//! [`Context`] records checked layouts and generated bindings. Printers choose
//! their control flow and state sinks; this crate selects neither a planner
//! nor a runtime dependency. Runtime names occur only in emitted Rust tokens.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! Public printer helpers preserve recorded union/anti representations, scoped Rust bindings and dedup state.
//! Shared source preparation exposes a fused stream for terminal pull and push consumers.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
use cq_lang_structures as structures;
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeMap;
use structures::physical::{self, Operator};
use structures::syntax::symbol_name;
use syn::{Expr, Ident, Stmt};

pub mod access;
pub mod expressions;
mod filtered;
mod ownership;
mod point;
mod range;
pub use ownership::own_captures;

/// A host relation's value: its name in scope, declared column order, and
/// key columns in the order the Structures entry lists them.
pub struct Host {
    pub value: Ident,
    pub layout: Vec<String>,
}

/// Where a result's rows live once printed.
pub enum Repr {
    /// A rename read in place through its host.
    Host,
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
    pub parameters: BTreeMap<String, Ident>,
    /// The identifier every result binds each of its columns to.
    pub idents: BTreeMap<String, BTreeMap<String, Ident>>,
    pub reprs: BTreeMap<String, Repr>,
    pub statements: Vec<Stmt>,
    /// Whether a statement binds a value the iterator borrows.
    pub bound: bool,
    /// Shared handles cloned into the move closures that use them.
    pub owned: Vec<Ident>,
    pub intersect: bool,
    pub candidate_reads: BTreeMap<String, Vec<Vec<String>>>,
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
                let mut n = hosts.len();
                while used.contains(&format!("__cq_input_{n}")) {
                    n += 1;
                }
                let value = format_ident!("__cq_input_{n}", span = proc_macro2::Span::mixed_site());
                used.insert(value.to_string());
                let input_name = &input.name;
                statements.push(syn::parse_quote! { let #value = &#input_name; });
                value
            };
            let name = symbol_name(&input.name);
            hosts.insert(
                name.clone(),
                Host {
                    value,
                    layout: analysis.host_layouts[&name].clone(),
                },
            );
        }
        // Formal rule IF-Parameter: Bind each captured relational value once outside all query loops.
        let mut parameters = BTreeMap::new();
        for definition in &module.physical.program.definitions {
            let theta = match &definition.operator {
                Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => &s.condition,
                _ => continue,
            };
            for parameter in theta
                .constraints
                .iter()
                .flat_map(cq_lang_structures::syntax::Constraint::parameters)
            {
                let name = symbol_name(parameter);
                if parameters.contains_key(&name) {
                    continue;
                }
                let mut n = parameters.len();
                while used.contains(&format!("__cq_parameter_{n}")) {
                    n += 1;
                }
                let id =
                    format_ident!("__cq_parameter_{n}", span = proc_macro2::Span::mixed_site());
                used.insert(id.to_string());
                statements.push(syn::parse_quote!(let #id = &#parameter;));
                parameters.insert(name, id);
            }
        }
        let mut candidate_reads: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
        for pair in module.physical.program.definitions.windows(2) {
            if !matches!(&pair[0].operator, Operator::Probe(_, k) if matches!(k.choice, Some(physical::Choice::Smallest(_))))
            {
                continue;
            }
            if let Some((a, b, v)) =
                physical::binary_intersection(&pair[0], &pair[1], &analysis.physical)
            {
                candidate_reads
                    .entry(symbol_name(&a.right))
                    .or_default()
                    .push(sorted(&a.columns));
                candidate_reads
                    .entry(symbol_name(&b.right))
                    .or_default()
                    .push(sorted(&b.columns).into_iter().filter(|c| *c != v).collect());
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
            parameters,
            idents: BTreeMap::new(),
            reprs: BTreeMap::new(),
            statements,
            bound,
            owned: Vec::new(),
            candidate_reads,
            intersect: module.physical.program.definitions.iter().any(|d| {
                matches!(&d.operator,
                Operator::Probe(_, k) if matches!(k.choice, Some(physical::Choice::Smallest(_))))
            }),
        }
    }

    pub fn relational_condition(
        &self,
        theta: &cq_lang_structures::syntax::algebra::Theta,
    ) -> TokenStream {
        expressions::relational_condition_with(theta, &self.parameters)
    }

    /// The sorted emitted row layout established by Structures.
    pub fn heading(&self, result: &str) -> &'a [String] {
        &self.analysis.layouts[result]
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

    /// Bind only names in the step's written lexical environment. Other fields
    /// pass through the row without shadowing Rust captures.
    pub fn scoped_borrowed_pattern(
        &self,
        result: &str,
        environment: Option<&structures::syntax::algebra::Environment>,
    ) -> TokenStream {
        let Some(environment) = environment else {
            return self.borrowed_pattern(result);
        };
        let names: std::collections::BTreeSet<_> =
            environment.variables.iter().map(symbol_name).collect();
        let binders = self.heading(result).iter().map(|column| {
            if !names.contains(column) {
                return quote!(_);
            }
            let id = &self.infos[result].bindings[column];
            if self.infos[result].computed.contains(column) {
                quote!(ref #id)
            } else {
                id.to_token_stream()
            }
        });
        quote!(&(#(#binders,)*))
    }

    /// Lexical aliases for a Rust expression over hidden physical row names.
    pub fn environment_aliases(
        &self,
        result: &str,
        environment: Option<&structures::syntax::algebra::Environment>,
    ) -> Vec<TokenStream> {
        let names: std::collections::BTreeSet<_> = environment
            .map(|e| e.variables.iter().map(symbol_name).collect())
            .unwrap_or_else(|| self.heading(result).iter().cloned().collect());
        self.heading(result)
            .iter()
            .filter(|name| names.contains(*name))
            .map(|name| {
                let logical = &self.infos[result].bindings[name];
                let physical = self.ident(result, name);
                if self.infos[result].computed.contains(name) {
                    quote!(#[allow(unused_variables)] let #logical = &#physical;)
                } else {
                    quote!(#[allow(unused_variables)] let #logical = #physical;)
                }
            })
            .collect()
    }

    /// Borrow computed bindings before evaluating embedded Rust expressions.
    pub fn shadows(&self, result: &str) -> Vec<TokenStream> {
        self.infos[result]
            .computed
            .iter()
            .filter(|c| self.heading(result).contains(c))
            .map(|c| {
                let ident = self.ident(result, c);
                quote!(#[allow(unused_variables)] let #ident = &#ident;)
            })
            .collect()
    }

    /// An index key is owned if either its rows or a keyed reader supplies a
    /// computed column. Temporary prefix bindings cannot borrow for the
    /// lifetime of the indexed input rows.
    pub fn owned_key(&self, result: &str, columns: &[String]) -> bool {
        let computed = |name: &str| {
            self.infos[name]
                .computed
                .iter()
                .any(|c| columns.contains(c))
        };
        computed(result)
            || self
                .physical_analysis
                .key_reads
                .get(result)
                .into_iter()
                .flatten()
                .any(|(reader, _)| match &self.physical[reader].operator {
                    Operator::Probe(_, keyed)
                    | Operator::Semi(_, keyed)
                    | Operator::Anti(_, keyed) => computed(&symbol_name(&keyed.left)),
                    _ => false,
                })
    }

    pub fn key_field(&self, result: &str, column: &str, owned: bool) -> TokenStream {
        let ident = self.ident(result, column);
        if !owned {
            return ident.to_token_stream();
        }
        if self.infos[result].computed.contains(column) {
            quote!(::core::clone::Clone::clone(&#ident))
        } else {
            quote!(::core::clone::Clone::clone(#ident))
        }
    }

    /// Keep original borrowed key columns alongside owned comparison keys,
    /// so result rows retain their original ownership and input identity.
    pub fn built_payload(&self, result: &str, levels: &[Vec<String>]) -> Vec<String> {
        let columns: Vec<_> = levels
            .iter()
            .flat_map(|level| self.stored_key(result, level))
            .collect();
        let payload = self.analysis.filtered.get(result).map(|f| {
            f.payload
                .columns
                .iter()
                .map(symbol_name)
                .collect::<Vec<_>>()
        });
        payload
            .as_deref()
            .unwrap_or_else(|| self.heading(result))
            .iter()
            .filter(|c| {
                !columns.contains(c)
                    || (!self.infos[result].computed.contains(*c)
                        && levels
                            .iter()
                            .any(|level| level.contains(c) && self.owned_key(result, level)))
            })
            .cloned()
            .collect()
    }

    /// Propagate Rust identifiers through the logical operation defining a result.
    pub fn register(&mut self, name: &str) {
        self.idents
            .insert(name.to_owned(), self.infos[name].bindings.clone());
        self.prepare_host(name);
    }

    /// Build the selected structures over a result and append their statements.
    pub fn materialize(&mut self, name: &str, _canon: bool, source: TokenStream) -> Repr {
        let structures: Vec<(Vec<Vec<String>>, syn::Type)> =
            self.analysis.built.get(name).cloned().unwrap_or_default();
        let rows = self
            .analysis
            .retained
            .contains_key(name)
            .then(|| format_ident!("__cq_{}_rows", name));
        let mut statements: Vec<Stmt> = Vec::new();
        let replay = self.analysis.filtered.contains_key(name)
            && matches!(&self.physical[name].operator,Operator::Union(union) if union.origin.as_ref().is_none_or(|origin|origin.over.is_none()));
        let direct_source = source.clone();
        let source = match &rows {
            Some(rows) => {
                let ty = &self.analysis.retained[name];
                statements.push(syn::parse_quote! {
                    let #rows: &#ty = &::core::iter::Iterator::collect(#source);
                });
                if self.infos[name].computed.is_empty() {
                    quote!(::core::iter::Iterator::copied(#rows.iter()))
                } else {
                    quote!(::core::iter::Iterator::cloned(#rows.iter()))
                }
            }
            None => source,
        };
        let source = if replay { direct_source } else { source };
        let mut built = Vec::new();
        for (levels, ty) in &structures {
            let columns: Vec<String> = levels.concat();
            let payload_columns = self.built_payload(name, levels);
            let stored_columns: Vec<_> = levels
                .iter()
                .flat_map(|level| self.stored_key(name, level))
                .collect();
            let binders = self.heading(name).iter().map(|column| {
                if stored_columns.contains(column) || payload_columns.contains(column) {
                    self.ident(name, column).to_token_stream()
                } else {
                    quote!(_)
                }
            });
            let pattern = quote!((#(#binders,)*));
            let payload = payload_columns.iter().map(|c| self.ident(name, c));
            // A curried value is built from nested pairs, one per level:
            // `((k₁,), ((k₂,), (payload,)))`.
            let mut nested = quote!((#(#payload,)*));
            for level in levels.iter().rev() {
                let owned = self.owned_key(name, level);
                let key = self
                    .stored_key(name, level)
                    .iter()
                    .map(|c| self.key_field(name, c, owned))
                    .collect::<Vec<_>>();
                nested = quote!(::cq_storage::require::key(((#(#key,)*), #nested)));
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
            // Guard the selected read structure: a candidate trie can share
            // its full key with a separate ordered representation.
            if self
                .analysis
                .built_on(name, &columns)
                .is_some_and(|(selected, _)| selected == ty)
            {
                for guard in self.guards(name, &columns, false) {
                    statements
                        .push(syn::parse_quote! { ::cq_storage::require::#guard(#structure); });
                }
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
                    Operator::Semi(..) | Operator::Anti(..) => require("contains"),
                    _ => unreachable!("key reads come from probes and semijoins"),
                }
            }
        }
        for (_, read) in physical.merge_reads.get(result).into_iter().flatten() {
            if read == columns {
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
            let owned = self.owned_key(right, level);
            let key = self
                .stored_key(right, level)
                .iter()
                .map(|c| self.key_field(left, c, owned))
                .collect::<Vec<_>>();
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
        if let Some(condition) = self.constant_key_condition(left, right, columns) {
            quote!(::core::iter::Iterator::flatten(::core::iter::IntoIterator::into_iter((#condition).then(|| #chain))))
        } else {
            chain
        }
    }

    /// Membership on a prefix of a curried structure: every level but the
    /// last is applied, the last is tested.
    pub fn chained_contains(
        &self,
        left: &str,
        structure: &Ident,
        right: &str,
        levels: &[Vec<String>],
    ) -> TokenStream {
        let key = |level: &Vec<String>| {
            let owned = self.owned_key(right, level);
            let key = self
                .stored_key(right, level)
                .iter()
                .map(|c| self.key_field(left, c, owned))
                .collect::<Vec<_>>();
            quote!(&(#(#key,)*))
        };
        let (last, init) = levels.split_last().expect("at least one level");
        let last = key(last);
        if init.is_empty() {
            if let Some(condition) = self.constant_key_condition(left, right, &levels.concat()) {
                return quote!((#condition) && ::cq_storage::ExactMembership::contains(&#structure, #last));
            }
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
