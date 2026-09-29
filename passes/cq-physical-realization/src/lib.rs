//! Structures → Rust: print one pull program.
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
//! Representation and execution:
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
//!   owned key is constructed from the bound columns by `Clone::clone`;
//!   built structures are keyed by `(&T, …)`. Distinct-state providers
//!   separately own retained copies and may clone rows.
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
use cq_print::expressions::{deref_predicates, window};
use cq_print::{Context, Repr, error, index, sorted};

/// Caller-resolved macro paths the program is printed into.
pub struct Endpoints {
    /// Distinct-state provider (`cq-protocol`): `new`, `insert`.
    pub distinct_state: syn::Path,
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
    syn::parse2(quote! { { #(#statements)* #value } })
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
            physical::ModeKind::Alias => Repr::Alias,
            physical::ModeKind::Pipe => {
                let mut rows = self.produce(&name, definition)?;
                if definition.canon.is_some() {
                    rows = self.distinct(&definition.name, rows);
                }
                Repr::Pipe(Some(rows))
            }
            physical::ModeKind::Let => {
                let rows = self.produce(&name, definition)?;
                self.materialize(&name, definition.canon.is_some(), rows)
            }
        };
        self.reprs.insert(name, repr);
        Ok(())
    }

    pub(crate) fn scan(&mut self, result: &Ident) -> syn::Result<TokenStream> {
        let name = symbol_name(result);
        match self.reprs.get(&name) {
            Some(Repr::Alias) => {
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
                let heading = self.heading(&name);
                let set = structures
                    .iter()
                    .find(|(c, _)| c.len() == 1 && c[0] == heading)
                    .map(|(_, s)| s);
                let source = match (self.physical_analysis.canon.contains(&name), set, rows) {
                    (true, Some(set), _) => set,
                    (_, _, Some(rows)) => rows,
                    _ => return error(result, format!("`{name}` is scanned but not materialized")),
                };
                Ok(quote!(::core::iter::Iterator::copied(#source.iter())))
            }
            None => error(result, format!("`{name}` is read before its definition")),
        }
    }

    pub(crate) fn distinct(&self, operator: &Ident, rows: TokenStream) -> TokenStream {
        let provider = &self.endpoints.distinct_state;
        quote! {{
            let mut __cq_seen = #provider! { operator #operator; new };
            ::core::iter::Iterator::filter(#rows, move |__cq_row| {
                #provider! { operator #operator; insert(__cq_seen, __cq_row) }
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
        quote! {
            ::core::iter::Iterator::map(#rows, move |#[allow(unused_variables)] #pattern| {
                let (#(#temporaries,)*) = { #(#shadows)* (#(#expressions,)*) };
                (#(#output,)*)
            })
        }
    }
}
