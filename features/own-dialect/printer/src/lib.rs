//! Structures → Rust push printer, composed with `pipeline { print = own_push::push; }`.
//!
//! The complete checked Structures program is the only compiler input. Its
//! Physical operators and concrete read, build, retention and canonical state
//! types are preserved. Nested loops push rows into a Vec endpoint; retained
//! construction uses a traversal buffer followed by the recorded Build and
//! FromIterator implementations through the public `cq-print` helpers.
//!
//! Queries denote `bottom + Set`, refining desugared written order. Opaque Rust
//! binds and guards keep their chain and receive shared references. Host columns
//! stay borrowed. Computed columns are owned; a retained scan or payload fan-out
//! requires Copy, as checked by the emitted Rust. Equality is Leibniz identity.
//! Mutation, I/O and evaluation counts are outside this meaning.
//!
//! Merge and smallest-candidate traversal are unsupported and refused. This
//! printer does not replace a written execution choice with a different one.

use cq_lang_structures as structures;
use cq_print::{Context, Repr, access::Access, error, index, sorted};
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeSet;
use structures::physical::{self, Operator};
use structures::syntax::symbol_name;

/// A terminal Structures → Rust pass with no arguments.
#[proc_macro]
pub fn push(input: TokenStream) -> TokenStream {
    cq_stage::pass::<structures::Program, syn::Expr>(input.into(), |source, arguments| {
        if !arguments.is_empty() {
            return error(&arguments, "own_push::push accepts no arguments");
        }
        compile(source)
    })
    .into()
}

struct Push<'a> {
    context: Context<'a>,
    consumed: BTreeSet<String>,
}
impl<'a> std::ops::Deref for Push<'a> {
    type Target = Context<'a>;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

fn compile(module: &structures::Program) -> syn::Result<syn::Expr> {
    let analysis = structures::contract::check(module)?;
    // Refuse unsupported written choices even in an otherwise unused result.
    for definition in &module.physical.program.definitions {
        match &definition.operator {
            Operator::Merge(..) => {
                return error(&definition.name, "own_push: merge is unsupported");
            }
            Operator::Probe(_, keyed)
                if matches!(keyed.choice, Some(physical::Choice::Smallest(_))) =>
            {
                return error(&definition.name, "own_push: probe smallest is unsupported");
            }
            _ => {}
        }
    }
    let mut push = Push {
        context: Context::new(module, &analysis),
        consumed: BTreeSet::new(),
    };
    for definition in &module.physical.program.definitions {
        let name = symbol_name(&definition.name);
        push.context.register(&name);
        if matches!(
            definition.mode,
            physical::Mode::Let(_) | physical::Mode::Exists(_)
        ) {
            let row = push.pattern(&name);
            let buffer = temporary("rows", &name);
            let body = push.emit(&name, quote!(#buffer.push(#row);))?;
            let source = quote!({
                let mut #buffer = ::std::vec::Vec::new();
                #body
                ::core::iter::IntoIterator::into_iter(#buffer)
            });
            let repr = push
                .context
                .materialize(&name, definition.canon.is_some(), source);
            push.context.reprs.insert(name, repr);
        }
    }
    let returned = &module.physical.program.returned;
    let name = symbol_name(returned);
    let out = temporary("out", &name);
    let body = push.head(&name, &module.physical.program.head, &out);
    let body = push.emit(&name, body)?;
    let head_state = analysis.head_dedup.get(&name).map(|ty| {
        let seen = temporary("head_seen", &name);
        quote!(let mut #seen = <#ty>::new();)
    });
    let statements = &push.statements;
    syn::parse2(quote!({
        #(#statements)*
        let mut #out = ::std::vec::Vec::new();
        #head_state
        #body
        ::core::iter::IntoIterator::into_iter(#out)
    }))
}

fn temporary(role: &str, result: &str) -> syn::Ident {
    format_ident!("__push_{}_{}", role, result, span = Span::mixed_site())
}

// Rust's standard tuple equality implementations end at arity twelve. Group
// wider keys while keeping the public answer and valuation rows flat.
fn grouped(fields: &[Tokens]) -> Tokens {
    if fields.len() <= 12 {
        quote!((#(#fields,)*))
    } else {
        grouped(&fields.chunks(12).map(grouped).collect::<Vec<_>>())
    }
}

fn cloned_key(fields: impl Iterator<Item = Tokens>) -> Tokens {
    grouped(
        &fields
            .map(|field| quote!(::core::clone::Clone::clone(&#field)))
            .collect::<Vec<_>>(),
    )
}

impl Push<'_> {
    fn head(&self, result: &str, head: &structures::syntax::Atom, out: &syn::Ident) -> Tokens {
        let mut bindings = Vec::new();
        let mut fields = Vec::new();
        for (i, expression) in head.args.iter().enumerate() {
            if let Some(variable) = structures::syntax::rust::variable(expression) {
                fields.push(self.ident(result, &symbol_name(variable)).to_token_stream());
            } else {
                let id = temporary(&format!("head_{i}"), result);
                fields.push(id.to_token_stream());
                bindings.push((id, expression));
            }
        }
        let ids = bindings.iter().map(|(id, _)| id);
        let expressions = bindings.iter().map(|(_, expression)| expression);
        let shadows = self.shadows(result);
        let evaluations = (!bindings.is_empty()).then(|| {
            quote! {
                let (#(#ids,)*) = { #(#shadows)* (#(#expressions,)*) };
            }
        });
        let insert = if self.analysis.head_dedup.contains_key(result) {
            let seen = temporary("head_seen", result);
            let key = cloned_key(fields.iter().cloned());
            quote! {
                let __push_key = #key;
                ::cq_storage::require::leibniz(&__push_key);
                if #seen.insert(__push_key) { #out.push((#(#fields,)*)); }
            }
        } else {
            quote!(#out.push((#(#fields,)*));)
        };
        quote! { #evaluations #insert }
    }

    fn emit(&mut self, result: &str, sink: Tokens) -> syn::Result<Tokens> {
        let definition = self.context.physical[result];
        if let Some(Repr::Let { rows, .. }) = self.reprs.get(result) {
            let Some(storage) = rows else {
                return error(
                    &definition.name,
                    "own_push: retained scan has no recorded rows",
                );
            };
            let pattern = self.pattern(result);
            return Ok(
                quote!(#[allow(unused_variables)] for &#pattern in #storage.iter() { #sink }),
            );
        }
        if matches!(definition.mode, physical::Mode::Pipe(_))
            && !matches!(definition.operator, Operator::Rename(_))
            && !self.consumed.insert(result.to_owned())
        {
            return error(
                &definition.name,
                format!("own_push: pipe `{result}` is read twice"),
            );
        }
        if definition.canon.is_some() {
            let seen = temporary("seen", result);
            let key = cloned_key(
                self.heading(result)
                    .iter()
                    .map(|column| self.ident(result, column).to_token_stream()),
            );
            let ty = &self.analysis.dedup[result];
            let initialize = quote!(let mut #seen = <#ty>::new(););
            let body = self.operator(
                result,
                quote! {
                    let __push_key = #key;
                    ::cq_storage::require::leibniz(&__push_key);
                    if #seen.insert(__push_key) { #sink }
                },
            )?;
            return Ok(quote!({ #initialize #body }));
        }
        self.operator(result, sink)
    }

    fn operator(&mut self, result: &str, sink: Tokens) -> syn::Result<Tokens> {
        let definition = self.context.physical[result];
        match &definition.operator {
            Operator::Unit(_) => Ok(sink),
            Operator::Rename(_) => {
                let rows = self.host_scan(result);
                let pattern = self.pattern(result);
                Ok(quote!(#[allow(unused_variables)] for #pattern in #rows { #sink }))
            }
            Operator::Project(project) => self.emit(&symbol_name(&project.input), sink),
            Operator::Point(_, select) | Operator::Range(_, select) => {
                let rows = if matches!(definition.operator, Operator::Point(..)) {
                    self.host_point(result, select)
                } else {
                    self.host_range(result, select)
                };
                if let Some(rows) = rows {
                    let pattern = self.pattern(result);
                    Ok(quote!(#[allow(unused_variables)] for #pattern in #rows { #sink }))
                } else {
                    self.select(select, sink)
                }
            }
            Operator::Select(select) => self.select(select, sink),
            Operator::Guard(guard) => {
                let condition = &guard.condition;
                let input = symbol_name(&guard.input);
                let shadows = self.shadows(&input);
                self.emit(&input, quote!(if { #(#shadows)* #condition } { #sink }))
            }
            Operator::Bind(bind) => {
                let pattern = &bind.pattern;
                let annotation = &bind.annotation;
                let expression = &bind.expression;
                let input = symbol_name(&bind.input);
                let shadows = self.shadows(&input);
                self.emit(
                    &input,
                    quote! {
                        let #pattern #annotation = { #(#shadows)* #expression };
                        #sink
                    },
                )
            }
            Operator::Probe(_, keyed) => {
                let left = symbol_name(&keyed.left);
                let right = symbol_name(&keyed.right);
                let columns = sorted(&keyed.columns);
                let (lookup, payload) = match self.keyed(&keyed.right, &columns)? {
                    Access::Host(access) => {
                        (self.host_lookup(&left, &access, false), access.payload)
                    }
                    Access::Built(structure) => (
                        self.chained_lookup(&left, &structure, &right, &columns),
                        self.built_payload(&right, &self.levels_of(&right, &structure)),
                    ),
                    Access::Prefix(..) => {
                        return error(
                            &keyed.right,
                            "own_push: prefix payload probe is unsupported",
                        );
                    }
                };
                let value = temporary("payload", result);
                let row = self.row(result, |column| {
                    match payload
                        .iter()
                        .position(|p| p == column && !columns.contains(p))
                    {
                        Some(i) => {
                            let i = index(i);
                            quote!(#value.#i)
                        }
                        None => self.ident(&left, column).to_token_stream(),
                    }
                });
                let pattern = self.pattern(result);
                self.emit(
                    &left,
                    quote! {
                        for #value in #lookup {
                            #[allow(unused_variables)] let #pattern = #row;
                            #sink
                        }
                    },
                )
            }
            Operator::Semi(_, keyed) => {
                let left = symbol_name(&keyed.left);
                let right = symbol_name(&keyed.right);
                let test = match self.keyed(&keyed.right, &sorted(&keyed.columns))? {
                    Access::Host(access) => self.host_lookup(&left, &access, true),
                    Access::Built(structure) => self.chained_contains(
                        &left,
                        &structure,
                        &right,
                        &self.levels_of(&right, &structure),
                    ),
                    Access::Prefix(structure, levels) => {
                        self.chained_contains(&left, &structure, &right, &levels)
                    }
                };
                self.emit(&left, quote!(if #test { #sink }))
            }
            Operator::Merge(..) => error(&definition.name, "own_push: merge is unsupported"),
        }
    }

    fn select(
        &mut self,
        select: &structures::syntax::algebra::Select,
        sink: Tokens,
    ) -> syn::Result<Tokens> {
        let condition = self.relational_condition(&select.condition);
        let input = symbol_name(&select.input);
        let shadows = self.shadows(&input);
        self.emit(&input, quote!(if { #(#shadows)* #condition } { #sink }))
    }
}
