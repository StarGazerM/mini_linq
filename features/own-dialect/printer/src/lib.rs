//! Structures → Rust push printer, composed with `pipeline { print = own_push::push; }`.
//!
//! The complete checked Structures program is the only compiler input. Its
//! Physical operators and concrete read, build, retention and canonical state
//! types are preserved. Nested loops push rows into a Vec endpoint; retained
//! construction shares the recorded Build, FromIterator and Selection providers
//! with pull. Positional pipes fetch survivors directly into their push sink.
//!
//! Queries denote `bottom + Set`, refining desugared written order. Opaque Rust
//! binds and guards keep their chain and receive shared references. Host columns
//! stay borrowed. Computed columns are owned; a retained scan or payload fan-out
//! requires Clone for retained scans and Copy for payload fan-out, as checked by the emitted Rust. Equality is Leibniz identity.
//! Mutation, I/O and evaluation counts are outside this meaning.
//!
//! Merge, smallest-candidate traversal, and positions representations are refused.
//! A recorded `stage = batch N` bounds lookahead from above (formal rule ST-Stage:
//! at most N rows ahead). Plain host selection chains run chunked at N; probe
//! outers keep the shared prepared lookahead; every other stage shape runs at
//! the admissible scalar bound rather than being refused.

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
use std::collections::{BTreeMap, BTreeSet};
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
    positions: BTreeMap<String, Tokens>,
    /// Staged results realized chunk by chunk; other stages run at the scalar bound.
    chunked: BTreeSet<String>,
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
    for entry in &module.structures.entries {
        match &entry.structure {
            structures::Structure::Repr { witness: Some(_), .. } => {
                return error(entry, format!("own_push: `{} repr witness` is unsupported", entry.name));
            }
            structures::Structure::Stage { expand: Some(_), .. } => {
                return error(entry, format!("own_push: `{} stage expand` is unsupported", entry.name));
            }
            _ => {}
        }
        if let structures::Structure::Repr {
            repr: structures::Repr::Positions { .. },
            ..
        } = &entry.structure
        {
            let name = symbol_name(&entry.name);
            return error(
                entry,
                format!("own_push: `{name} repr = positions` is unsupported"),
            );
        }
    }
    let mut positions = BTreeMap::new();
    let context = cq_physical_realization::sources_with(module, &analysis, |name, repr| {
        if let Repr::Positions(Some(ids)) = repr {
            positions.insert(name.to_owned(), ids.clone());
        }
    })?;
    let chunked = analysis
        .stages
        .keys()
        .filter(|name| selection_chain(module, &analysis, name).is_some())
        .cloned()
        .collect();
    let mut push = Push {
        context,
        consumed: BTreeSet::new(),
        positions,
        chunked,
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
        #[allow(unused_imports)]
        use ::cq_storage::HashSetConstructor as _;
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

fn cloned_key(fields: impl Iterator<Item = Tokens>) -> Tokens {
    cq_print::group(
        &fields
            .map(|field| quote!(::core::clone::Clone::clone(&#field)))
            .collect::<Vec<_>>(),
    )
}

/// The supported stage is one uninterrupted selection spine over a host scan.
fn selection_chain(
    module: &structures::Program,
    analysis: &structures::Analysis,
    result: &str,
) -> Option<()> {
    let mut current = result.to_owned();
    loop {
        let definition = module
            .physical
            .program
            .definitions
            .iter()
            .find(|definition| symbol_name(&definition.name) == current)?;
        if current != result
            && (definition.canon.is_some()
                || !matches!(definition.mode, physical::Mode::Pipe(_))
                || analysis.stages.contains_key(&current))
        {
            return None;
        }
        if analysis.reads.contains_key(&current) {
            return None;
        }
        match &definition.operator {
            Operator::Rename(_) => return Some(()),
            Operator::Project(project) => current = symbol_name(&project.input),
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                current = symbol_name(&select.input);
            }
            _ => return None,
        }
    }
}

impl Push<'_> {
    fn head(&self, result: &str, head: &structures::syntax::Atom, out: &syn::Ident) -> Tokens {
        let mut bindings = Vec::new();
        let mut fields = Vec::new();
        for (i, expression) in head.args.iter().enumerate() {
            if let Some(variable) = structures::syntax::rust::variable(expression) {
                fields.push(self.access(result, &symbol_name(variable)));
            } else {
                let id = temporary(&format!("head_{i}"), result);
                fields.push(id.to_token_stream());
                bindings.push((id, expression));
            }
        }
        let ids = bindings.iter().map(|(id, _)| id);
        let expressions = bindings.iter().map(|(_, expression)| expression);
        let shadows = bindings
            .iter()
            .flat_map(|(_, expression)| self.rust_aliases(result, expression, None));
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

    // Shared preparation preserves fused branches and recorded built-probe
    // lookahead without imposing PositionalRows on ordinary keyed hosts.
    fn prepared_dependency(&self, result: &str) -> bool {
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
            if matches!(&definition.operator,Operator::Union(u) if u.fused.is_some())
                || matches!(&definition.operator, Operator::Probe(_, keyed)
                    if self.analysis.stages.contains_key(&symbol_name(&keyed.left)))
            {
                return true;
            }
            pending.extend(definition.operator.operands().into_iter().map(symbol_name));
        }
        false
    }

    fn emit(&mut self, result: &str, sink: Tokens) -> syn::Result<Tokens> {
        let definition = self.context.physical[result];
        if matches!(definition.mode, physical::Mode::Pipe(_)) && self.prepared_dependency(result) {
            if !self.consumed.insert(result.to_owned()) {
                return error(&definition.name, "own_push: prepared pipe is read twice");
            }
            let rows = self.context.take_pipe(result).ok_or_else(|| {
                syn::Error::new_spanned(&definition.name, "prepared row stream is missing")
            })?;
            let pattern = self.bind_columns(result, self.carried(result), false);
            return Ok(quote!(for #pattern in #rows { #sink }));
        }
        // Formal rule IF-PushPositional: Push accepted positions straight to the sink.
        // Preparation already fused the checked read spine and all residuals.
        if let Some(ids) = self.positions.remove(result) {
            if !self.consumed.insert(result.to_owned()) {
                return error(&definition.name, "own_push: positional pipe is read twice");
            }
            let host = format_ident!(
                "__cq_positional_{}",
                self.analysis.lineage[result].base,
                span = Span::mixed_site()
            );
            let pattern = self.positional_pattern(result);
            let id = temporary("position", result);
            return Ok(quote! {
                for #id in #ids {
                    #[allow(unused_variables)]
                    let #pattern = ::cq_storage::PositionalRows::row_at(#host, #id)
                        .expect("host-issued live ID");
                    #sink
                }
            });
        }
        if let Some(Repr::Selection { selection, .. }) = self.reprs.get(result) {
            let pattern = self.positional_pattern(result);
            return Ok(
                quote!(#[allow(unused_variables)] for #pattern in #selection.rows() { #sink }),
            );
        }
        if let Some(Repr::Let { rows, .. }) = self.reprs.get(result) {
            let Some(storage) = rows else {
                return error(
                    &definition.name,
                    "own_push: retained scan has no recorded rows",
                );
            };
            let pattern = self.bind_columns(result, self.carried(result), false);
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
                    .map(|column| self.access(result, column)),
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
        if self.chunked.contains(result) {
            return self.staged(result, self.analysis.stages[result], sink);
        }
        match &definition.operator {
            Operator::Union(union) => {
                let mut body = Tokens::new();
                for operand in &union.operands {
                    let input = symbol_name(operand);
                    let pattern = self.bind_columns(result, self.carried(result), false);
                    let row = self.carrier(result, |column| {
                        let value = self.access(&input, column);
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
                let pattern = self.bind_columns(result, self.carried(result), false);
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
                self.context.idents.insert(input_name.clone(), hidden);
                self.context.idents.insert(result.to_owned(), output_hidden);
                if let Some(rows) = rows {
                    // The shared read yields the input's carrier; narrow it to
                    // the result's carried columns before binding the loop.
                    let rows = self.forward(result, &input_name, rows);
                    let pattern = self.bind_columns(result, self.carried(result), false);
                    Ok(quote!(#[allow(unused_variables)] for #pattern in #rows { #sink }))
                } else {
                    self.select(select, sink)
                }
            }
            Operator::Select(select) => self.select(select, sink),
            Operator::Guard(guard) => {
                let condition = &guard.condition;
                let input = symbol_name(&guard.input);
                let shadows = self.rust_aliases(&input, condition, guard.environment.as_ref());
                self.emit(&input, quote!(if { #(#shadows)* #condition } { #sink }))
            }
            Operator::Bind(bind) => {
                let pattern = &bind.pattern;
                let annotation = &bind.annotation;
                let expression = &bind.expression;
                let input = symbol_name(&bind.input);
                let shadows = self.rust_aliases(&input, expression, bind.environment.as_ref());
                self.emit(
                    &input,
                    {
                        let row = self.carrier(result, |column| {
                            if self.heading(&input).iter().any(|c| c == column) {
                                self.access(&input, column)
                            } else {
                                self.infos[result].bindings[column].to_token_stream()
                            }
                        });
                        let output = self.bind_columns(result, self.carried(result), false);
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
                let row = self.carrier(result, |column| {
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
                            .unwrap_or_else(|| self.access(&left, column)),
                    }
                });
                let pattern = self.bind_columns(result, self.carried(result), false);
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
    fn staged(&self, result: &str, size: usize, sink: Tokens) -> syn::Result<Tokens> {
        let mut chain = Vec::new();
        let mut current = result.to_owned();
        loop {
            let definition = self.context.physical[&current];
            chain.push(definition);
            match &definition.operator {
                Operator::Rename(_) => break,
                Operator::Project(project) => current = symbol_name(&project.input),
                Operator::Select(select)
                | Operator::Point(_, select)
                | Operator::Range(_, select) => current = symbol_name(&select.input),
                _ => unreachable!("chunked stages are plain selection chains"),
            }
        }
        let host = self.host_access(&current, &[]).value;
        let chunk = temporary("chunk", result);
        let id = temporary("position", result);
        let mut predicates = Vec::new();
        for definition in chain.iter().rev() {
            if let Operator::Select(select)
            | Operator::Point(_, select)
            | Operator::Range(_, select) = &definition.operator
            {
                let input = symbol_name(&select.input);
                let pattern = self.positional_pattern(&input);
                let condition = self.relational_condition(&select.condition);
                let shadows = self.predicate_aliases(&input, &select.condition);
                predicates.push(quote! {
                    #chunk.select_with(|#id| {
                        #[allow(unused_variables)]
                        let #pattern = ::cq_storage::PositionalRows::row_at(#host, #id)
                            .expect("host-issued live ID");
                        #(#shadows)*
                        #condition
                    });
                });
            }
        }
        let pattern = self.positional_pattern(result);
        Ok(quote! {
            for mut #chunk in ::cq_storage::stage::for_chunks::<#size, _>(#host) {
                #(#predicates)*
                for #id in #chunk.survivors() {
                    #[allow(unused_variables)]
                    let #pattern = ::cq_storage::PositionalRows::row_at(#host, #id)
                        .expect("host-issued live ID");
                    #sink
                }
            }
        })
    }

    fn predicate_aliases(
        &self,
        input: &str,
        condition: &structures::syntax::algebra::Theta,
    ) -> Vec<Tokens> {
        cq_print::columns_used(condition)
            .iter()
            .map(|column| {
                let logical = &self.infos[input].bindings[column];
                let value = self.access(input, column);
                if self.infos[input].computed.contains(column) {
                    quote!(let #logical = &#value;)
                } else {
                    quote!(let #logical = #value;)
                }
            })
            .collect()
    }

    fn membership(
        &mut self,
        keyed: &physical::Keyed,
        sink: Tokens,
        negative: bool,
    ) -> syn::Result<Tokens> {
        let left = symbol_name(&keyed.left);
        let right = symbol_name(&keyed.right);
        let columns = sorted(&keyed.columns);
        let test = if let Some(test) = self.keyset_contains(&left, &right, &columns) {
            test
        } else {
            match self.keyed(&keyed.right, &columns)? {
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
        let shadows = self.predicate_aliases(&input, &select.condition);
        self.emit(&input, quote!(if { #(#shadows)* #condition } { #sink }))
    }
}
