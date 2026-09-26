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
        structures: Vec<(Vec<String>, Ident)>,
    },
}

/// A host read through a rename, in the rename's attribute names.
struct HostAccess {
    value: Ident,
    keyed: bool,
    /// Key columns in the host's listed order.
    key: Vec<String>,
    /// Remaining columns in the host's declared order.
    payload: Vec<String>,
}

/// The value answering a keyed read.
enum Access {
    Host(HostAccess),
    Built(Ident),
}

/// One side of a merge.
enum Side {
    Host(HostAccess),
    Built(Ident),
    Pipe(TokenStream),
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

    fn heading(&self, result: &str) -> &'a [String] {
        &self.analysis.physical.headings[result]
    }

    fn ident(&self, result: &str, column: &str) -> &Ident {
        &self.idents[result][column]
    }

    /// `(a, b, …,)` binding or rebuilding a row of `result`.
    fn pattern(&self, result: &str) -> TokenStream {
        let columns = self.heading(result).iter().map(|c| self.ident(result, c));
        quote!((#(#columns,)*))
    }

    /// A row of `result` with each column given by `expression`.
    fn row(&self, result: &str, expression: impl Fn(&str) -> TokenStream) -> TokenStream {
        let fields = self.heading(result).iter().map(|c| expression(c));
        quote!((#(#fields,)*))
    }

    /// `&(a, ref x, …,)`: binds a borrowed row so every column is `&T`.
    fn borrowed_pattern(&self, result: &str) -> TokenStream {
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
    fn shadows(&self, result: &str) -> Vec<TokenStream> {
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
    fn register(&mut self, name: &str, operator: &ra::Operator) {
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

    fn define(
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
    fn materialize(&mut self, name: &str, canon: bool, source: TokenStream) -> Repr {
        let heading = self.heading(name);
        let scanned = self.analysis.physical.scanned.contains(name);
        let mut structures: Vec<(Vec<String>, syn::Type)> =
            self.analysis.built.get(name).cloned().unwrap_or_default();
        if canon && !structures.iter().any(|(c, _)| c == heading) {
            structures.push((
                heading.to_vec(),
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
        for (columns, ty) in &structures {
            let key = columns.iter().map(|c| self.ident(name, c));
            let payload = heading
                .iter()
                .filter(|c| !columns.contains(c))
                .map(|c| self.ident(name, c));
            let structure = format_ident!("__cq_{}_on_{}", name, columns.join("_"));
            let pairs = quote! {
                ::core::iter::Iterator::map(#source, |#pattern| ((#(#key,)*), (#(#payload,)*)))
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
            for guard in self.guards(name, columns, canon && columns == heading) {
                statements.push(syn::parse_quote! { ::cq_storage::require::#guard(#structure); });
            }
            built.push((columns.clone(), structure));
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
    fn guards(&self, result: &str, columns: &[String], set: bool) -> Vec<Ident> {
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
    fn scan(&mut self, result: &Ident) -> syn::Result<TokenStream> {
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
                    .find(|(c, _)| c == heading)
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

    fn take_pipe(&mut self, name: &str) -> Option<TokenStream> {
        match self.reprs.get_mut(name) {
            Some(Repr::Pipe(rows)) => rows.take(),
            _ => None,
        }
    }

    /// Deduplicate a stream through the caller's distinct-state provider.
    fn distinct(&self, operator: &Ident, rows: TokenStream) -> TokenStream {
        let provider = &self.endpoints.distinct_state;
        quote! {{
            let mut __cq_seen = #provider! { operator #operator; new };
            ::core::iter::Iterator::filter(#rows, move |__cq_row| {
                #provider! { operator #operator; insert(__cq_seen, __cq_row) }
            })
        }}
    }

    /// The host behind a rename, with its key and payload in the rename's
    /// attribute names.
    fn host_access(&self, result: &str) -> HostAccess {
        let host = &self.hosts[&self.analysis.physical.hosts[result]];
        let inverse: BTreeMap<&String, &String> = self.analysis.physical.renames[result]
            .iter()
            .map(|(target, source)| (source, target))
            .collect();
        let key: Vec<String> = host
            .key
            .iter()
            .flatten()
            .map(|c| inverse[c].clone())
            .collect();
        let payload = host
            .layout
            .iter()
            .filter(|c| !host.key.as_ref().is_some_and(|k| k.contains(c)))
            .map(|c| inverse[c].clone())
            .collect();
        HostAccess {
            value: host.value.clone(),
            keyed: host.key.is_some(),
            key,
            payload,
        }
    }

    /// Enumerate a host in declared order, rebound to the rename's names.
    fn host_scan(&self, result: &str) -> TokenStream {
        let access = self.host_access(result);
        let value = &access.value;
        let row = self.pattern(result);
        let payload = access.payload.iter().map(|c| self.ident(result, c));
        if !access.keyed {
            return quote! {
                ::core::iter::Iterator::map(#value.iter(), move |(#(#payload,)*)| #row)
            };
        }
        let key = access.key.iter().map(|c| self.ident(result, c));
        quote! {
            ::core::iter::Iterator::flat_map(#value.iter(), move |((#(#key,)*), __cq_values)| {
                ::core::iter::Iterator::map(__cq_values.iter(), move |(#(#payload,)*)| #row)
            })
        }
    }

    /// The value answering a read of `result` keyed on sorted `columns`: a
    /// structure built on them, else the host key the rename translates to.
    fn keyed(&self, result: &Ident, columns: &[String]) -> syn::Result<Access> {
        let name = symbol_name(result);
        if let Some(structure) = self.built(&name, columns) {
            return Ok(Access::Built(structure));
        }
        if self.analysis.host_key_for(&name, columns).is_some() {
            return Ok(Access::Host(self.host_access(&name)));
        }
        error(
            result,
            format!("no structure or host key answers the read of `{name}` on {columns:?}"),
        )
    }

    fn built(&self, name: &str, columns: &[String]) -> Option<Ident> {
        let Some(Repr::Let { structures, .. }) = self.reprs.get(name) else {
            return None;
        };
        structures
            .iter()
            .find(|(c, _)| c == columns)
            .map(|(_, s)| s.clone())
    }

    /// `&(k…,)`: the key a host map is entered with, constructed from the
    /// bound columns by the one copy the map's owned key type imposes.
    fn host_key(&self, result: &str, access: &HostAccess) -> TokenStream {
        let key = access.key.iter().map(|c| {
            let ident = self.ident(result, c);
            quote!(::core::clone::Clone::clone(#ident))
        });
        quote!(&(#(#key,)*))
    }

    /// The cursor merging or ranging over `result` on sorted `columns`; a
    /// pipe sorted on them by its own merge is consumed here.
    fn ordered(&mut self, reader: &Ident, result: &Ident, columns: &[String]) -> syn::Result<Side> {
        let name = symbol_name(result);
        if let Some(structure) = self.built(&name, columns) {
            return Ok(Side::Built(structure));
        }
        if self
            .analysis
            .host_key_for(&name, columns)
            .is_some_and(|k| k.ordered)
        {
            let access = self.host_access(&name);
            if access.key != columns {
                return error(
                    reader,
                    format!(
                        "`{reader}` reads `{name}` in the order {columns:?}, but host `{}` is ordered by {:?}",
                        access.value, access.key
                    ),
                );
            }
            return Ok(Side::Host(access));
        }
        if let Some(Operator::Merge(_, k)) = self.physical.get(&name).map(|d| &d.operator) {
            if sorted(&k.columns) == columns {
                if let Some(rows) = self.take_pipe(&name) {
                    return Ok(Side::Pipe(rows));
                }
            }
        }
        error(
            reader,
            format!(
                "no ordered structure, host key or sorted stream answers the read of `{name}` on {columns:?}"
            ),
        )
    }

    /// The rows of one physical operator, in the result's heading order.
    fn produce(
        &mut self,
        name: &str,
        definition: &physical::Definition,
        ra_definition: &ra::Definition,
    ) -> syn::Result<TokenStream> {
        match (&definition.operator, &ra_definition.operator) {
            (Operator::Unit(_), _) => Ok(quote!(::core::iter::once(()))),
            (Operator::Rename(_), _) => Ok(self.host_scan(name)),
            (Operator::Select(_), ra::Operator::Select(select)) => {
                let input = self.scan(&select.input)?;
                let pattern = self.borrowed_pattern(&symbol_name(&select.input));
                let condition = deref_predicates(&select.condition);
                Ok(quote! {
                    ::core::iter::Iterator::filter(
                        #input,
                        move |#[allow(unused_variables)] #pattern| #condition
                    )
                })
            }
            (Operator::Range(_, ranged), ra::Operator::Select(select)) => {
                self.range(name, &definition.name, &ranged.column, select)
            }
            (Operator::Extend(_), ra::Operator::Extend(extend)) => {
                let input = self.scan(&extend.input)?;
                let input_name = symbol_name(&extend.input);
                let pattern = self.pattern(&input_name);
                let shadows = self.shadows(&input_name);
                let names = extend.columns.iter().map(|c| &c.name);
                let types = extend.columns.iter().map(|c| &c.ty);
                let value = &extend.value;
                let row = self.pattern(name);
                Ok(quote! {
                    ::core::iter::Iterator::map(#input, move |#pattern| {
                        let (#(#names,)*): (#(#types,)*) = { #(#shadows)* #value };
                        #row
                    })
                })
            }
            (Operator::Project(_), ra::Operator::Project(project)) => {
                let input = self.scan(&project.input)?;
                let input_name = symbol_name(&project.input);
                let kept = self.heading(name);
                let binders = self.heading(&input_name).iter().map(|c| {
                    if kept.contains(c) {
                        self.ident(&input_name, c).to_token_stream()
                    } else {
                        quote!(_)
                    }
                });
                let row = self.pattern(name);
                Ok(quote! {
                    ::core::iter::Iterator::map(#input, move |(#(#binders,)*)| #row)
                })
            }
            (Operator::Probe(_, keyed), _) => self.probe(name, keyed),
            (Operator::Semi(_, keyed), _) => self.semi(keyed),
            (Operator::Merge(_, keyed), _) => self.merge(name, &definition.name, keyed),
            (operator, _) => error(operator, "physical operator does not match its RA operator"),
        }
    }

    /// For every outer row, enter the right side at the join key and append
    /// the payload of every match.
    fn probe(&mut self, name: &str, keyed: &physical::Keyed) -> syn::Result<TokenStream> {
        let columns = sorted(&keyed.columns);
        let outer = self.scan(&keyed.left)?;
        let left = symbol_name(&keyed.left);
        let right = symbol_name(&keyed.right);
        let outer_pattern = self.pattern(&left);
        let access = self.keyed(&keyed.right, &columns)?;
        let (lookup, payload, host): (TokenStream, Vec<String>, bool) = match &access {
            Access::Host(access) => {
                let value = &access.value;
                let key = self.host_key(&left, access);
                (
                    quote!(::cq_storage::IndexRead::lookup(&&#value, #key)),
                    access.payload.clone(),
                    true,
                )
            }
            Access::Built(structure) => {
                let key = columns.iter().map(|c| self.ident(&left, c));
                (
                    quote!(::cq_storage::IndexRead::lookup(&#structure, &(#(#key,)*))),
                    self.heading(&right)
                        .iter()
                        .filter(|c| !columns.contains(c))
                        .cloned()
                        .collect(),
                    false,
                )
            }
        };
        // Payload fields are read by position: a host payload is a borrowed
        // tuple, a built payload a value; field access copies the `&T`
        // column out of either.
        let row = self.row(name, |c| match payload.iter().position(|p| p == c) {
            Some(i) => {
                let i = index(i);
                if host {
                    quote!(&__cq_v.#i)
                } else {
                    quote!(__cq_v.#i)
                }
            }
            None => self.ident(&left, c).to_token_stream(),
        });
        Ok(quote! {
            ::core::iter::Iterator::flat_map(#outer, move |#outer_pattern| {
                ::core::iter::Iterator::map(#lookup, move |__cq_v| #row)
            })
        })
    }

    /// Keep the rows whose key the right side contains.
    fn semi(&mut self, keyed: &physical::Keyed) -> syn::Result<TokenStream> {
        let columns = sorted(&keyed.columns);
        let input = self.scan(&keyed.left)?;
        let left = symbol_name(&keyed.left);
        let binders = self.heading(&left).iter().map(|c| {
            if columns.contains(c) {
                self.ident(&left, c).to_token_stream()
            } else {
                quote!(_)
            }
        });
        let test = match self.keyed(&keyed.right, &columns)? {
            Access::Host(access) => {
                let value = &access.value;
                let key = self.host_key(&left, &access);
                quote!(::cq_storage::ExactMembership::contains(&#value, #key))
            }
            Access::Built(structure) => {
                let key = columns.iter().map(|c| self.ident(&left, c));
                quote!(::cq_storage::ExactMembership::contains(&#structure, &(#(#key,)*)))
            }
        };
        Ok(quote! {
            ::core::iter::Iterator::filter(#input, move |&(#(#binders,)*)| #test)
        })
    }

    /// Co-iterate both sides on the sorted key columns.
    fn merge(
        &mut self,
        name: &str,
        reader: &Ident,
        keyed: &physical::Keyed,
    ) -> syn::Result<TokenStream> {
        let columns = sorted(&keyed.columns);
        let left = self.ordered(reader, &keyed.left, &columns)?;
        let right = self.ordered(reader, &keyed.right, &columns)?;
        let native = matches!(
            (&left, &right),
            (Side::Host(_), Side::Host(_)) | (Side::Built(_), Side::Built(_))
        );
        let mut fields: BTreeMap<String, TokenStream> = BTreeMap::new();
        let mut cursors = Vec::new();
        for (side, result, item) in [
            (&left, &keyed.left, format_ident!("__cq_l")),
            (&right, &keyed.right, format_ident!("__cq_r")),
        ] {
            let result = symbol_name(result);
            let payload: Vec<String> = match side {
                Side::Host(access) => access.payload.clone(),
                Side::Built(_) | Side::Pipe(_) => self
                    .heading(&result)
                    .iter()
                    .filter(|c| !columns.contains(c))
                    .cloned()
                    .collect(),
            };
            for (i, column) in payload.iter().enumerate() {
                let i = index(i);
                let field = match side {
                    Side::Host(_) => quote!(&#item.#i),
                    Side::Built(_) | Side::Pipe(_) => quote!(#item.#i),
                };
                fields.insert(column.clone(), field);
            }
            let cursor = match side {
                Side::Host(access) => {
                    let value = &access.value;
                    let cursor = quote!(::cq_storage::OrderedRead::ordered(&#value));
                    if native {
                        cursor
                    } else {
                        let key = (0..columns.len()).map(index).map(|i| quote!(&__cq_k.#i));
                        quote! {
                            ::cq_storage::sorted_by(::core::iter::Iterator::map(
                                #cursor,
                                |(__cq_k, __cq_v)| ((#(#key,)*), __cq_v)
                            ))
                        }
                    }
                }
                Side::Built(structure) => {
                    let cursor = quote!(::cq_storage::OrderedRead::ordered(&#structure));
                    if native {
                        cursor
                    } else {
                        let key = (0..columns.len()).map(index).map(|i| quote!(__cq_k.#i));
                        quote! {
                            ::cq_storage::sorted_by(::core::iter::Iterator::map(
                                #cursor,
                                |(__cq_k, __cq_v)| ((#(#key,)*), __cq_v)
                            ))
                        }
                    }
                }
                Side::Pipe(rows) => {
                    let pattern = self.pattern(&result);
                    let key = columns.iter().map(|c| self.ident(&result, c));
                    let rest = payload.iter().map(|c| self.ident(&result, c));
                    quote! {
                        ::cq_storage::sorted_by(::core::iter::Iterator::map(
                            #rows,
                            move |#pattern| ((#(#key,)*), (#(#rest,)*))
                        ))
                    }
                }
            };
            cursors.push(cursor);
        }
        for (i, column) in columns.iter().enumerate() {
            let i = index(i);
            let field = match (&left, native) {
                (Side::Host(_), true) => quote!(&__cq_k.#i),
                _ => quote!(__cq_k.#i),
            };
            fields.insert(column.clone(), field);
        }
        let row = self.row(name, |c| fields[c].clone());
        let (left, right) = (&cursors[0], &cursors[1]);
        Ok(quote! {
            ::core::iter::Iterator::map(
                ::cq_storage::merge(#left, #right),
                move |(__cq_k, __cq_l, __cq_r)| #row
            )
        })
    }

    /// Enter an ordered value at the select's lower bound on `column` and
    /// leave it at the upper bound; the other conjuncts remain a filter.
    fn range(
        &mut self,
        name: &str,
        reader: &Ident,
        column: &Ident,
        select: &ra::Select,
    ) -> syn::Result<TokenStream> {
        let input = symbol_name(&select.input);
        let column_name = symbol_name(column);
        let window = window(&select.condition, &column_name);
        if window.low.is_none() && window.high.is_none() {
            return error(
                column,
                format!(
                    "`{reader}` ranges over `{column}`, which the condition does not compare with an integer literal"
                ),
            );
        }
        let side = self.ordered(reader, &select.input, std::slice::from_ref(&column_name))?;
        let (cursor, payload, host): (TokenStream, Vec<String>, bool) = match &side {
            Side::Host(access) => {
                let value = &access.value;
                (
                    quote!(::cq_storage::OrderedRead::ordered(&#value)),
                    access.payload.clone(),
                    true,
                )
            }
            Side::Built(structure) => (
                quote!(::cq_storage::OrderedRead::ordered(&#structure)),
                self.heading(&input)
                    .iter()
                    .filter(|c| **c != column_name)
                    .cloned()
                    .collect(),
                false,
            ),
            Side::Pipe(_) => unreachable!("a range reads through a key, never a pipe"),
        };
        // A host key is `(i32,)`, a built key `(&i32,)`.
        let seek = window.low.map(|low| {
            let bound = if host {
                quote!(&&(#low,))
            } else {
                quote!(&&(&#low,))
            };
            quote! { ::cq_storage::OrderedCursor::seek(&mut __cq_cursor, #bound); }
        });
        let scan = match window.high {
            Some(high) => {
                let bound = if host {
                    quote!((#high,))
                } else {
                    quote!((&#high,))
                };
                quote! {
                    ::core::iter::Iterator::take_while(__cq_cursor, |(__cq_k, _)| **__cq_k <= #bound)
                }
            }
            None => quote!(__cq_cursor),
        };
        let row = self.row(name, |c| {
            if c == column_name {
                return if host {
                    quote!(&__cq_k.0)
                } else {
                    quote!(__cq_k.0)
                };
            }
            let i = index(payload.iter().position(|p| p == c).expect("payload column"));
            if host {
                quote!(&__cq_v.#i)
            } else {
                quote!(__cq_v.#i)
            }
        });
        let rows = quote! {{
            let mut __cq_cursor = #cursor;
            #seek
            ::core::iter::Iterator::map(#scan, move |(__cq_k, __cq_v)| #row)
        }};
        if window.residual.is_empty() {
            return Ok(rows);
        }
        let pattern = self.borrowed_pattern(&input);
        let residual = window
            .residual
            .iter()
            .map(deref_predicates)
            .reduce(|left, right| syn::parse_quote!((#left) && (#right)))
            .expect("nonempty residual");
        Ok(quote! {
            ::core::iter::Iterator::filter(
                #rows,
                move |#[allow(unused_variables)] #pattern| #residual
            )
        })
    }

    /// Map the returned rows to the head's argument order, evaluating
    /// computed arguments with every column as `&T`.
    fn head(&self, result: &str, head: &cq::Atom, rows: TokenStream) -> TokenStream {
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

/// The comparison window of a select on one attribute: the tightest integer
/// bounds its `&&`-conjuncts state against literals, and the conjuncts that
/// state something else.
struct Window {
    low: Option<i32>,
    high: Option<i32>,
    residual: Vec<Expr>,
}

fn conjuncts(expr: &Expr) -> Vec<&Expr> {
    match cq::rust::unparen(expr) {
        Expr::Binary(syn::ExprBinary {
            op: syn::BinOp::And(_),
            left,
            right,
            ..
        }) => {
            let mut all = conjuncts(left);
            all.extend(conjuncts(right));
            all
        }
        other => vec![other],
    }
}

fn window(condition: &Expr, column: &str) -> Window {
    use cq::{Comparison, Predicate, Term};
    let mut window = Window {
        low: None,
        high: None,
        residual: Vec::new(),
    };
    for part in conjuncts(condition) {
        // `column cmp literal`, with a literal on the left mirrored.
        let bounds = Predicate::from_expr(part).and_then(|predicate| {
            let (mirrored, literal) = match (&predicate.left, &predicate.right) {
                (Term::Variable(v), Term::Integer(k)) if symbol_name(v) == column => (false, *k),
                (Term::Integer(k), Term::Variable(v)) if symbol_name(v) == column => (true, *k),
                _ => return None,
            };
            let (below, above) = match (predicate.comparison, mirrored) {
                (Comparison::Eq(_), _) => (Some(literal), Some(literal)),
                (Comparison::Ge(_), false) | (Comparison::Le(_), true) => (Some(literal), None),
                (Comparison::Gt(_), false) | (Comparison::Lt(_), true) => {
                    (Some(literal.checked_add(1)?), None)
                }
                (Comparison::Le(_), false) | (Comparison::Ge(_), true) => (None, Some(literal)),
                (Comparison::Lt(_), false) | (Comparison::Gt(_), true) => {
                    (None, Some(literal.checked_sub(1)?))
                }
                (Comparison::Ne(_), _) => return None,
            };
            Some((below, above))
        });
        match bounds {
            Some((below, above)) => {
                if let Some(v) = below {
                    window.low = Some(window.low.map_or(v, |l| l.max(v)));
                }
                if let Some(v) = above {
                    window.high = Some(window.high.map_or(v, |h| h.min(v)));
                }
            }
            None => window.residual.push(part.clone()),
        }
    }
    window
}

/// Bound columns are references; the total comparison fragment is written
/// over values, so its variables are dereferenced here. Other Rust is the
/// author's and is emitted verbatim.
fn deref_predicates(expr: &Expr) -> Expr {
    use cq::{Predicate, Term};
    if let Expr::Binary(syn::ExprBinary {
        op: syn::BinOp::And(and),
        left,
        right,
        ..
    }) = cq::rust::unparen(expr)
    {
        let (left, right) = (deref_predicates(left), deref_predicates(right));
        return syn::parse_quote!((#left) #and (#right));
    }
    let Some(predicate) = Predicate::from_expr(expr) else {
        return expr.clone();
    };
    let term = |term: &Term| -> Expr {
        match term {
            Term::Variable(v) => syn::parse_quote!(*#v),
            Term::Integer(i) => syn::parse_quote!(#i),
        }
    };
    let (left, right, comparison) = (
        term(&predicate.left),
        term(&predicate.right),
        predicate.comparison,
    );
    syn::parse_quote!(#left #comparison #right)
}
