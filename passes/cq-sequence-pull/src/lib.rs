//! Sequential interpretation of checked Sequence terms.
//!
//! Views are iterator expressions; only `store` publishes a replayable Vec.
//! Physical adapters supply row functions, never a second program translation.
use std::collections::BTreeMap;

use cq_lang_sequence::{Analysis, Capability, Guard, Monoid, Program, Term};
use cq_lang_structures::physical::{Operator, symbol_name};
use cq_physical_realization::RowEmitter;
use cq_print::{Context, Repr, access::Access};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

/// A constructor's emitted iterator and its checked row-layout provenance.
#[derive(Clone)]
pub struct Value {
    pub rows: TokenStream,
    pub result: Option<String>,
    pub depth: usize,
    pub stored: bool,
}

impl Value {
    pub fn iterator(&self) -> TokenStream {
        self.rows.clone()
    }
}

/// Shared host adapters for target-specific structural folds.
pub struct Emitter<'a> {
    pub analysis: &'a Analysis,
    program: &'a Program,
    adapters: RowEmitter<'a>,
    values: BTreeMap<String, Value>,
    owner: String,
}

impl<'a> Emitter<'a> {
    pub fn new(program: &'a Program, analysis: &'a Analysis) -> Self {
        let mut context = Context::new(&program.structures, &analysis.structures);
        context.certify_host_sets(&program.structures);
        Self {
            analysis,
            program,
            adapters: RowEmitter::new(context),
            values: BTreeMap::new(),
            owner: String::new(),
        }
    }

    pub fn context(&self) -> &Context<'a> {
        self.adapters.context()
    }

    pub fn context_mut(&mut self) -> &mut Context<'a> {
        self.adapters.context_mut()
    }

    fn register(&mut self, name: &str) {
        if !self.context().idents.contains_key(name) {
            self.context_mut().register(name);
        }
    }

    fn rows(&mut self, name: &Ident) -> syn::Result<Value> {
        let result = symbol_name(name);
        let rows = if self.context().physical.contains_key(&result) {
            self.register(&result);
            if let Some(value) = self.values.get(&result).filter(|v| v.stored) {
                value.rows.clone()
            } else if matches!(self.context().physical[&result].operator, Operator::Unit(_)) {
                quote!(::core::iter::once(()))
            } else if self
                .analysis
                .structures
                .physical
                .hosts
                .contains_key(&result)
            {
                self.context().host_scan(&result)
            } else {
                self.adapters.scan(name)?
            }
        } else {
            let host = self.context().hosts.get(&result).ok_or_else(|| {
                syn::Error::new_spanned(name, "Sequence rows has no checked host")
            })?;
            let leaf = &host.value;
            quote!(::cq_storage::rows::Rows::rows(#leaf))
        };
        Ok(Value {
            rows,
            result: Some(result),
            depth: 0,
            stored: false,
        })
    }

    fn reference(&self, name: &Ident) -> syn::Result<Value> {
        let mut value = self
            .values
            .get(&symbol_name(name))
            .cloned()
            .ok_or_else(|| {
                syn::Error::new_spanned(name, "Sequence reference has no earlier definition")
            })?;
        value.stored = false;
        Ok(value)
    }

    fn map(&mut self, name: &Ident, mut input: Value) -> syn::Result<Value> {
        let result = symbol_name(name);
        if input.result.as_ref() == Some(&result) {
            input.stored = false;
            return Ok(input);
        }
        self.register(&result);
        let definition = self.context().physical[&result];
        if let Some(parent) = definition.operator.input() {
            if input.result.as_deref() != Some(&symbol_name(parent)) {
                input = self.map(parent, input)?;
            }
            self.context_mut()
                .reprs
                .insert(symbol_name(parent), Repr::Pipe(Some(input.rows)));
        }
        let rows = self.adapters.operator(&result)?;
        // Physical fan-out adapters already flatten their row iterator. Rewrap
        // it as one part so the explicit Sequence flatten remains structural.
        let nested = matches!(
            definition.operator,
            Operator::Probe(..) | Operator::Merge(..)
        );
        Ok(Value {
            rows: if nested {
                quote!(::core::iter::once(#rows))
            } else {
                rows
            },
            result: Some(result),
            depth: usize::from(nested),
            stored: false,
        })
    }

    fn compact(&mut self, name: &Ident, input: Value) -> syn::Result<Value> {
        self.map(name, input)
    }

    fn flatten(&self, mut input: Value) -> Value {
        let rows = input.rows;
        input.rows =
            quote!(::core::iter::Iterator::flat_map(#rows, ::core::iter::IntoIterator::into_iter));
        input.depth = input.depth.saturating_sub(1);
        input.stored = false;
        input
    }

    fn tile(&self, mut input: Value) -> Value {
        let rows = input.rows;
        input.rows = quote!(::core::iter::once(#rows));
        input.depth += 1;
        input.stored = false;
        input
    }

    fn union(&mut self, arms: Vec<Value>) -> syn::Result<Value> {
        let result = self
            .analysis
            .results
            .get(&self.owner)
            .ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "union lacks checked result provenance",
                )
            })?
            .clone();
        self.register(&result);
        let mut chain = None;
        for arm in arms {
            let input = arm.result.as_ref().ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "union arm lacks a row layout",
                )
            })?;
            let context = self.context();
            let pattern = context.bind_columns(input, context.carried(&result), false);
            let row = context.carrier(&result, |column| {
                let value = context.access(input, column);
                if context.infos[&result].computed.contains(column)
                    && !context.infos[input].computed.contains(column)
                {
                    quote!(::core::clone::Clone::clone(#value))
                } else {
                    value
                }
            });
            let rows = arm.rows;
            let rows = quote!(::core::iter::Iterator::map(#rows, move |#pattern| #row));
            chain = Some(match chain {
                None => rows,
                Some(previous) => quote!(::core::iter::Iterator::chain(#previous, #rows)),
            });
        }
        Ok(Value {
            rows: chain
                .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "empty union"))?,
            result: Some(result),
            depth: 0,
            stored: false,
        })
    }
    fn choose<'t>(&self, arms: &'t [(Guard, Term)]) -> syn::Result<&'t Term> {
        arms.iter()
            .find(|(guard, _)| *guard == Guard::Else)
            .or_else(|| arms.last())
            .map(|(_, term)| term)
            .ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "choose has no alternative")
            })
    }

    fn lookup(&mut self, provider: &Ident, keys: &[Ident], input: Value) -> syn::Result<Value> {
        let right = symbol_name(provider);
        let left = input
            .result
            .as_deref()
            .ok_or_else(|| syn::Error::new_spanned(provider, "lookup requires row provenance"))?;
        let mut columns: Vec<_> = keys.iter().map(symbol_name).collect();
        columns.sort();
        let capability = self
            .analysis
            .capabilities
            .get(&(right.clone(), columns.clone()))
            .ok_or_else(|| {
                syn::Error::new_spanned(provider, "lookup lacks a recorded capability")
            })?;
        self.register(&right);
        // Membership buckets are consumed by compact; that adapter uses the
        // exact membership provider and never enumerates payloads.
        if *capability == Capability::Membership {
            return Ok(input);
        }
        if *capability == Capability::Ordered {
            let result = self
                .analysis
                .results
                .get(&self.owner)
                .ok_or_else(|| {
                    syn::Error::new_spanned(provider, "ordered lookup lacks a result fact")
                })?
                .clone();
            return self.map(&Ident::new(&result, provider.span()), input);
        }
        let result = self
            .analysis
            .structures
            .physical
            .key_reads
            .get(&right)
            .into_iter()
            .flatten()
            .find_map(|(reader, read)| {
                let definition = self.context().physical.get(reader)?;
                match &definition.operator {
                    Operator::Probe(_, k) | Operator::Merge(_, k)
                        if read == &columns && symbol_name(&k.left) == left =>
                    {
                        Some(reader.clone())
                    }
                    _ => None,
                }
            })
            .ok_or_else(|| {
                syn::Error::new_spanned(provider, "lookup has no checked result layout")
            })?;
        self.register(&result);
        let context = self.context();
        let mut setup = TokenStream::new();
        let (lookup, payload) = match context.keyed(provider, &columns)? {
            Access::Host(access) if *capability == Capability::Range => {
                let fallback = &access.fallback;
                let key = context.host_key(left, &access);
                setup = quote!(let __cq_range_provider = #fallback.get(););
                (
                    quote!({
                        let (__cq_begin, __cq_end) = ::cq_storage::sequence::RangeCursor::range(__cq_range_provider, #key);
                        ::core::iter::Iterator::copied(::cq_storage::sequence::RangeCursor::items(__cq_range_provider)[__cq_begin..__cq_end].iter())
                    }),
                    access.payload,
                )
            }
            Access::Host(access) => (context.host_lookup(left, &access, false), access.payload),
            Access::Built(structure) => {
                let key = context
                    .stored_key(&right, &columns)
                    .iter()
                    .map(|c| context.key_field(left, c, context.owned_key(&right, &columns)))
                    .collect::<Vec<_>>();
                let lookup = match capability {
                    Capability::Range => quote!({
                        let (__cq_begin, __cq_end) = ::cq_storage::sequence::RangeCursor::range(#structure, &(#(#key,)*));
                        ::core::iter::Iterator::copied(::cq_storage::sequence::RangeCursor::items(#structure)[__cq_begin..__cq_end].iter())
                    }),
                    _ => context.chained_lookup(left, &structure, &right, &columns),
                };
                let lookup = if *capability == Capability::Range {
                    if let Some(condition) = context.constant_key_condition(left, &right, &columns)
                    {
                        quote!(::core::iter::Iterator::flatten(::core::iter::IntoIterator::into_iter((#condition).then(|| #lookup))))
                    } else {
                        lookup
                    }
                } else {
                    lookup
                };
                (
                    lookup,
                    context.built_payload(&right, &context.levels_of(&right, &structure)),
                )
            }
            Access::Prefix(..) => {
                return Err(syn::Error::new_spanned(
                    provider,
                    "Sequence lookup needs a complete recorded payload provider",
                ));
            }
        };
        let pattern = context.pattern(left);
        let row = context.carrier(&result, |column| {
            if let Some(position) = payload
                .iter()
                .position(|p| p == column && !columns.iter().any(|k| k == column))
            {
                let index = syn::Index::from(position);
                quote!(__cq_payload.#index)
            } else {
                context
                    .constant_value(&right, column)
                    .filter(|_| !columns.iter().any(|key| key == column))
                    .unwrap_or_else(|| context.access(left, column))
            }
        });
        let rows = input.rows;
        Ok(Value {
            rows: quote!({
                #setup
                ::core::iter::Iterator::map(#rows, move |#pattern| {
                    ::core::iter::Iterator::map(#lookup, move |__cq_payload| #row)
                })
            }),
            result: Some(result),
            depth: 1,
            stored: false,
        })
    }

    fn fold(&self, monoid: &Monoid, input: Value) -> syn::Result<Value> {
        let rows = input.rows;
        let value = match monoid {
            Monoid::Count => quote!(::core::iter::Iterator::fold(#rows, 0usize, |n, _| n + 1)),
            Monoid::Or => quote!(::core::iter::Iterator::fold(#rows, false, |_, _| true)),
            Monoid::Sum(column) => {
                let result = input
                    .result
                    .as_ref()
                    .ok_or_else(|| syn::Error::new_spanned(column, "sum requires a row layout"))?;
                let pattern = self.context().pattern(result);
                let value = self.context().access(result, &symbol_name(column));
                quote!(::core::iter::Iterator::fold(#rows, 0, |sum, #pattern| sum + #value))
            }
        };
        Ok(Value {
            rows: quote!(::core::iter::once(#value)),
            result: None,
            depth: 0,
            stored: false,
        })
    }

    /// Apply the recorded canonical provider to flat rows, preserving first occurrences.
    pub fn canon(&mut self, mut input: Value) -> syn::Result<Value> {
        let ty = input
            .result
            .as_ref()
            .and_then(|name| self.analysis.structures.dedup.get(name))
            .or_else(|| {
                self.analysis
                    .obligations
                    .iter()
                    .find_map(|obligation| match obligation {
                        cq_lang_sequence::Obligation::Canon { result, ty }
                            if result == &self.owner =>
                        {
                            ty.as_ref()
                        }
                        _ => None,
                    })
            })
            .ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "Sequence canon requires a recorded provider",
                )
            })?;
        let width = input
            .result
            .as_ref()
            .map_or(1, |result| self.context().carried(result).len());
        let key = if width > 12 {
            let fields = (0..width)
                .map(|position| {
                    let position = syn::Index::from(position);
                    quote!(::core::clone::Clone::clone(&__cq_row.#position))
                })
                .collect::<Vec<_>>();
            let grouped = cq_print::group(&fields);
            quote!(&#grouped)
        } else {
            quote!(__cq_row)
        };
        let rows = input.rows;
        input.rows = quote!({
            let mut __cq_seen = <#ty as ::cq_storage::Build<_>>::build(::core::iter::empty::<(_, ())>());
            ::core::iter::Iterator::filter(#rows, move |__cq_row| {
                ::cq_storage::require::leibniz(#key);
                __cq_seen.insert(::core::clone::Clone::clone(#key))
            })
        });
        input.stored = false;
        Ok(input)
    }

    fn store(&self, mut input: Value) -> Value {
        let rows = input.rows;
        input.rows = quote!(::core::iter::IntoIterator::into_iter(::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#rows)));
        input.stored = true;
        input
    }

    fn publish(&mut self, name: &Ident, mut value: Value) -> syn::Result<Value> {
        if value.stored {
            let id = format_ident!("__cq_sequence_store_{}", name);
            let rows = &value.rows;
            self.context_mut().statements.push(syn::parse_quote!(let #id = ::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#rows);));
            value.rows = quote!(::core::iter::Iterator::cloned(#id.iter()));
            if let Some(result) = &value.result
                && value.depth == 0
            {
                let builds = self
                    .analysis
                    .structures
                    .built
                    .get(result)
                    .cloned()
                    .unwrap_or_default();
                let mut structures = Vec::new();
                for (levels, ty) in builds {
                    let context = self.context();
                    let payload = context.built_payload(result, &levels);
                    let fields = payload.iter().map(|c| context.access(result, c));
                    let mut pair = quote!((#(#fields,)*));
                    for level in levels.iter().rev() {
                        let key = context
                            .stored_key(result, level)
                            .iter()
                            .map(|c| context.key_field(result, c, context.owned_key(result, level)))
                            .collect::<Vec<_>>();
                        pair = quote!(((#(#key,)*), #pair));
                    }
                    let pattern = context.pattern(result);
                    let index = format_ident!("__cq_sequence_index_{}_{}", name, structures.len());
                    let source = &value.rows;
                    let pairs = quote!(::core::iter::Iterator::map(#source, |#pattern| #pair));
                    let bare = matches!(&ty, syn::Type::Path(p) if p.qself.is_none() && p.path.segments.iter().all(|s| s.arguments.is_none()));
                    let build = if bare {
                        quote!({ use ::cq_storage::Build as _; #ty::build(#pairs) })
                    } else {
                        quote!(<#ty as ::cq_storage::Build<_>>::build(#pairs))
                    };
                    self.context_mut()
                        .statements
                        .push(syn::parse_quote!(let #index = &#build;));
                    structures.push((levels, index));
                }
                self.context_mut().reprs.insert(
                    result.clone(),
                    Repr::Let {
                        rows: Some(id),
                        structures,
                    },
                );
            }
        } else if let Some(result) = &value.result {
            self.context_mut()
                .reprs
                .insert(result.clone(), Repr::Pipe(Some(value.rows.clone())));
        }
        self.values.insert(symbol_name(name), value.clone());
        Ok(value)
    }

    pub fn finish(&mut self, value: Value) -> syn::Result<syn::Expr> {
        let mut rows = value.rows;
        for _ in 0..value.depth {
            rows = quote!(::core::iter::Iterator::flat_map(#rows, ::core::iter::IntoIterator::into_iter));
        }
        if let Some(result) = value.result {
            let head = &self.program.structures.physical.program.head;
            rows = self.adapters.head(&result, head, rows);
        }
        let context = self.context();
        let statements = &context.statements;
        let mut output = syn::parse2(
            quote!({ #(#statements)* ::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#rows) }),
        )?;
        cq_print::own_captures(&mut output, &context.owned);
        Ok(output)
    }
}

/// Emit a host-only checked Sequence program. Analysis belongs to this program.
pub fn translate(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    if analysis
        .levels
        .values()
        .any(|path| path.as_slice() != ["host"])
    {
        return Err(syn::Error::new_spanned(
            program,
            "sequential translation requires host-only grading",
        ));
    }
    translate_with(program, analysis, |_, _, _, _, _, _| Ok(()))
}

/// Structural fold with a target hook after each constructor. `input` is its
/// original child, `owner` its definition and `tiles` its enclosing tile depth.
/// The hook may replace emitted iterator tokens without inspecting the program.
pub fn translate_with(
    program: &Program,
    analysis: &Analysis,
    mut hook: impl FnMut(
        &Term,
        &mut Emitter<'_>,
        Option<&Value>,
        &mut Value,
        &str,
        usize,
    ) -> syn::Result<()>,
) -> syn::Result<syn::Expr> {
    fn term(
        emitter: &mut Emitter<'_>,
        node: &Term,
        owner: &str,
        tiles: usize,
        hook: &mut impl FnMut(
            &Term,
            &mut Emitter<'_>,
            Option<&Value>,
            &mut Value,
            &str,
            usize,
        ) -> syn::Result<()>,
    ) -> syn::Result<Value> {
        let child = match node {
            Term::Map(_, inner)
            | Term::Compact(_, inner)
            | Term::Flatten(inner)
            | Term::Fold(_, inner)
            | Term::Canon(inner)
            | Term::Store { inner, .. }
            | Term::Lookup { inner, .. } => Some(term(emitter, inner, owner, tiles, hook)?),
            Term::Tile { inner, .. } => Some(term(emitter, inner, owner, tiles + 1, hook)?),
            _ => None,
        };
        let input = || child.clone().expect("unary constructor");
        let mut value = match node {
            Term::Rows(name) => emitter.rows(name)?,
            Term::Ref(name) => emitter.reference(name)?,
            Term::Map(name, _) => emitter.map(name, input())?,
            Term::Compact(name, _) => emitter.compact(name, input())?,
            Term::Flatten(_) => emitter.flatten(input()),
            Term::Tile { .. } => emitter.tile(input()),
            Term::Lookup { provider, keys, .. } => emitter.lookup(provider, keys, input())?,
            Term::Fold(monoid, _) => emitter.fold(monoid, input())?,
            Term::Canon(_) => emitter.canon(input())?,
            Term::Store { .. } => emitter.store(input()),
            Term::Union(arms) => {
                let values = arms
                    .iter()
                    .map(|arm| term(emitter, arm, owner, tiles, hook))
                    .collect::<syn::Result<Vec<_>>>()?;
                emitter.union(values)?
            }
            Term::Choose(arms) => {
                let alternative = emitter.choose(arms)?;
                term(emitter, alternative, owner, tiles, hook)?
            }
        };
        hook(node, emitter, child.as_ref(), &mut value, owner, tiles)?;
        Ok(value)
    }
    // Formal rule T-Pull: Interpret Sequence by the host iterator algebra.
    let mut emitter = Emitter::new(program, analysis);
    let mut output = None;
    for (position, definition) in program.sequence.definitions.iter().enumerate() {
        let owner = symbol_name(&definition.name);
        emitter.owner = owner.clone();
        let value = term(&mut emitter, &definition.term, &owner, 0, &mut hook)?;
        output = Some(if position + 1 == program.sequence.definitions.len() {
            value
        } else {
            emitter.publish(&definition.name, value)?
        });
    }
    emitter.finish(
        output
            .ok_or_else(|| syn::Error::new_spanned(program, "Sequence has no output definition"))?,
    )
}
