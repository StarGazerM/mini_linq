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
//! requires Clone for retained scans and Copy for payload fan-out, as checked by the emitted Rust. Equality is Leibniz identity.
//! Mutation, I/O and evaluation counts are outside this meaning.
//!
//! Merge and smallest-candidate traversal are unsupported and refused. This
//! printer does not replace a written execution choice with a different one.

//! Push realizes standalone union/anti loops and consumes the prepared fused-dependent pipe once before pushing tuples to its sink.
//! Recorded canonical providers and any/all witness choices are retained.
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
        context: cq_physical_realization::sources(module, &analysis)?,
        consumed: BTreeSet::new(),
    };
    // Hidden row names preserve Rust captures in push loops, where later
    // planned binders would otherwise surround an earlier opaque expression.
    for bindings in push.context.idents.values_mut() {
        for (column, id) in bindings {
            *id = format_ident!("__push_value_{}", column, span = Span::mixed_site());
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
    let mut expression = syn::parse2(quote!({
        #(#statements)*
        let mut #out = ::std::vec::Vec::new();
        #head_state
        #body
        ::core::iter::IntoIterator::into_iter(#out)
    }))?;
    cq_print::own_captures(&mut expression, &push.owned);
    Ok(expression)
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
        let shadows = self.environment_aliases(result, None);
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

    fn fused_dependency(&self, result: &str) -> bool {
        let mut pending = vec![result.to_owned()];
        let mut seen = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let definition = self.context.physical[&name];
            if !matches!(definition.mode, physical::Mode::Pipe(_)) {
                continue;
            }
            if matches!(&definition.operator,Operator::Union(u) if u.fused.is_some()) {
                return true;
            }
            pending.extend(definition.operator.operands().into_iter().map(symbol_name));
        }
        false
    }

    fn emit(&mut self, result: &str, sink: Tokens) -> syn::Result<Tokens> {
        let definition = self.context.physical[result];
        if matches!(definition.mode, physical::Mode::Pipe(_)) && self.fused_dependency(result) {
            if !self.consumed.insert(result.to_owned()) {
                return error(&definition.name, "own_push: fused pipe is read twice");
            }
            let rows = self.context.take_pipe(result).ok_or_else(|| {
                syn::Error::new_spanned(&definition.name, "fused row stream is missing")
            })?;
            let pattern = self.pattern(result);
            return Ok(quote!(for #pattern in #rows { #sink }));
        }
        if let Some(Repr::Let { rows, .. }) = self.reprs.get(result) {
            let Some(storage) = rows else {
                return error(
                    &definition.name,
                    "own_push: retained scan has no recorded rows",
                );
            };
            let pattern = self.pattern(result);
            let rows = if self.infos[result].computed.is_empty() {
                quote!(::core::iter::Iterator::copied(#storage.iter()))
            } else {
                quote!(::core::iter::Iterator::cloned(#storage.iter()))
            };
            return Ok(quote!(#[allow(unused_variables)] for #pattern in #rows { #sink }));
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
            Operator::Union(union) => {
                let mut body = Tokens::new();
                for operand in &union.operands {
                    let input = symbol_name(operand);
                    let pattern = self.pattern(result);
                    let row = self.row(result, |column| {
                        let value = self.ident(&input, column);
                        if self.infos[result].computed.contains(column)
                            && !self.infos[&input].computed.contains(column)
                        {
                            quote!(::core::clone::Clone::clone(#value))
                        } else {
                            value.to_token_stream()
                        }
                    });
                    body.extend(self.emit(&input, quote!(let #pattern = #row; #sink))?);
                }
                Ok(body)
            }
            Operator::Anti(_, keyed) => self.membership(keyed, sink, true),
            Operator::Unit(_) => Ok(sink),
            Operator::Rename(_) => {
                let rows = self.host_scan(result);
                let pattern = self.pattern(result);
                Ok(quote!(#[allow(unused_variables)] for #pattern in #rows { #sink }))
            }
            Operator::Project(project) => self.emit(&symbol_name(&project.input), sink),
            Operator::Point(_, select) | Operator::Range(_, select) => {
                let input_name = symbol_name(&select.input);
                let hidden = self.context.idents[&input_name].clone();
                let output_hidden = self.context.idents[result].clone();
                self.context
                    .idents
                    .insert(result.to_owned(), self.infos[result].bindings.clone());
                self.context
                    .idents
                    .insert(input_name.clone(), self.infos[&input_name].bindings.clone());
                let rows = if matches!(definition.operator, Operator::Point(..)) {
                    self.host_point(result, select)
                } else {
                    self.host_range(result, select)
                };
                self.context.idents.insert(input_name, hidden);
                self.context.idents.insert(result.to_owned(), output_hidden);
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
                let shadows = self.environment_aliases(&input, guard.environment.as_ref());
                self.emit(&input, quote!(if { #(#shadows)* #condition } { #sink }))
            }
            Operator::Bind(bind) => {
                let pattern = &bind.pattern;
                let annotation = &bind.annotation;
                let expression = &bind.expression;
                let input = symbol_name(&bind.input);
                let shadows = self.environment_aliases(&input, bind.environment.as_ref());
                self.emit(
                    &input,
                    {
                        let row = self.row(result,|column| if self.heading(&input).contains(&column.to_owned()) { self.ident(&input,column).to_token_stream() } else { self.infos[result].bindings[column].to_token_stream() });
                        let output = self.pattern(result);
                        quote! { let #pattern #annotation = { #(#shadows)* #expression }; let #output = #row; #sink }
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
                        None => self
                            .constant_value(&right, column)
                            .filter(|_| !columns.iter().any(|key| key == column))
                            .unwrap_or_else(|| self.ident(&left, column).to_token_stream()),
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
            Operator::Semi(_, keyed) => self.membership(keyed, sink, false),
            Operator::Merge(..) => error(&definition.name, "own_push: merge is unsupported"),
        }
    }

    fn membership(
        &mut self,
        keyed: &physical::Keyed,
        sink: Tokens,
        negative: bool,
    ) -> syn::Result<Tokens> {
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
        let test = if negative { quote!(!(#test)) } else { test };
        self.emit(&left, quote!(if #test { #sink }))
    }
    fn select(
        &mut self,
        select: &structures::syntax::algebra::Select,
        sink: Tokens,
    ) -> syn::Result<Tokens> {
        let condition = self.relational_condition(&select.condition);
        let input = symbol_name(&select.input);
        let names = select.condition.columns();
        let environment = structures::syntax::algebra::Environment {
            path: None,
            scope_token: Default::default(),
            bracket: Default::default(),
            variables: names
                .iter()
                .map(|name| self.infos[&input].bindings[name].clone())
                .collect(),
        };
        let shadows = self.environment_aliases(&input, Some(&environment));
        self.emit(&input, quote!(if { #(#shadows)* #condition } { #sink }))
    }
}
