//! Structures → Rust push printer, composed with `pipeline { print_input = structures; print = own_push::push; }`.
//!
//! This explicit terminal keeps its Structures boundary. The default pull route
//! instead lowers through `sequence`, checks `sequential` grading, and ends at
//! `translate_pull`; direct comparisons must supply that complete pass queue.
//!
//! The complete checked Structures program is the only compiler input. Its
//! Physical operators and concrete read, build, retention and canonicalization
//! strategies are preserved. `push` collects results into a Vec endpoint;
//! `push_into` invokes its consumer after any recorded canonicalization barrier.
//! Retained construction shares the recorded Build, FromIterator and Selection
//! providers with pull. Positional pipes fetch survivors directly into their sink.
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
//! Optional calibration observes these same push loops through the shared
//! OperatorProgram counters; local row tallies merge on exhaustion or unwind.
//! Recorded canonicalization strategies and any/all witness choices are retained.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only fallback construction proves union arms
//! disjoint; literal values prove nothing, and no data keys or constraints are assumed.
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
    cq_stage::Structures::<physical::Program, structures::Program>::pass::<syn::Expr>(input.into(), |physical, source, arguments| {
        if !arguments.is_empty() {
            return error(&arguments, "own_push::push accepts no arguments");
        }
        compile_observed(physical, source, None)
    })
    .into()
}

mod kw {
    syn::custom_keyword!(consumer);
}

struct Consumer(syn::Expr);
impl syn::parse::Parse for Consumer {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::consumer>()?;
        let contents;
        syn::braced!(contents in input);
        let expression = contents.parse()?;
        if !contents.is_empty() {
            return Err(contents.error("expected one consumer expression"));
        }
        input.parse::<syn::Token![;]>()?;
        Ok(Self(expression))
    }
}

/// Structures → unit, invoking an `FnMut(Row)` consumer once per answer.
///
/// Select `print_input = structures; print = own_push::push_into(consumer { callback };)`
/// or use `arguments { consumer { callback }; };` at a direct pass entry.
/// Consumer evaluation happens once. Whole-stream canon buffers before callbacks;
/// paths without a canon barrier invoke the callback as rows become available.
#[proc_macro]
pub fn push_into(input: TokenStream) -> TokenStream {
    cq_stage::Structures::<physical::Program, structures::Program>::pass::<syn::Expr>(input.into(), |physical, source, arguments| {
        let consumer: Consumer = syn::parse2(arguments)?;
        compile_observed(physical, source, Some(&consumer.0))
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

fn compile(physical: &physical::Program, module: &structures::Program, consumer: Option<&syn::Expr>) -> syn::Result<syn::Expr> {
    compile_mode(physical, module, consumer, false)
}

fn compile_observed(physical: &physical::Program, module: &structures::Program, consumer: Option<&syn::Expr>) -> syn::Result<syn::Expr> {
    let plain = compile(physical, module, consumer)?;
    let calibrated = compile_mode(physical, module, consumer, true)?;
    syn::parse2(quote!(::cq_storage::calibration_select! {
        plain { #plain }
        calibration { #calibrated }
    }))
}

fn compile_mode(physical: &physical::Program, module: &structures::Program, consumer: Option<&syn::Expr>, profile: bool) -> syn::Result<syn::Expr> {
    let analysis = structures::contract::check(physical, module)?;
    // Refuse unsupported written choices even in an otherwise unused result.
    for definition in &physical.program.definitions {
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
            structures::Structure::Repr {
                witness: Some(_), ..
            } => {
                return error(
                    entry,
                    format!("own_push: `{} repr witness` is unsupported", entry.name),
                );
            }
            structures::Structure::Stage {
                expand: Some(_), ..
            } => {
                return error(
                    entry,
                    format!("own_push: `{} stage expand` is unsupported", entry.name),
                );
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
    let prepared = |name: &str, repr: &Repr| {
        if let Repr::Positions(Some(ids)) = repr {
            positions.insert(name.to_owned(), ids.clone());
        }
    };
    let context = if profile {
        cq_physical_realization::sources_profile_with(physical, module, &analysis, prepared)?
    } else {
        cq_physical_realization::sources_with(physical, module, &analysis, prepared)?
    };
    let chunked = analysis
        .stages
        .keys()
        .filter(|name| selection_chain(physical, &analysis, name).is_some())
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
    let returned = &physical.program.returned;
    let name = symbol_name(returned);
    let out = temporary("out", &name);
    let head_rows = temporary("head_rows", &name);
    let canonical_head = analysis.head_dedup.get(&name);
    let body = push.head(
        &name,
        &physical.program.head,
        if canonical_head.is_some() {
            &head_rows
        } else {
            &out
        },
        canonical_head.is_none() && consumer.is_some(),
    );
    let body = push.emit(&name, body)?;
    let head_state = canonical_head.map(|_| quote!(let mut #head_rows = ::std::vec::Vec::new();));
    let head_publish = canonical_head.map(|provider| {
        let rows = cq_physical_realization::RowEmitter::canon(
            &provider.ty,
            quote!(#head_rows),
            physical.program.head.args.len(),
        );
        if consumer.is_some() {
            quote!(for __push_answer in #rows { (#out)(__push_answer); })
        } else {
            quote!(#out.extend(#rows);)
        }
    });
    let registration = push.context.profile_registration();
    let statements = &push.statements;
    let (endpoint, result) = match consumer {
        Some(consumer) => (quote!(let mut #out = #consumer;), Tokens::new()),
        None => (
            quote!(let mut #out = ::std::vec::Vec::new();),
            quote!(::core::iter::IntoIterator::into_iter(#out)),
        ),
    };
    // IF-Capture: `if` captures decide once whether the push loops run at all.
    let gates = &push.gates;
    let run = if gates.is_empty() {
        quote!(#body #head_publish)
    } else {
        quote!(if #(#gates)&&* { #body #head_publish })
    };
    let mut expression = syn::parse2(quote!({
        #registration
        #(#statements)*
        #endpoint
        #head_state
        #run
        #result
    }))?;
    // Move closures own clones of the capture values they read.
    cq_print::own_captures(&mut expression, &push.owned, &push.captured_values);
    Ok(expression)
}

fn temporary(role: &str, result: &str) -> syn::Ident {
    format_ident!("__push_{}_{}", role, result, span = Span::mixed_site())
}

/// The supported stage is one uninterrupted selection spine over a host scan.
fn selection_chain(
    physical: &physical::Program,
    analysis: &structures::Analysis,
    result: &str,
) -> Option<()> {
    let mut current = result.to_owned();
    loop {
        let definition = physical
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
    fn head(
        &self,
        result: &str,
        head: &structures::syntax::Atom,
        out: &syn::Ident,
        consumer: bool,
    ) -> Tokens {
        let mut bindings = Vec::new();
        let mut fields = Vec::new();
        for (i, expression) in head.args.iter().enumerate() {
            if let Some(variable) = structures::syntax::rust::variable(expression) {
                let name = symbol_name(variable);
                // A capture copies its host value; it is never a row column.
                fields.push(
                    self.capture_value(&name)
                        .unwrap_or_else(|| self.access(result, &name)),
                );
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
        let emit = if consumer {
            quote!((#out)((#(#fields,)*));)
        } else {
            quote!(#out.push((#(#fields,)*));)
        };
        quote! { #evaluations #emit }
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
            if !self.streamed(&name) {
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

    /// A pipe, or a stored result whose single-consumer rows record is moved
    /// into its reader (STR-RowsByValue): both are prepared row streams.
    fn streamed(&self, result: &str) -> bool {
        matches!(self.context.physical[result].mode, physical::Mode::Pipe(_))
            || matches!(self.context.reprs.get(result), Some(Repr::Pipe(_)))
    }

    fn emit(&mut self, result: &str, sink: Tokens) -> syn::Result<Tokens> {
        let measured = self.context.profile_index(result).filter(|_| {
            !(self.streamed(result) && self.prepared_dependency(result))
                && !self.positions.contains_key(result)
                && !matches!(self.reprs.get(result), Some(Repr::Selection { .. } | Repr::Let { .. }))
                && !self.chunked.contains(result)
        });
        let Some(index) = measured else { return self.emit_inner(result, sink); };
        let count = temporary("count", result);
        let profile = self.profile_handle().clone();
        let body = self.emit_inner(result, quote!(#count.output(); #sink))?;
        Ok(quote!({
            let mut #count = #profile.push(#index);
            #body
        }))
    }

    fn emit_inner(&mut self, result: &str, sink: Tokens) -> syn::Result<Tokens> {
        let definition = self.context.physical[result];
        if self.streamed(result) && self.prepared_dependency(result) {
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
            let pattern = self.bind_columns(result, self.carried(result), false);
            let provider = match self.analysis.canonical[result].clone() {
                structures::CanonSource::Host => {
                    let rows = cq_physical_realization::RowEmitter::ordered_source(
                        &self.context,
                        cq_physical_realization::CanonTarget::Sequential,
                        result,
                    );
                    return Ok(quote!(for #pattern in #rows { #sink }));
                }
                structures::CanonSource::Rows(provider)
                    if matches!(definition.mode, physical::Mode::Let(_)) =>
                {
                    provider
                }
                structures::CanonSource::Rows(_) | structures::CanonSource::Index(_) => {
                    return self.operator(result, sink);
                }
            };
            let buffer = temporary("canon_rows", result);
            let row = self.carrier(result, |column| self.access(result, column));
            let rows = cq_physical_realization::RowEmitter::canon(
                &provider.ty,
                quote!(#buffer),
                self.carried(result).len(),
            );
            let body = self.operator(result, quote!(#buffer.push(#row);))?;
            return Ok(quote!({
                let mut #buffer = ::std::vec::Vec::new();
                #body
                for #pattern in #rows { #sink }
            }));
        }
        self.operator(result, sink)
    }

    fn emit_input(&mut self, result: &str, input: &str, sink: Tokens) -> syn::Result<Tokens> {
        let sink = if self.profile_index(result).is_some() {
            let count = temporary("count", result);
            quote!(#count.input(); #sink)
        } else {
            sink
        };
        self.emit(input, sink)
    }

    fn operator(&mut self, result: &str, sink: Tokens) -> syn::Result<Tokens> {
        let definition = self.context.physical[result];
        if self.chunked.contains(result) {
            return self.staged(result, self.analysis.stages[result], sink);
        }
        match &definition.operator {
            Operator::Group(_) => error(
                &definition.name,
                "own_push: grouped folds require the Sequence pull or Tasks translation",
            ),
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
                    body.extend(self.emit_input(result, &input, quote!(let #pattern = #row; #sink))?);
                }
                Ok(body)
            }
            Operator::Anti(_, keyed) => self.membership(result, keyed, sink, true),
            Operator::Unit(_) => Ok(sink),
            Operator::Rename(_) => {
                let rows = self.host_scan(result);
                let pattern = self.bind_columns(result, self.carried(result), false);
                Ok(quote!(#[allow(unused_variables)] for #pattern in #rows { #sink }))
            }
            Operator::Project(project) => self.emit_input(result, &symbol_name(&project.input), sink),
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
                    self.select(result, select, sink)
                }
            }
            Operator::Select(select) => self.select(result, select, sink),
            Operator::Guard(guard) => {
                let input = symbol_name(&guard.input);
                let shadows =
                    self.rust_aliases(&input, &guard.condition, guard.environment.as_ref());
                let condition = guard.test();
                self.emit_input(result, &input, quote!(if { #(#shadows)* #condition } { #sink }))
            }
            Operator::Bind(bind) => {
                let pattern = &bind.pattern;
                let annotation = &bind.annotation;
                let input = symbol_name(&bind.input);
                let shadows =
                    self.rust_aliases(&input, &bind.expression, bind.environment.as_ref());
                let expression = match self.binding_functions.get(result) {
                    Some((function, arguments)) => quote!(#function(#(#arguments),*)),
                    None => bind.value().to_token_stream(),
                };
                self.emit_input(
                    result,
                    &input,
                    {
                        let row = self.carrier(result, |column| {
                            if self.heading(&input).iter().any(|c| c == column) {
                                let value = self.access(&input, column);
                                if bind.for_token.is_some()
                                    && self.infos[&input].computed.contains(column)
                                {
                                    quote!(::core::clone::Clone::clone(&#value))
                                } else {
                                    value
                                }
                            } else {
                                self.infos[result].bindings[column].to_token_stream()
                            }
                        });
                        let output = self.bind_columns(result, self.carried(result), false);
                        if bind.for_token.is_some() {
                            let item = temporary("generated", result);
                            quote! {
                                for #item in { #(#shadows)* #expression } {
                                    #[allow(unused_variables)]
                                    let #pattern #annotation = #item;
                                    let #output = #row;
                                    #sink
                                }
                            }
                        } else {
                            quote! { let #pattern #annotation = { #(#shadows)* #expression }; let #output = #row; #sink }
                        }
                    },
                )
            }
            Operator::Probe(_, keyed) => {
                let left = symbol_name(&keyed.left);
                let right = symbol_name(&keyed.right);
                let columns = sorted(&keyed.columns);
                let (lookup, payload) = match self.keyed(result, &keyed.right, &columns)? {
                    Access::Rows => {
                        let row = self.carrier(result, |column| {
                            let source = if self.heading(&right).iter().any(|c| c == column) {
                                &right
                            } else {
                                &left
                            };
                            let value = self.access(source, column);
                            if source == &left && self.infos[source].computed.contains(column) {
                                quote!(::core::clone::Clone::clone(&#value))
                            } else {
                                value
                            }
                        });
                        let pattern = self.bind_columns(result, self.carried(result), false);
                        let inner = self.emit(&right, quote!(let #pattern = #row; #sink))?;
                        return self.emit_input(result, &left, inner);
                    }
                    Access::Host(access) => (
                        self.host_lookup(&left, &access, false, false),
                        access.payload,
                    ),
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
                            .unwrap_or_else(|| {
                                let value = self.access(&left, column);
                                if self.infos[&left].computed.contains(column) {
                                    quote!(::core::clone::Clone::clone(&#value))
                                } else {
                                    value
                                }
                            }),
                    }
                });
                let pattern = self.bind_columns(result, self.carried(result), false);
                self.emit_input(
                    result,
                    &left,
                    quote! {
                        for #value in #lookup {
                            #[allow(unused_variables)] let #pattern = #row;
                            #sink
                        }
                    },
                )
            }
            Operator::Semi(_, keyed) => self.membership(result, keyed, sink, false),
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
        let host = self.host_access(&current, None, &[]).value;
        let chunk = temporary("chunk", result);
        let id = temporary("position", result);
        let mut predicates = Vec::new();
        for definition in chain.iter().rev() {
            let index = self.profile_index(&symbol_name(&definition.name));
            let before = temporary("stage_input", &symbol_name(&definition.name));
            if index.is_some() {
                predicates.push(quote!(let #before = #chunk.survivors().size_hint().0 as u64;));
            }
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
            if let Some(index) = index {
                let profile = self.profile_handle();
                let input = if matches!(definition.operator, Operator::Rename(_)) {
                    quote!(0)
                } else {
                    quote!(#before)
                };
                predicates.push(quote!(
                    #profile.rows(#index, #input, #chunk.survivors().size_hint().0 as u64, 0);
                ));
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
        reader: &str,
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
            match self.keyed(reader, &keyed.right, &columns)? {
                Access::Rows => {
                    let label =
                        syn::Lifetime::new(&format!("'__cq_nonempty_{reader}"), Span::mixed_site());
                    let rows = self.emit(&right, quote!(break #label true;))?;
                    quote!(#label: { #rows false })
                }
                Access::Host(access) => self.host_lookup(&left, &access, true, negative),
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
        self.emit_input(reader, &left, quote!(if #test { #sink }))
    }
    fn select(
        &mut self,
        result: &str,
        select: &structures::syntax::algebra::Select,
        sink: Tokens,
    ) -> syn::Result<Tokens> {
        let condition = self.relational_condition(&select.condition);
        let input = symbol_name(&select.input);
        let shadows = self.predicate_aliases(&input, &select.condition);
        self.emit_input(result, &input, quote!(if { #(#shadows)* #condition } { #sink }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_stack(tokens: Tokens) -> syn::Result<(physical::Program, structures::Program)> {
        syn::parse::Parser::parse2(|input: syn::parse::ParseStream<'_>| {
            let physical = input.parse()?;
            let structures = structures::Program::parse(input, &physical)?;
            Ok((physical, structures))
        }, tokens)
    }

    #[test]
    fn certificates_distinguish_local_checks_from_discharged_facts() {
        for (fact, certified) in [
            (quote!(host_set R;), true),
            (quote!(host_set R discharged;), false),
        ] {
            let (physical, module) = parse_stack(quote! {
                physical {
                    relation R(a:i32); #fact
                    r = pipe rename R@0 {a -> a};
                    return r as answer(a);
                }
                structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; } r host scan = @host_scan as ::core::iter::Map<_,_> []; }
            })
            .unwrap();
            let printed = compile(&physical, &module, None)
                .unwrap()
                .to_token_stream()
                .to_string();
            assert_eq!(printed.contains("__cq_host_set_seen"), certified);
        }
    }

    #[test]
    fn canonical_consumer_expansion_has_no_local_answer_set() {
        for (head, project) in [
            (quote!(a, b), false),
            (quote!(a), true),
            (quote!(*a % 2), false),
        ] {
            let projection = project.then(|| quote!(p = pipe project r keep {a};));
            let returned = if project { quote!(p) } else { quote!(r) };
            let (physical, module) = parse_stack(quote! {
                physical {
                    relation R(a:i32,b:i32);
                    consumer canonical;
                    r = pipe rename R@0 {a -> a,b -> b};
                    #projection
                    return #returned as answer(#head);
                }
                structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; } r host scan = @host_scan as ::core::iter::Map<_,_> []; }
            })
            .unwrap();
            let callback = syn::parse_quote!(consume);
            for consumer in [None, Some(&callback)] {
                let printed = compile(&physical, &module, consumer)
                    .unwrap()
                    .to_token_stream()
                    .to_string();
                assert!(!printed.contains("head_seen"), "{printed}");
                assert!(!printed.contains("HashSet <"), "{printed}");
                assert!(!printed.contains("__cq_seen"), "{printed}");
            }
        }
    }

    #[test]
    fn calibrated_push_keeps_loops_and_registers_real_operator_counters() {
        let (physical, module) = parse_stack(quote! {
            physical {
                relation R(a:i32);
                consumer canonical;
                r = pipe rename R@0 {a -> a};
                p = pipe project r keep {a};
                return p as answer(a);
            }
            structures {
                providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; }
                r host scan = @host_scan as ::core::iter::Map<_,_> [];
            }
        }).unwrap();
        let consumer = syn::parse_quote!(consume);
        let plain = compile(&physical, &module, Some(&consumer)).unwrap().to_token_stream().to_string();
        let observed = compile_mode(&physical, &module, Some(&consumer), true).unwrap().to_token_stream().to_string();
        assert!(!plain.contains("OperatorProgram"));
        assert!(observed.contains("OperatorProgram :: new"));
        assert!(observed.contains(". push ("));
        assert!(observed.contains(". output ()"));
        assert_eq!(observed.matches("for ").count(), plain.matches("for ").count());
        assert!(!observed.contains("rows_enumerated"));
        assert!(!observed.contains("collect"));
    }
}
