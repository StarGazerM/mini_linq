//! Structures → Rust: print one pull program.
//!
//! Queries denote `bottom + Set`: one panic aborts evaluation. Opaque Rust
//! binds and guards keep their written chain; relational steps may move only
//! earlier across them. Their free variables determine their partial-function
//! meaning; I/O, mutation and evaluation counts are outside that meaning.
//! Equality, keyed storage and deduplication require Eq, with Hash or Ord
//! supplied by the selected representation.
//!
//! This pass has no independent Rust semantic checker. Its obligations are
//! SemanticAgreement against oracle answers and rustc's generated trait
//! guards; authored expansion fixtures separately pin the emitted syntax.
//!
//! The printer preserves the physical operator graph and storage choices. It
//! chooses Rust control flow and layout. When `Endpoints.intersect` is enabled,
//! supported adjacent probe/check pairs enumerate the smaller runtime candidate
//! set without changing RA/VO order.
//!
//! Rows borrow declared host columns and retain owned computed columns. The
//! printer binds hosts once and dispatches reusable host reads by type. Only
//! leading declaration prefixes are eligible for native keyed reads; other
//! reads use one lazy fallback. Built lets use the caller's index provider.
//! Canon boundaries use the caller's distinct provider before materialization
//! or stream consumption. Merge requires ordered inputs; mixed cursors normalize
//! key references. Host-only iterators own their shared lazy dispatch state;
//! answers borrowing materialized block-local storage are collected.

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

use cq_print::access::{Access, Side};
use cq_print::expressions::relational_condition;
use cq_print::{Context, Repr, error, index, sorted};

/// Caller-resolved macro paths the program is printed into.
pub struct Endpoints {
    /// Distinct-state provider (`cq-protocol`): `new`, `insert`.
    pub distinct_state: syn::Path,
    pub index_provider: syn::Path,
    /// Enumerate the smaller candidate set for supported binary intersections.
    pub intersect: bool,
}

/// Print a complete Structures program as a Rust pull expression.
pub fn compile(module: &structures::Program, endpoints: &Endpoints) -> syn::Result<Expr> {
    let analysis = structures::contract::check(module)?;
    let mut printer = Printer {
        context: Context::new(module, &analysis),
        endpoints,
    };
    printer.intersect = endpoints.intersect;
    let definitions = &module.physical.program.definitions;
    let mut position = 0;
    while position < definitions.len() {
        if endpoints.intersect
            && position + 1 < definitions.len()
            && printer.intersection(&definitions[position], &definitions[position + 1])?
        {
            position += 2;
        } else {
            printer.define(&definitions[position])?;
            position += 1;
        }
    }
    let returned = &module.physical.program.returned;
    let rows = printer.scan(returned)?;
    let output = printer.head(&symbol_name(returned), &module.physical.program.head, rows);
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
    let provider = &endpoints.index_provider;
    let import = quote!(#[allow(unused_imports)] use #provider as index_provider;);
    let mut expression = syn::parse2(quote! { { #import #(#statements)* #value } })?;
    cq_print::own_captures(&mut expression, &printer.owned);
    Ok(expression)
}

struct Printer<'a> {
    context: Context<'a>,
    endpoints: &'a Endpoints,
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
        self.register(&name);
        let repr = match physical::ModeKind::from(&definition.mode) {
            physical::ModeKind::Pipe if matches!(definition.operator, Operator::Rename(_)) => {
                Repr::Host
            }
            physical::ModeKind::Pipe => {
                let mut rows = self.produce(&name, definition)?;
                if definition.canon.is_some() {
                    rows = self.distinct(&definition.name, rows);
                }
                Repr::Pipe(Some(rows))
            }
            physical::ModeKind::Let => {
                let mut rows = self.produce(&name, definition)?;
                if definition.canon.is_some() {
                    rows = self.distinct(&definition.name, rows);
                }
                self.materialize(&name, definition.canon.is_some(), rows)
            }
        };
        self.reprs.insert(name, repr);
        Ok(())
    }

    pub(crate) fn scan(&mut self, result: &Ident) -> syn::Result<TokenStream> {
        let name = symbol_name(result);
        match self.reprs.get(&name) {
            Some(Repr::Host) => {
                let rows = self.host_scan(&name);
                Ok(if self.physical_analysis.canon.contains(&name) {
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
                let _ = structures;
                let Some(source) = rows else {
                    return error(result, format!("`{name}` is scanned but not materialized"));
                };
                Ok(quote!(::core::iter::Iterator::copied(#source.iter())))
            }
            None => error(result, format!("`{name}` is read before its definition")),
        }
    }

    pub(crate) fn distinct(&self, operator: &Ident, rows: TokenStream) -> TokenStream {
        self.distinct_width(operator, rows, self.heading(&symbol_name(operator)).len())
    }

    fn distinct_width(&self, operator: &Ident, rows: TokenStream, width: usize) -> TokenStream {
        let provider = &self.endpoints.distinct_state;
        // Std implements tuple Eq/Hash/Ord only through arity twelve. Keep
        // iterator rows flat, but recursively group wider equality keys. Clone
        // fields instead of borrowing the filter's temporary row: a provider
        // may retain this key after that row has been yielded and dropped.
        fn group(fields: &[TokenStream]) -> TokenStream {
            if fields.len() <= 12 {
                quote!((#(#fields,)*))
            } else {
                group(&fields.chunks(12).map(group).collect::<Vec<_>>())
            }
        }
        let key = if width <= 12 {
            quote!(__cq_row)
        } else {
            let fields = (0..width)
                .map(|field| {
                    let field = index(field);
                    quote!(::core::clone::Clone::clone(&__cq_row.#field))
                })
                .collect::<Vec<_>>();
            let nested = group(&fields);
            quote!(&#nested)
        };
        quote! {{
            let mut __cq_seen = #provider! { operator #operator; new };
            ::core::iter::Iterator::filter(#rows, move |__cq_row| {
                ::cq_storage::require::setoid(#key);
                #provider! { operator #operator; insert(__cq_seen, #key) }
            })
        }}
    }

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
            self.distinct_width(&self.physical[result].name, mapped, head.args.len())
        }
    }
}
