//! Structures → Rust: print one pull program.
//!
//! Queries denote `bottom + Set`: one panic aborts evaluation. Opaque Rust
//! binds and guards keep their written chain; relational steps may move only
//! earlier across them. Their free variables determine their partial-function
//! meaning; I/O, mutation and evaluation counts are outside that meaning.
//! Relational `==` decides Leibniz identity. `Eq` and the selected storage's
//! `Hash` or `Ord` bounds are necessary checks, not a proof of admissibility.
//!
//! This pass has no independent Rust semantic checker. Its obligations are
//! SemanticAgreement against oracle answers and rustc's generated trait
//! guards; authored expansion fixtures separately pin the emitted syntax.
//!
//! The printer preserves the physical operator graph and storage choices. It
//! chooses Rust control flow and layout. When Physical records `probe smallest`,
//! supported adjacent probe/check pairs enumerate the smaller runtime candidate
//! set without changing RA/VO order.
//!
//! Rows borrow declared host columns and retain owned computed columns. The
//! printer binds hosts once and dispatches reusable host reads by type. Only
//! leading declaration prefixes are eligible for native keyed reads; other
//! reads use one lazy fallback. Built lets use their recorded Rust types.
//! Canon boundaries use the recorded dedup type before materialization
//! or stream consumption. Merge requires ordered inputs; mixed cursors normalize
//! key references. Host-only iterators own their shared lazy dispatch state;
//! answers borrowing materialized block-local storage are collected.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! Pull realization executes fused block arms from one prefix row.
//! Union chains arms lazily; fallback evaluates the next arm only when the earlier result is empty.
//! Anti uses the recorded eager/lazy membership route and canon uses the recorded set provider.
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
use cq_lang_structures::physical;
use cq_lang_structures::physical::Operator;
use cq_lang_structures::syntax as cq;
use cq_lang_structures::syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeMap;
use syn::{Expr, Ident};

mod intersection;
mod operators;
mod stage;

use cq_print::access::{Access, Side};
use cq_print::{Context, Repr, error, index, sorted};

/// Print a complete Structures program as a Rust pull expression.
pub fn compile(module: &structures::Program) -> syn::Result<Expr> {
    compile_with_profile(module, false)
}

/// Emit a measured pull expression. Runtime counters must be enabled.
pub fn compile_profile(module: &structures::Program) -> syn::Result<Expr> {
    compile_with_profile(module, true)
}

fn compile_with_profile(module: &structures::Program, profile: bool) -> syn::Result<Expr> {
    let analysis = structures::contract::check(module)?;
    let mut printer = Printer {
        context: sources_profile(module, &analysis, |_, _| {}, profile)?,
        active_prefixes: BTreeMap::new(),
        consumer: None,
    };
    let returned = &module.physical.program.returned;
    let rows = printer.scan(returned)?;
    let answer = format!("{}.answer", module.physical.program.returned);
    if profile {
        printer.profile_label(&answer, "ANSWER", "", vec![symbol_name(returned)]);
    }
    let rows = printer.profile_input(&answer, rows);
    let output = printer.head(&symbol_name(returned), &module.physical.program.head, rows);
    let output = printer.profile_stream(&answer, output);
    if let Some(registration) = printer.profile_registration() {
        printer.statements.insert(0, registration);
    }
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
    let mut expression = syn::parse2(quote! { { #(#statements)* #value } })?;
    cq_print::own_captures(&mut expression, &printer.owned);
    Ok(expression)
}

/// Prepare row sources and recorded storage for a checked Structures program.
/// Alternative emitters can share index construction without staging rows in
/// a second buffer. `analysis` must be the contract result for `module`.
/// Opted-in host-set facts are certified eagerly over complete declared rows
/// before any prepared operator can rely on their uniqueness.
pub fn sources<'a>(
    module: &'a structures::Program,
    analysis: &'a structures::Analysis,
) -> syn::Result<Context<'a>> {
    sources_with(module, analysis, |_, _| {})
}

/// Prepare sources while exposing each representation before its pipe is consumed.
/// Alternative printers can retain the emitted positional stream for fusion into
/// their own sinks. This observes compiler tokens, never traverses runtime rows.
/// `analysis` must be the contract result for `module`.
pub fn sources_with<'a>(
    module: &'a structures::Program,
    analysis: &'a structures::Analysis,
    prepared: impl FnMut(&str, &Repr),
) -> syn::Result<Context<'a>> {
    sources_profile(module, analysis, prepared, false)
}
fn sources_profile<'a>(
    module: &'a structures::Program,
    analysis: &'a structures::Analysis,
    mut prepared: impl FnMut(&str, &Repr),
    profile: bool,
) -> syn::Result<Context<'a>> {
    for definition in &module.physical.program.definitions {
        let name = symbol_name(&definition.name);
        let Some(structures::Repr::Positions { bases, owned, .. }) = analysis.repr.get(&name)
        else {
            continue;
        };
        if bases.len() != 1 || !owned.columns.is_empty() {
            return error(
                &definition.name,
                format!(
                    "{name} repr positions with {} bases / {} owned columns is not realized by print_pull yet",
                    bases.len(),
                    owned.columns.len()
                ),
            );
        }
        let parent = match &definition.operator {
            Operator::Rename(_) => Some(None),
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                Some(Some(&s.input))
            }
            Operator::Project(p) => Some(Some(&p.input)),
            Operator::Semi(_, k) | Operator::Anti(_, k) => Some(Some(&k.left)),
            _ => None,
        };
        if definition.canon.is_some()
            || analysis.physical.modes[&name] != physical::ModeKind::Pipe
            || !parent.is_some_and(|parent| {
                parent.is_none_or(|parent| {
                    matches!(
                        analysis.repr.get(&symbol_name(parent)),
                        Some(structures::Repr::Positions { .. })
                    )
                })
            })
        {
            return error(
                &definition.name,
                format!(
                    "{name} repr positions outside a dependency-closed direct pipe scan chain is not realized by print_pull yet"
                ),
            );
        }
    }
    let mut printer = Printer {
        context: Context::new(module, analysis),
        active_prefixes: BTreeMap::new(),
        consumer: None,
    };
    printer.certify_host_sets(module);
    if profile {
        printer.enable_profile(module);
    }
    let definitions = &module.physical.program.definitions;
    let mut position = 0;
    let deferred: std::collections::BTreeSet<_> = analysis
        .physical
        .fused
        .values()
        .flat_map(|group| group.members.iter().cloned())
        .collect();
    while position < definitions.len() {
        if deferred.contains(&symbol_name(&definitions[position].name)) {
            position += 1;
            continue;
        }
        if matches!(&definitions[position].operator, Operator::Probe(_, k) if matches!(k.choice, Some(physical::Choice::Smallest(_))))
        {
            let Some(second) = definitions.get(position + 1) else {
                return error(
                    &definitions[position].name,
                    "smallest requires its adjacent check",
                );
            };
            if !printer.intersection(&definitions[position], second)? {
                return error(
                    &definitions[position].name,
                    "Structures does not discharge smallest candidate reads",
                );
            }
            for definition in &definitions[position..position + 2] {
                let name = symbol_name(&definition.name);
                prepared(&name, &printer.reprs[&name]);
            }
            position += 2;
        } else {
            printer.define(&definitions[position])?;
            let name = symbol_name(&definitions[position].name);
            prepared(&name, &printer.reprs[&name]);
            position += 1;
        }
    }
    Ok(printer.context)
}

/// Single-operator adapters shared with the Sequence translation.
///
/// The caller supplies checked layouts and operand representations; this API
/// neither checks nor prepares a complete program.
pub struct RowEmitter<'a> {
    printer: Printer<'a>,
}

impl<'a> RowEmitter<'a> {
    pub fn new(context: Context<'a>) -> Self {
        Self {
            printer: Printer {
                context,
                consumer: None,
                active_prefixes: BTreeMap::new(),
            },
        }
    }

    pub fn context(&self) -> &Context<'a> {
        &self.printer.context
    }

    pub fn context_mut(&mut self) -> &mut Context<'a> {
        &mut self.printer.context
    }

    /// Emit exactly one operator, reading caller-supplied operand streams.
    pub fn operator(&mut self, name: &str) -> syn::Result<TokenStream> {
        let definition = self.printer.physical[name];
        let previous = self.printer.consumer.replace(name.to_owned());
        let result = self.printer.produce(name, definition);
        self.printer.consumer = previous;
        result
    }

    /// Read a representation already installed by the caller.
    pub fn scan(&mut self, name: &Ident) -> syn::Result<TokenStream> {
        self.printer.scan(name)
    }

    /// Apply the checked answer projection, including its recorded image canon.
    pub fn head(&mut self, name: &str, head: &cq::Atom, rows: TokenStream) -> TokenStream {
        self.printer.head(name, head, rows)
    }
}

struct Printer<'a> {
    context: Context<'a>,
    consumer: Option<String>,
    active_prefixes: BTreeMap<String, TokenStream>,
}
impl<'a> std::ops::Deref for Printer<'a> {
    type Target = Context<'a>;
    fn deref(&self) -> &Context<'a> {
        &self.context
    }
}
impl std::ops::DerefMut for Printer<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.context
    }
}
impl Printer<'_> {
    pub(crate) fn define(&mut self, definition: &physical::Definition) -> syn::Result<()> {
        let name = symbol_name(&definition.name);
        if !self.idents.contains_key(&name) {
            self.register(&name);
        }
        let consumer = self.consumer.replace(name.clone());
        let repr = match physical::ModeKind::from(&definition.mode) {
            physical::ModeKind::Pipe
                if (!self.carried_witnesses[&name].is_empty()
                    || (self.analysis.stages.contains_key(&name) && !self.probe_stage(&name)))
                    && !matches!(definition.operator, Operator::Rename(_)) =>
            {
                let mut rows = self.produce(&name, definition)?;
                if definition.canon.is_some() {
                    rows = self.distinct(&definition.name, rows);
                }
                Repr::Pipe(Some(self.profile_stream(&name, rows)))
            }
            physical::ModeKind::Pipe if matches!(definition.operator, Operator::Rename(_)) => {
                Repr::Host
            }
            physical::ModeKind::Pipe if matches!(&definition.operator, Operator::Project(p) if definition.canon.is_none() && self.analysis.lineage.contains_key(&name) && matches!(self.reprs.get(&symbol_name(&p.input)), Some(Repr::Positions(_) | Repr::Selection { .. } | Repr::Host)) && self.analysis.reads.values().any(|r| symbol_name(&r.origin.base) == self.analysis.lineage[&name].base)) =>
            {
                let Operator::Project(p) = &definition.operator else {
                    unreachable!()
                };
                let input = symbol_name(&p.input);
                let ids = match self.reprs.get_mut(&input) {
                    Some(Repr::Positions(ids)) => ids.take().expect("single pipe consumer"),
                    Some(Repr::Selection { selection, .. }) => quote!(#selection.positions()),
                    Some(Repr::Host) => {
                        if let Some(ids) = self.join_filter_ids(&name) {
                            ids
                        } else {
                            let host = format_ident!(
                                "__cq_positional_{}",
                                self.analysis.lineage[&name].base,
                                span = proc_macro2::Span::mixed_site()
                            );
                            let ids = quote!(::cq_storage::PositionalRows::positions(#host));
                            self.profile_stream(&input, ids)
                        }
                    }
                    _ => unreachable!(),
                };
                let ids = self.profile_input(&name, ids);
                Repr::Positions(Some(self.profile_stream(&name, ids)))
            }
            physical::ModeKind::Pipe if self.analysis.reads.contains_key(&name) => {
                let ids = self.read_ids(&name)?;
                Repr::Positions(Some(self.profile_stream(&name, ids)))
            }
            physical::ModeKind::Pipe => {
                let mut rows = self.produce(&name, definition)?;
                if definition.canon.is_some() {
                    rows = self.distinct(&definition.name, rows);
                }
                Repr::Pipe(Some(self.profile_stream(&name, rows)))
            }
            physical::ModeKind::Let if self.analysis.selections.contains_key(&name) => {
                let selection = self.selection_source(&name)?;
                self.materialize(&name, selection)
            }
            physical::ModeKind::Let | physical::ModeKind::Exists => {
                let mut rows = self.produce(&name, definition)?;
                if definition.canon.is_some() {
                    rows = self.distinct(&definition.name, rows);
                }
                let rows = self.profile_stream(&name, rows);
                self.materialize(&name, rows)
            }
        };
        self.consumer = consumer;
        self.reprs.insert(name, repr);
        Ok(())
    }

    pub(crate) fn scan(&mut self, result: &Ident) -> syn::Result<TokenStream> {
        let rows = self.scan_source(result)?;
        Ok(match &self.consumer {
            Some(name) => self.profile_input(name, rows),
            None => rows,
        })
    }
    fn scan_source(&mut self, result: &Ident) -> syn::Result<TokenStream> {
        let name = symbol_name(result);
        if let Some(row) = self.active_prefixes.get(&name) {
            return Ok(quote!(::core::iter::once(#row)));
        }
        match self.reprs.get(&name) {
            Some(Repr::Host) => {
                let rows = self
                    .staged_rows(&name)
                    .unwrap_or_else(|| self.host_scan(&name));
                let rows = if self.physical_analysis.canon.contains(&name) {
                    self.distinct(&self.physical[&name].name, rows)
                } else {
                    rows
                };
                Ok(self.profile_stream(&name, rows))
            }
            Some(Repr::Pipe(_)) => match self.take_pipe(&name) {
                Some(rows) => Ok(rows),
                None => error(result, format!("pipe `{name}` is read twice")),
            },
            Some(Repr::Selection { selection, .. }) => Ok(self.selection_rows(&name, selection)),
            Some(Repr::Positions(_)) => {
                let ids = self
                    .take_positions(&name)
                    .ok_or_else(|| syn::Error::new_spanned(result, "positional pipe read twice"))?;
                Ok(self.positional_rows(&name, ids))
            }
            Some(Repr::Let { rows, structures }) => {
                let _ = structures;
                let Some(source) = rows else {
                    return error(result, format!("`{name}` is scanned but not materialized"));
                };
                Ok(if self.infos[&name].computed.is_empty() {
                    quote!(::core::iter::Iterator::copied(#source.iter()))
                } else {
                    quote!(::core::iter::Iterator::cloned(#source.iter()))
                })
            }
            None => error(result, format!("`{name}` is read before its definition")),
        }
    }

    pub(crate) fn distinct(&mut self, operator: &Ident, rows: TokenStream) -> TokenStream {
        let name = symbol_name(operator);
        let ty = self.analysis.dedup[&name].clone();
        self.distinct_width(&name, &ty, rows, self.carried(&name).len())
    }

    fn distinct_width(
        &mut self,
        name: &str,
        ty: &syn::Type,
        rows: TokenStream,
        width: usize,
    ) -> TokenStream {
        // Std implements tuple Eq/Hash/Ord only through arity twelve. Keep
        // iterator rows flat, but recursively group wider equality keys. Clone
        // fields instead of borrowing the filter's temporary row: a provider
        // may retain this key after that row has been yielded and dropped.
        let key = if width <= 12 {
            quote!(__cq_row)
        } else {
            let fields = (0..width)
                .map(|field| {
                    let field = index(field);
                    quote!(::core::clone::Clone::clone(&__cq_row.#field))
                })
                .collect::<Vec<_>>();
            let nested = cq_print::group(&fields);
            quote!(&#nested)
        };
        let dedup = format!("{name}.dedup");
        let input = name.strip_suffix(".head").unwrap_or(name);
        let key_columns = self.heading(input).join(",");
        self.profile_label(&dedup, "DEDUP", &key_columns, vec![input.into()]);
        let rows = self.profile_inserting(&dedup, rows);
        let result = quote! {{
            #[allow(unused_imports)]
            use ::cq_storage::HashSetConstructor as _;
            let mut __cq_seen = <#ty>::new();
            ::core::iter::Iterator::filter(#rows, move |__cq_row| {
                ::cq_storage::require::leibniz(#key);
                __cq_seen.insert(::core::clone::Clone::clone(#key))
            })
        }};
        self.profile_stream(&dedup, result)
    }

    pub(crate) fn head(&mut self, result: &str, head: &cq::Atom, rows: TokenStream) -> TokenStream {
        if self.analysis.head_dedup.contains_key(result) {
            self.profile_label(
                &format!("{result}.head.dedup"),
                "DEDUP",
                &head.args.to_token_stream().to_string(),
                vec![result.into()],
            );
        }
        let rows = if self.carries_positions(result) {
            let pattern = self.bind_columns(result, self.carried(result), false);
            let values = self
                .carried(result)
                .iter()
                .map(|column| self.access(result, column));
            quote!(::core::iter::Iterator::map(#rows, move |#pattern| (#(#values,)*)))
        } else {
            rows
        };
        let heading = self.carried(result);
        let variables: Vec<Option<&Ident>> = head.args.iter().map(cq::rust::variable).collect();
        let identity = variables.len() == heading.len()
            && variables
                .iter()
                .zip(heading)
                .all(|(v, h)| v.is_some_and(|v| symbol_name(v) == *h));
        if identity {
            return if let Some(ty) = self.analysis.head_dedup.get(result) {
                self.distinct_width(
                    &format!("{result}.head"),
                    &ty.clone(),
                    rows,
                    head.args.len(),
                )
            } else {
                rows
            };
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
            let rows = quote! {
                ::core::iter::Iterator::map(#rows, move |#pattern| (#(#output,)*))
            };
            return if let Some(ty) = self.analysis.head_dedup.get(result) {
                self.distinct_width(
                    &format!("{result}.head"),
                    &ty.clone(),
                    rows,
                    head.args.len(),
                )
            } else {
                rows
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
        let mapped = quote! {
            ::core::iter::Iterator::map(#rows, move |#[allow(unused_variables)] #pattern| {
                let (#(#temporaries,)*) = { #(#shadows)* (#(#expressions,)*) };
                (#(#output,)*)
            })
        };
        // Canonical valuations can have the same computed answer. The map is
        // injective only when its bare outputs retain every source column.
        // Otherwise canonicalize the image with the caller's state provider.
        if heading.iter().all(|column| {
            variables
                .iter()
                .flatten()
                .any(|variable| symbol_name(variable) == *column)
        }) {
            mapped
        } else {
            self.distinct_width(
                &format!("{result}.head"),
                &self.analysis.head_dedup[result].clone(),
                mapped,
                head.args.len(),
            )
        }
    }
}
