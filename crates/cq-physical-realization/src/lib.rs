//! Structures → Rust: print one pull program.
//!
//! Nothing about the query is decided here: every physical operator, mode,
//! host fact and built type is fixed by Structures. This pass decides only
//! the Rust shape of each read and the layout of the values flowing between
//! operators:
//!
//! - Rows flow as tuples in sorted-heading order, each column bound to the
//!   RA heading's own identifier so user Rust inside nested macros resolves.
//!   Host columns are `&T` (borrowed from the host value, or from rows a
//!   structure was built over); computed columns are the owned value the
//!   extend produced. User Rust (select conditions, extend values, head
//!   expressions) sees every column as `&T`.
//! - A `let` is collected once (`__cq_<r>_rows`) only when its rows are
//!   scanned as a bag or more than one structure is built over them. Each
//!   built structure is `__cq_<r>_on_<cols>`, bound by reference and built
//!   through `cq_storage::Build` from `((key cols,), (remaining cols,))`
//!   pairs in sorted column order, followed by a `cq_storage::require` guard
//!   naming the trait every read on it uses. A `let` flagged `canon` gets a
//!   `HashSet` of its full rows unless a full-key structure exists; scans of
//!   it read that set.
//! - A host keyed `[k…]` is `Map<(k…,), Coll<(rest…,)>>` with tuples in the
//!   host's declared column order; an unkeyed host iterates full rows. Its
//!   key is constructed from the bound columns by `Clone::clone`, the one
//!   copy the type imposes; built structures are keyed by `(&T, …)` and
//!   nothing else is ever cloned.
//! - `merge` co-iterates native cursors when both sides are hosts or both
//!   are built structures; otherwise (a sorted pipe, or a mixed pair) each
//!   side is presented through `cq_storage::sorted_by` with its key
//!   re-tupled as references in sorted column order. A host merged on a
//!   multi-column key must list the key in that sorted order.
//! - The block's value is an iterator of the answer tuples in head order.
//!   A `canon` pipe or alias is deduplicated where it is scanned through the
//!   caller's distinct-state provider. Structures live only for the block,
//!   so a program that binds any statement collects its answers inside the
//!   block and evaluates to `Vec::into_iter`; a program over hosts alone
//!   evaluates to the lazy iterator itself.

use std::collections::BTreeMap;

use cq_ir::physical::{self, Operator};
use cq_ir::{cq, relational_plan as ra, structures, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Expr, Ident, Stmt};

mod access;
mod expressions;
mod operators;

use access::{Access, Side};
use expressions::{deref_predicates, window};

/// Caller-resolved macro paths the program is printed into.
pub struct Endpoints {
    /// Distinct-state provider ([`cq_ir::state_protocol`]): `new`, `insert`.
    pub distinct_state: syn::Path,
}

/// The pull program of a Structures module.
pub fn compile(module: &structures::Module, endpoints: &Endpoints) -> syn::Result<Expr> {
    let analysis = structures::contract::check(module)?;
    let ra_module = &module.physical.index_requirements.relational_plan;
    let infos = ra::check_in(ra_module.program.inputs(), &ra_module.plan)?;
    let mut printer = Printer::new(module, &analysis, infos, endpoints);
    for (definition, ra_definition) in module
        .physical
        .program
        .definitions
        .iter()
        .zip(&ra_module.plan.definitions)
    {
        printer.define(definition, ra_definition)?;
    }
    let returned = &module.physical.program.returned;
    let rows = printer.scan(returned)?;
    let output = printer.head(&symbol_name(returned), &ra_module.plan.output.head, rows);
    let statements = &printer.statements;
    let value = if !printer.bound {
        output
    } else {
        quote! {
            ::core::iter::IntoIterator::into_iter(
                ::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#output)
            )
        }
    };
    syn::parse2(quote! { { #(#statements)* #value } })
}

/// Whether `target` is the program this pass prints for `source`.
pub fn check(source: &structures::Module, target: &Expr, endpoints: &Endpoints) -> bool {
    compile(source, endpoints).is_ok_and(|expected| expected == *target)
}

/// A host relation's value: its name in scope, declared column order, and
/// key columns in the order the Structures entry lists them.
struct Host {
    value: Ident,
    layout: Vec<String>,
    key: Option<Vec<String>>,
}

/// Where a result's rows live once printed.
enum Repr {
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

struct Printer<'a> {
    analysis: &'a structures::Analysis,
    infos: BTreeMap<String, ra::RelationInfo>,
    physical: BTreeMap<String, &'a physical::Definition>,
    hosts: BTreeMap<String, Host>,
    /// The identifier every result binds each of its columns to.
    idents: BTreeMap<String, BTreeMap<String, Ident>>,
    reprs: BTreeMap<String, Repr>,
    endpoints: &'a Endpoints,
    statements: Vec<Stmt>,
    /// Whether a statement binds a value the iterator borrows.
    bound: bool,
}

fn error<T>(spanned: &impl ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

fn sorted(columns: &cq_ir::CommaList<Ident>) -> Vec<String> {
    let mut names: Vec<String> = columns.iter().map(symbol_name).collect();
    names.sort();
    names
}

fn index(i: usize) -> syn::Index {
    syn::Index::from(i)
}

impl<'a> Printer<'a> {
    fn new(
        module: &'a structures::Module,
        analysis: &'a structures::Analysis,
        infos: BTreeMap<String, ra::RelationInfo>,
        endpoints: &'a Endpoints,
    ) -> Self {
        let program = &module.physical.index_requirements.relational_plan.program;
        let mut hosts: BTreeMap<String, Host> = program
            .inputs()
            .map(|input| {
                let name = symbol_name(&input.name);
                let host = Host {
                    value: input.name.clone(),
                    layout: analysis.host_layouts[&name].clone(),
                    key: None,
                };
                (name, host)
            })
            .collect();
        let mut statements = Vec::new();
        for entry in &module.entries {
            if let structures::Structure::Host(host) = &entry.structure {
                let name = symbol_name(&entry.name);
                if let Some(key) = &host.key {
                    hosts.get_mut(&name).expect("checked host").key =
                        Some(key.columns.iter().map(symbol_name).collect());
                }
                if host.canon.is_some() {
                    let value = &hosts[&name].value;
                    statements.push(syn::parse_quote! {
                        ::cq_storage::require::canonical(&#value);
                    });
                }
            }
        }
        Printer {
            analysis,
            infos,
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
            endpoints,
            statements,
            bound: false,
        }
    }

    pub(crate) fn heading(&self, result: &str) -> &'a [String] {
        &self.analysis.physical.headings[result]
    }

    pub(crate) fn ident(&self, result: &str, column: &str) -> &Ident {
        &self.idents[result][column]
    }

    /// `(a, b, …,)` binding or rebuilding a row of `result`.
    pub(crate) fn pattern(&self, result: &str) -> TokenStream {
        let columns = self.heading(result).iter().map(|c| self.ident(result, c));
        quote!((#(#columns,)*))
    }

    /// A row of `result` with each column given by `expression`.
    pub(crate) fn row(
        &self,
        result: &str,
        expression: impl Fn(&str) -> TokenStream,
    ) -> TokenStream {
        let fields = self.heading(result).iter().map(|c| expression(c));
        quote!((#(#fields,)*))
    }

    /// `&(a, ref x, …,)`: binds a borrowed row so every column is `&T`.
    pub(crate) fn borrowed_pattern(&self, result: &str) -> TokenStream {
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

    /// `#[allow(unused_variables)] let x = &x;` for every computed column,
    /// so user Rust sees it as `&T` like every other column.
    pub(crate) fn shadows(&self, result: &str) -> Vec<TokenStream> {
        self.infos[result]
            .computed
            .iter()
            .map(|c| {
                let ident = self.ident(result, c);
                quote!(#[allow(unused_variables)] let #ident = &#ident;)
            })
            .collect()
    }

    /// The identifiers each result binds its columns to, from the RA
    /// operator that introduces or forwards them.
    pub(crate) fn register(&mut self, name: &str, operator: &ra::Operator) {
        let of = |input: &Ident| self.idents[&symbol_name(input)].clone();
        let idents = match operator {
            ra::Operator::Unit(_) => BTreeMap::new(),
            ra::Operator::Rename(r) => r
                .mapping
                .iter()
                .map(|m| (symbol_name(&m.target), m.target.clone()))
                .collect(),
            ra::Operator::NaturalJoin(j) => {
                let mut idents = of(&j.right);
                idents.extend(of(&j.left));
                idents
            }
            ra::Operator::Semijoin(j) => of(&j.left),
            ra::Operator::Select(s) => of(&s.input),
            ra::Operator::Extend(e) => {
                let mut idents = of(&e.input);
                idents.extend(
                    e.columns
                        .iter()
                        .map(|c| (symbol_name(&c.name), c.name.clone())),
                );
                idents
            }
            ra::Operator::Project(p) => {
                let heading = self.heading(name);
                of(&p.input)
                    .into_iter()
                    .filter(|(c, _)| heading.contains(c))
                    .collect()
            }
        };
        self.idents.insert(name.to_owned(), idents);
    }

    pub(crate) fn define(
        &mut self,
        definition: &physical::Definition,
        ra_definition: &ra::Definition,
    ) -> syn::Result<()> {
        let name = symbol_name(&definition.name);
        self.register(&name, &ra_definition.operator);
        let repr = match definition.mode {
            physical::Mode::Alias(_) => Repr::Alias,
            physical::Mode::Pipe(_) => {
                let mut rows = self.produce(&name, definition, ra_definition)?;
                if definition.canon.is_some() {
                    rows = self.distinct(&definition.name, rows);
                }
                Repr::Pipe(Some(rows))
            }
            physical::Mode::Let(_) => {
                let rows = self.produce(&name, definition, ra_definition)?;
                self.materialize(&name, definition.canon.is_some(), rows)
            }
        };
        self.reprs.insert(name, repr);
        Ok(())
    }

    /// Bind the rows and structures of a `let`, with the guard of every read.
    pub(crate) fn materialize(&mut self, name: &str, canon: bool, source: TokenStream) -> Repr {
        let heading = self.heading(name);
        let scanned = self.analysis.physical.scanned.contains(name);
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

    /// The `cq_storage::require` guards of the reads on `result` keyed on
    /// `columns`: seek for probes, contains for semijoins, ordered for
    /// merges and ranges, canonical for the set a `canon` scan reads.
    pub(crate) fn guards(&self, result: &str, columns: &[String], set: bool) -> Vec<Ident> {
        let physical = &self.analysis.physical;
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

    /// The rows of `result` as an iterator of heading-order tuples.
    pub(crate) fn scan(&mut self, result: &Ident) -> syn::Result<TokenStream> {
        let name = symbol_name(result);
        match self.reprs.get(&name) {
            Some(Repr::Alias) => {
                let rows = self.host_scan(&name);
                Ok(if self.analysis.physical.canon.contains(&name) {
                    self.distinct(&self.physical[&name].name, rows)
                } else {
                    rows
                })
            }
            Some(Repr::Pipe(_)) => match self.take_pipe(&name) {
                Some(rows) => Ok(rows),
                None => error(result, format!("pipe `{name}` is read twice")),
            },
            Some(Repr::Let { rows, structures }) => {
                let heading = self.heading(&name);
                let set = structures
                    .iter()
                    .find(|(c, _)| c.len() == 1 && c[0] == heading)
                    .map(|(_, s)| s);
                let source = match (self.analysis.physical.canon.contains(&name), set, rows) {
                    (true, Some(set), _) => set,
                    (_, _, Some(rows)) => rows,
                    _ => return error(result, format!("`{name}` is scanned but not materialized")),
                };
                Ok(quote!(::core::iter::Iterator::copied(#source.iter())))
            }
            None => error(result, format!("`{name}` is read before its definition")),
        }
    }

    pub(crate) fn take_pipe(&mut self, name: &str) -> Option<TokenStream> {
        match self.reprs.get_mut(name) {
            Some(Repr::Pipe(rows)) => rows.take(),
            _ => None,
        }
    }

    /// Deduplicate a stream through the caller's distinct-state provider.
    pub(crate) fn distinct(&self, operator: &Ident, rows: TokenStream) -> TokenStream {
        let provider = &self.endpoints.distinct_state;
        quote! {{
            let mut __cq_seen = #provider! { operator #operator; new };
            ::core::iter::Iterator::filter(#rows, move |__cq_row| {
                #provider! { operator #operator; insert(__cq_seen, __cq_row) }
            })
        }}
    }

    /// Map the returned rows to the head's argument order, evaluating
    /// computed arguments with every column as `&T`.
    pub(crate) fn head(&self, result: &str, head: &cq::Atom, rows: TokenStream) -> TokenStream {
        let heading = self.heading(result);
        let variables: Vec<Option<&Ident>> = head.args.iter().map(cq::rust::variable).collect();
        let identity = variables.len() == heading.len()
            && variables
                .iter()
                .zip(heading)
                .all(|(v, h)| v.is_some_and(|v| symbol_name(v) == *h));
        if identity {
            return rows;
        }
        let computed = variables.iter().any(Option::is_none);
        // A column the head names as a variable binds under the head's own
        // identifier; the others keep the heading's, or are dropped.
        let binders = heading.iter().map(|c| {
            match variables.iter().flatten().find(|v| symbol_name(v) == *c) {
                Some(v) => v.to_token_stream(),
                None if computed => self.ident(result, c).to_token_stream(),
                None => quote!(_),
            }
        });
        let pattern = quote!((#(#binders,)*));
        if !computed {
            let output = variables.iter().flatten();
            return quote! {
                ::core::iter::Iterator::map(#rows, move |#pattern| (#(#output,)*))
            };
        }
        let shadows = self.shadows(result);
        let temporaries = (0..head.args.len())
            .filter(|&i| variables[i].is_none())
            .map(|i| format_ident!("__cq_h{i}"));
        let expressions = head
            .args
            .iter()
            .zip(&variables)
            .filter(|(_, v)| v.is_none())
            .map(|(e, _)| e);
        let output = variables.iter().enumerate().map(|(i, v)| match v {
            Some(v) => v.to_token_stream(),
            None => format_ident!("__cq_h{i}").to_token_stream(),
        });
        quote! {
            ::core::iter::Iterator::map(#rows, move |#[allow(unused_variables)] #pattern| {
                let (#(#temporaries,)*) = { #(#shadows)* (#(#expressions,)*) };
                (#(#output,)*)
            })
        }
    }
}
