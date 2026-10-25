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

/// The native iterator target executes entirely at the host level.
pub fn machine() -> cq_lang_sequence::Machine {
    cq_lang_sequence::Machine {
        layout: None,
        name: syn::parse_quote!(sequential),
        items: 1,
        key_partition_threshold: 65536,
        root: cq_lang_sequence::LevelNode {
            shared_bytes: None,
            name: syn::parse_quote!(host),
            collaborative: false,
            dynamic: false,
            width: None,
            children: vec![],
        },
    }
}

/// Native provider, value-refinement and canonicalization tokens.
///
/// Generic holes describe families of authored types, not a replacement for
/// their arguments. Rust trait checking retains each token's row-type bounds.
pub fn catalog() -> cq_lang_sequence::Catalog {
    use Capability::{Exact, Membership, Ordered, Range};
    use cq_lang_sequence::{CanonToken, Catalog, CodecToken, ProviderToken};

    let levels = vec![vec!["host".into()]];
    let providers = [
        (
            syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>),
            vec![Exact, Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::Trie<'_, _, _>),
            vec![Exact, Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::SortedRun<_, _>),
            vec![Exact, Range, Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::SortedRun<_, _, _>),
            vec![Exact, Range, Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::HashSet<_>),
            vec![Membership],
        ),
        (
            syn::parse_quote!(::std::collections::HashSet<_>),
            vec![Membership],
        ),
        (
            syn::parse_quote!(::std::collections::HashSet<_, _>),
            vec![Membership],
        ),
        (
            syn::parse_quote!(::std::collections::BTreeSet<_>),
            vec![Membership, Ordered],
        ),
        (
            syn::parse_quote!(::std::collections::HashMap<_, ::std::vec::Vec<_>>),
            vec![Exact, Membership],
        ),
        (
            syn::parse_quote!(::std::collections::HashMap<_, ::std::vec::Vec<_>, _>),
            vec![Exact, Membership],
        ),
        (
            syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>),
            vec![Exact, Membership, Ordered],
        ),
        (
            syn::parse_quote!(::cq_storage::GroupedAnswers<_, _>),
            vec![Exact, Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::SingleGroup<_, _>),
            vec![Exact, Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::dispatch::KeySet<_>),
            vec![Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::dispatch::KeySet<_, false>),
            vec![Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::dispatch::KeySet<_, true>),
            vec![Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::AdaptiveKeySet<_, _, _>),
            vec![Membership],
        ),
    ]
    .into_iter()
    .map(|(ty, capabilities)| ProviderToken {
        ty,
        capabilities,
        levels: levels.clone(),
        elements: None,
    })
    .collect();
    Catalog {
        providers,
        codecs: [
            syn::parse_quote!(::cq_storage::sequence::Identity),
            syn::parse_quote!(::cq_storage::sequence::Dense<_>),
        ]
        .into_iter()
        .map(|ty| CodecToken {
            ty,
            ordered: true,
            levels: levels.clone(),
        })
        .collect(),
        canons: [
            syn::parse_quote!(::cq_storage::HashSet<_>),
            syn::parse_quote!(::std::collections::HashSet<_>),
            syn::parse_quote!(::std::collections::HashSet<_, _>),
            syn::parse_quote!(::std::collections::BTreeSet<_>),
        ]
        .into_iter()
        .map(|ty| CanonToken {
            ty,
            levels: levels.clone(),
            max_packed_bits: None,
        })
        .collect(),
    }
}

/// A constructor's emitted iterator and its checked row-layout provenance.
#[derive(Clone)]
pub struct Value {
    pub rows: TokenStream,
    pub result: Option<String>,
    pub depth: usize,
    pub stored: bool,
    /// A target restricts this iterator to one partition; adapters must not
    /// replace it with a scan of the complete host representation.
    pub partitioned: bool,
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
    target_published: std::collections::BTreeSet<String>,
    lengths: BTreeMap<String, TokenStream>,
    owner: String,
    pull: bool,
    owned_answer: bool,
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
            target_published: std::collections::BTreeSet::new(),
            lengths: BTreeMap::new(),
            owner: String::new(),
            pull: false,
            owned_answer: false,
        }
    }

    pub fn context(&self) -> &Context<'a> {
        self.adapters.context()
    }

    pub fn context_mut(&mut self) -> &mut Context<'a> {
        self.adapters.context_mut()
    }

    /// A target hook has already emitted this store's binding and providers.
    pub fn mark_target_published(&mut self, owner: &str) {
        self.target_published.insert(owner.to_owned());
    }

    /// Publish the exact count of an already evaluated Sequence store.
    pub fn publish_length(&mut self, name: &str, length: TokenStream) {
        self.lengths.insert(name.to_owned(), length);
    }

    /// Re-publish an immutable flat row view already owned by an ancestor.
    pub fn alias_store(&mut self, name: &str, source: &str) -> bool {
        if !self.values.get(source).is_some_and(|value| value.stored && value.depth == 0) {
            return false;
        }
        let Some(length) = self.lengths.get(source).cloned() else { return false };
        self.lengths.insert(name.to_owned(), length);
        self.mark_target_published(name);
        true
    }

    fn runtime_length(&self, length: &cq_lang_sequence::Len) -> syn::Result<TokenStream> {
        use cq_lang_sequence::Len;
        match length {
            Len::Lit(value) => Ok(quote!(#value)),
            Len::Add(left, right) => {
                let left = self.runtime_length(left)?;
                let right = self.runtime_length(right)?;
                Ok(quote!((#left).saturating_add(#right)))
            }
            Len::Input(name) => {
                if let Some(length) = self.lengths.get(name) {
                    return Ok(length.clone());
                }
                if let Some(value) = self.values.get(name).filter(|value| value.stored && value.depth == 0) {
                    let rows = &value.rows;
                    return Ok(quote!((#rows).count()));
                }
                if let Some(host) = self.context().hosts.get(name) {
                    let host = &host.value;
                    return Ok(quote!({
                        let __cq_input_rows = ::cq_storage::rows::Rows::rows(#host);
                        let (__cq_lower, __cq_upper) = __cq_input_rows.size_hint();
                        if __cq_upper == ::core::option::Option::Some(__cq_lower) {
                            __cq_lower
                        } else {
                            __cq_input_rows.count()
                        }
                    }));
                }
                Err(syn::Error::new(proc_macro2::Span::call_site(),
                    format!("runtime choice length `{name}` has no published flat store")))
            }
            _ => Err(syn::Error::new(proc_macro2::Span::call_site(),
                "runtime choice requires a recorded input length, literal, or sum")),
        }
    }

    fn runtime_guard(&self, guard: &Guard) -> syn::Result<TokenStream> {
        match guard {
            Guard::Lt(length, threshold) => {
                let length = self.runtime_length(length)?;
                Ok(quote!(#length < #threshold))
            }
            Guard::Ge(length, threshold) => {
                let length = self.runtime_length(length)?;
                Ok(quote!(#length >= #threshold))
            }
            Guard::Else => Ok(quote!(true)),
        }
    }

    /// A target's final task buffers already contain the owned answer head.
    /// Keep their joined iterator instead of copying into another host buffer.
    pub fn own_task_answer(&mut self, result: &str, rows: TokenStream) -> TokenStream {
        self.owned_answer = true;
        self.adapters.head(result, &self.program.structures.physical.program.head, rows)
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
                self.context_mut().reprs.insert(result.clone(), Repr::Host);
                let rows = self.context().host_scan(&result);
                self.context().profile_stream(&result, rows)
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
            partitioned: false,
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

    /// Lift an innermost constructor through `levels` outer sequence partitions.
    pub fn lift(
        &mut self,
        mut input: Value,
        levels: usize,
        operation: &mut impl FnMut(&mut Self, Value) -> syn::Result<Value>,
    ) -> syn::Result<Value> {
        if levels == 0 {
            return operation(self, input);
        }
        let source = input.rows.clone();
        let part = format_ident!("__cq_sequence_part_{}", levels);
        input.rows = quote!(::core::iter::IntoIterator::into_iter(#part));
        input.depth -= 1;
        let mut output = self.lift(input, levels - 1, operation)?;
        let rows = output.rows;
        output.rows = quote!(::core::iter::Iterator::map(#source, move |#part| #rows));
        output.depth += 1;
        Ok(output)
    }

    fn map(&mut self, name: &Ident, mut input: Value, expand: bool) -> syn::Result<Value> {
        let partition = input.partitioned;
        let result = symbol_name(name);
        if input.result.as_ref() == Some(&result) {
            input.stored = false;
            return Ok(input);
        }
        self.register(&result);
        if let Some(rows) = self.adapters.intersection(&result)? {
            let nested = expand
                && matches!(
                    self.context().physical[&result].operator,
                    Operator::Probe(..)
                );
            return Ok(Value {
                rows: if nested {
                    quote!(::core::iter::once(#rows))
                } else {
                    rows
                },
                result: Some(result),
                depth: usize::from(nested),
                stored: false,
                partitioned: partition,
            });
        }
        if self.analysis.structures.selections.contains_key(&result) {
            let selection = self.context_mut().selection_source(&result)?;
            let repr = self.context_mut().materialize(&result, selection);
            self.context_mut().reprs.insert(result.clone(), repr);
            return Ok(Value {
                rows: self.adapters.scan(name)?,
                result: Some(result),
                depth: 0,
                stored: false,
                partitioned: partition,
            });
        }
        if !partition && let Some(ids) = self.adapters.positions(&result)? {
            return Ok(Value {
                rows: self.context().positional_rows(&result, ids),
                result: Some(result),
                depth: 0,
                stored: false,
                partitioned: false,
            });
        }
        let definition = self.context().physical[&result];
        let operand = if let Some(parent) = definition.operator.input() {
            let parent_name = symbol_name(parent);
            if input.result.as_deref() != Some(&parent_name) {
                input = self.map(parent, input, expand)?;
            }
            let positional = !partition && self.analysis.structures.reads.contains_key(&result)
                && matches!(
                    self.context().reprs.get(&parent_name),
                    Some(Repr::Host | Repr::Positions(_) | Repr::Selection { .. })
                );
            let ordered = matches!(definition.operator, Operator::Merge(..))
                && matches!(
                    self.context().reprs.get(&parent_name),
                    Some(Repr::Let { .. } | Repr::Selection { .. })
                );
            if positional || ordered {
                // Positional and ordered reads consume the recorded source
                // representation rather than an unstructured row iterator.
                None
            } else {
                Some((parent_name, input.rows))
            }
        } else {
            None
        };
        // The input stream is separate from its retained keyed representation:
        // an operator may scan and probe the same stored result.
        let rows = if partition {
            self.adapters.operator_in_partition(&result, operand)?
        } else {
            self.adapters.operator(&result, operand)?
        };
        let rows = self.context().profile_stream(&result, rows);
        // Physical fan-out adapters already flatten their row iterator. Rewrap
        // only an expanding constructor: Compact consumes membership directly,
        // including a Probe whose recorded provider is a membership projection.
        let nested = expand
            && matches!(
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
            partitioned: partition,
        })
    }

    fn compact(&mut self, name: &Ident, input: Value) -> syn::Result<Value> {
        self.map(name, input, false)
    }

    fn flatten(&self, mut input: Value) -> Value {
        let rows = input.rows;
        input.rows =
            quote!(::core::iter::Iterator::flat_map(#rows, ::core::iter::IntoIterator::into_iter));
        input.depth = input.depth.saturating_sub(1);
        input.stored = false;
        input
    }

    fn tile(&self, n: usize, mut input: Value) -> Value {
        let rows = input.rows;
        input.rows = quote!({
            let mut __cq_tile_rows = #rows;
            ::core::iter::from_fn(move || {
                let __cq_part = ::core::iter::Iterator::collect::<::std::vec::Vec<_>>(
                    ::core::iter::Iterator::take(
                        ::core::iter::Iterator::by_ref(&mut __cq_tile_rows), #n));
                if __cq_part.is_empty() { None } else { Some(__cq_part.into_iter()) }
            })
        });
        input.depth += 1;
        input.stored = false;
        input
    }
    /// Serial interpretation of the same fixed key routing used by task targets.
    pub fn partition_k(&mut self, keys: &[Ident], parts: usize, mut input: Value) -> syn::Result<Value> {
        let result = input.result.as_ref().ok_or_else(|| syn::Error::new(
            proc_macro2::Span::call_site(), "partition_k requires row provenance"))?;
        self.register(result);
        let columns: Vec<_> = keys.iter().map(symbol_name).collect();
        let pattern = self.context().bind_columns(result, &columns, false);
        let key: Vec<_> = columns.iter().map(|column| self.context().access(result, column)).collect();
        let key = cq_print::group(&key);
        let rows = input.rows;
        input.rows = quote!({
            let mut __cq_parts: ::std::vec::Vec<::std::vec::Vec<_>> =
                (0..#parts).map(|_| ::std::vec::Vec::new()).collect();
            for __cq_row in #rows {
                let #pattern = &__cq_row;
                let __cq_part = ::cq_storage::sharded::route_index(&#key, #parts);
                __cq_parts[__cq_part].push(__cq_row);
            }
            __cq_parts.into_iter()
        });
        input.depth += 1;
        input.stored = false;
        Ok(input)
    }


    // Row-layout conversion is shared by union injection and checked
    // equivalent-result aliases. It changes carriers, never query decisions.
    fn rehead(&mut self, result: &str, mut input: Value) -> syn::Result<Value> {
        let source = input.result.as_ref().ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "row alias lacks checked provenance",
            )
        })?;
        if source == result {
            return Ok(input);
        }
        self.register(result);
        let context = self.context();
        let pattern = context.bind_columns(source, context.carried(result), false);
        let row = context.carrier(result, |column| {
            let value = context.access(source, column);
            if context.infos[result].computed.contains(column)
                && !context.infos[source].computed.contains(column)
            {
                quote!(::core::clone::Clone::clone(#value))
            } else {
                value
            }
        });
        let rows = input.rows;
        input.rows = quote!(::core::iter::Iterator::map(#rows, move |#pattern| #row));
        input.result = Some(result.to_owned());
        Ok(input)
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
        if self
            .analysis
            .structures
            .physical
            .fused
            .contains_key(&result)
        {
            let rows = self.adapters.operator(&result, None)?;
            return Ok(Value {
                rows: self.context().profile_stream(&result, rows),
                result: Some(result),
                depth: 0,
                stored: false,
                partitioned: false,
            });
        }
        let mut chain = None;
        for arm in arms {
            let rows = self.rehead(&result, arm)?.rows;
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
            partitioned: false,
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

    fn lookup(
        &mut self,
        provider: &Ident,
        keys: &[Ident],
        input: Value,
        membership: bool,
    ) -> syn::Result<Value> {
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
        if membership || *capability == Capability::Membership {
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
            return self.map(&Ident::new(&result, provider.span()), input, true);
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
        if let Some(rows) = self.adapters.intersection(&result)? {
            return Ok(Value {
                rows: quote!(::core::iter::once(#rows)),
                result: Some(result),
                depth: 1,
                stored: false,
                partitioned: input.partitioned,
            });
        }
        if self
            .context()
            .forcing_witness(&result, &right, &columns)
            .is_some()
            || self.analysis.structures.stages.contains_key(&result)
        {
            return self.map(&Ident::new(&result, provider.span()), input, true);
        }
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
            Access::Host(access) => (
                context.host_lookup(left, &access, false, false),
                access.payload,
            ),
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
                return self.map(&Ident::new(&result, provider.span()), input, true);
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
        let rows = context.profile_input(&result, input.rows);
        let output = quote!(::core::iter::Iterator::map(#lookup, move |__cq_payload| #row));
        let output = context.profile_stream(&result, output);
        Ok(Value {
            rows: quote!({
                #setup
                ::core::iter::Iterator::map(#rows, move |#pattern| {
                    #output
                })
            }),
            result: Some(result),
            depth: 1,
            stored: false,
            partitioned: input.partitioned,
        })
    }

    pub fn group(&mut self, name: &Ident, input: Value) -> syn::Result<Value> {
        self.group_with_output(name, input, false)
    }

    /// A task-local global aggregate produces partials, not final Group rows.
    pub fn group_partial(&mut self, name: &Ident, input: Value) -> syn::Result<Value> {
        self.group_with_output(name, input, true)
    }

    fn group_with_output(&mut self, name: &Ident, input: Value, partial: bool) -> syn::Result<Value> {
        let result = symbol_name(name);
        self.register(&result);
        let Operator::Group(group) = &self.context().physical[&result].operator else {
            return Err(syn::Error::new_spanned(
                name,
                "group lacks Physical group provenance",
            ));
        };
        let operand = symbol_name(&group.input);
        let output = if partial {
            let label = format!("{result}.partial");
            self.context_mut().profile_label(&label, "GROUP_PARTIAL", "", vec![operand.clone()]);
            label
        } else {
            result.clone()
        };
        let rows = self
            .adapters
            .operator(&result, Some((operand, input.rows)))?;
        let rows = self.context().profile_stream(&output, rows);
        Ok(Value {
            rows,
            result: Some(result),
            depth: 0,
            stored: false,
            partitioned: input.partitioned,
        })
    }

    pub fn fold(&self, monoid: &Monoid, input: Value) -> syn::Result<Value> {
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
            partitioned: input.partitioned,
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
        let label = format!("{}.dedup", self.owner);
        let inputs = input.result.iter().cloned().collect();
        self.context_mut()
            .profile_label(&label, "DEDUP", "", inputs);
        let rows = self.context().profile_inserting(&label, input.rows);
        let rows = RowEmitter::canon(ty, rows, width);
        input.rows = self.context().profile_stream(&label, rows);
        input.stored = false;
        Ok(input)
    }

    fn store(&self, mut input: Value, nested: bool) -> Value {
        let physical_store = input.result.as_ref().is_some_and(|result| {
            input.depth == 0
                && self.analysis.structures.physical.modes[result]
                    != cq_lang_structures::physical::ModeKind::Pipe
        });
        if nested && !physical_store {
            let rows = input.rows;
            input.rows = quote!(::core::iter::IntoIterator::into_iter(::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#rows)));
        }
        input.stored = true;
        input
    }

    fn publish(&mut self, name: &Ident, mut value: Value) -> syn::Result<Value> {
        if let Some(result) = self.analysis.results.get(&symbol_name(name)).cloned()
            && value.result.as_ref() != Some(&result)
        {
            let depth = value.depth;
            value = self.lift(value, depth, &mut |emitter, rows| {
                emitter.rehead(&result, rows)
            })?;
        }
        if self.target_published.remove(&symbol_name(name)) {
            self.values.insert(symbol_name(name), value.clone());
            return Ok(value);
        }
        if value.stored {
            let physical_store = value
                .result
                .as_ref()
                .filter(|result| {
                    value.depth == 0
                        && self.analysis.structures.physical.modes[*result]
                            != cq_lang_structures::physical::ModeKind::Pipe
                })
                .cloned();
            if let Some(result) = physical_store {
                if !matches!(
                    self.context().reprs.get(&result),
                    Some(Repr::Selection { .. })
                ) {
                    let repr = self.context_mut().materialize(&result, value.rows.clone());
                    self.context_mut().reprs.insert(result.clone(), repr);
                }
                if matches!(
                    self.context().reprs.get(&result),
                    Some(Repr::Selection { .. } | Repr::Let { rows: Some(_), .. })
                ) {
                    value.rows = self.adapters.scan(&Ident::new(&result, name.span()))?;
                }
            } else {
                self.context_mut().bound = true;
                let id = format_ident!("__cq_sequence_store_{}", name);
                let label = format!("{name}.rows");
                self.context_mut().profile_label(
                    &label,
                    "ROWS_BUILD",
                    "",
                    value.result.iter().cloned().collect(),
                );
                let rows = self.context().profile_inserting(&label, value.rows);
                let build = self.context().profile_build(
                    &label,
                    quote!(::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#rows)),
                );
                self.context_mut()
                    .statements
                    .push(syn::parse_quote!(let #id = #build;));
                // Sequence-owned stores need the same independent primitive and
                // capacity accounting as Physical-owned row providers.
                let allocation = format_ident!("__cq_sequence_store_{}_allocation", name);
                self.context_mut().statements.push(syn::parse_quote!(
                    let #allocation = ::cq_storage::retain_rows(&#id);
                ));
                value.rows = quote!(::core::iter::Iterator::cloned(#id.iter()));
                if let Some(result) = &value.result
                    && value.depth == 0
                {
                    let mut repr = self.context_mut().materialize(result, value.rows.clone());
                    if let Repr::Let { rows, .. } = &mut repr {
                        *rows = Some(id);
                    }
                    self.context_mut().reprs.insert(result.clone(), repr);
                }
            }
        } else if let Some(result) = &value.result
            && !matches!(
                self.context().reprs.get(result),
                Some(Repr::Host | Repr::Positions(_) | Repr::Selection { .. })
            )
        {
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
        let answer = format!(
            "{}.answer",
            self.program.structures.physical.program.returned
        );
        self.context_mut().profile_label(
            &answer,
            "ANSWER",
            "",
            value.result.iter().cloned().collect(),
        );
        rows = self.context().profile_input(&answer, rows);
        if !self.owned_answer {
            if let Some(result) = value.result {
                let head = &self.program.structures.physical.program.head;
                rows = self.adapters.head(&result, head, rows);
            }
        }
        rows = self.context().profile_stream(&answer, rows);
        // Target interpreters eagerly join their answer. Measure preparation
        // and final evaluation at the host, never by adding concurrent spans.
        let phases = if !self.pull && self.context().profile.is_some() {
            let build = self.context_mut().profile_label(
                "__cq_tasks_build", "TASKS_BUILD_WALL", "", Vec::new(),
            );
            let query = self.context_mut().profile_label(
                "__cq_tasks_query", "TASKS_QUERY_WALL", "", vec![answer],
            );
            Some((build, query))
        } else {
            None
        };
        let registration = self.context().profile_registration();
        let context = self.context();
        let statements = &context.statements;
        let rows = if self.owned_answer {
            rows
        } else if self.pull {
            if context.bound || value.stored {
                quote!(::core::iter::IntoIterator::into_iter(
                    ::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#rows)))
            } else {
                rows
            }
        } else {
            quote!(::core::iter::Iterator::collect::<::std::vec::Vec<_>>(#rows))
        };
        let mut output = if let Some((build, query)) = phases {
            let profile = context.profile_handle();
            syn::parse2(quote!({
                #registration
                let __cq_build_clock = #profile.start_scope(#build);
                #(#statements)*
                ::core::mem::drop(__cq_build_clock);
                #profile.scope(#query, || #rows)
            }))?
        } else {
            syn::parse2(quote!({ #registration #(#statements)* #rows }))?
        };
        cq_print::own_captures(&mut output, &context.owned);
        Ok(output)
    }
}

/// Emit a host-only checked Sequence iterator. Analysis belongs to this program.
pub fn translate(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    translate_host(program, analysis, false)
}

/// Emit the host interpretation with operator, build, and answer counters.
pub fn translate_profile(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    translate_host(program, analysis, true)
}

fn translate_host(program: &Program, analysis: &Analysis, profile: bool) -> syn::Result<syn::Expr> {
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
    translate_with_profile(program, analysis, profile, true, |_, _, _, _, _, _| Ok(()))
}

/// Structural fold with a target hook after each constructor. `input` is its
/// original child, `owner` its definition and `tiles` its enclosing tile depth.
/// The hook may replace emitted iterator tokens without inspecting the program.
pub fn translate_with(
    program: &Program,
    analysis: &Analysis,
    hook: impl FnMut(
        &Term,
        &mut Emitter<'_>,
        Option<&Value>,
        &mut Value,
        &str,
        usize,
    ) -> syn::Result<()>,
) -> syn::Result<syn::Expr> {
    translate_with_profile(program, analysis, false, false, hook)
}

/// Target-hook interpretation with real operator counters and disjoint host
/// preparation/final-evaluation wall scopes. Unlike operator spans, these
/// phases include each concurrent segment only once, including its join.
pub fn translate_profile_with(
    program: &Program,
    analysis: &Analysis,
    hook: impl FnMut(
        &Term,
        &mut Emitter<'_>,
        Option<&Value>,
        &mut Value,
        &str,
        usize,
    ) -> syn::Result<()>,
) -> syn::Result<syn::Expr> {
    translate_with_profile(program, analysis, true, false, hook)
}

fn translate_with_profile(
    program: &Program,
    analysis: &Analysis,
    profile: bool,
    pull: bool,
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
        membership: bool,
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
            Term::Compact(_, inner) => Some(term(
                emitter,
                inner,
                owner,
                tiles,
                matches!(inner.as_ref(), Term::Lookup { .. }),
                hook,
            )?),
            Term::Map(_, inner)
            | Term::Flatten(inner)
            | Term::Fold(_, inner)
            | Term::Group(_, inner)
            | Term::Canon(inner)
            | Term::Store { inner, .. }
            | Term::PartitionK { inner, .. }
            | Term::Lookup { inner, .. } => Some(term(emitter, inner, owner, tiles, false, hook)?),
            Term::Tile { inner, .. } => Some(term(emitter, inner, owner, tiles + 1, false, hook)?),
            _ => None,
        };
        let input = || child.clone().expect("unary constructor");
        let mut value = match node {
            Term::Rows(name) => emitter.rows(name)?,
            Term::Ref(name) => emitter.reference(name)?,
            Term::Map(name, _) => emitter.lift(input(), input().depth, &mut |emitter, rows| {
                emitter.map(name, rows, true)
            })?,
            Term::Compact(name, _) => {
                emitter.lift(input(), input().depth, &mut |emitter, rows| {
                    emitter.compact(name, rows)
                })?
            }
            Term::Flatten(_) => emitter.lift(
                input(),
                input().depth.saturating_sub(1),
                &mut |emitter, rows| Ok(emitter.flatten(rows)),
            )?,
            Term::Tile { n, .. } => {
                emitter.lift(input(), input().depth, &mut |emitter, rows| {
                    Ok(emitter.tile(*n, rows))
                })?
            }
            Term::PartitionK { keys, parts, .. } => emitter.partition_k(keys, *parts, input())?,
            Term::Lookup { provider, keys, .. } => {
                emitter.lift(input(), input().depth, &mut |emitter, rows| {
                    emitter.lookup(provider, keys, rows, membership)
                })?
            }
            Term::Group(name, _) => {
                let global = matches!(&emitter.context().physical[&symbol_name(name)].operator,
                    Operator::Group(group) if group.keys.is_empty());
                if global && input().depth != 0 {
                    let flat = emitter.flatten(input());
                    emitter.group(name, flat)?
                } else {
                    emitter.lift(input(), input().depth, &mut |emitter, rows| emitter.group(name, rows))?
                }
            }
            Term::Fold(monoid, _) => {
                emitter.lift(input(), input().depth, &mut |emitter, rows| {
                    emitter.fold(monoid, rows)
                })?
            }
            Term::Canon(_) => emitter.lift(input(), input().depth, &mut |emitter, rows| {
                emitter.canon(rows)
            })?,
            Term::Store { .. } => emitter.lift(input(), input().depth, &mut |emitter, rows| {
                Ok(emitter.store(rows, input().depth > 0))
            })?,
            Term::Union(arms) => {
                let fused = emitter.analysis.results.get(owner).is_some_and(|result| {
                    emitter
                        .analysis
                        .structures
                        .physical
                        .fused
                        .contains_key(result)
                });
                let values = if fused {
                    Vec::new()
                } else {
                    arms.iter()
                        .map(|arm| term(emitter, arm, owner, tiles, false, hook))
                        .collect::<syn::Result<Vec<_>>>()?
                };
                emitter.union(values)?
            }
            Term::Choose(arms) if emitter.pull => {
                let alternative = emitter.choose(arms)?;
                term(emitter, alternative, owner, tiles, false, hook)?
            }
            Term::Choose(arms) => {
                let mut values = Vec::with_capacity(arms.len());
                for (guard, arm) in arms {
                    let condition = emitter.runtime_guard(guard)?;
                    let start = emitter.context().statements.len();
                    let value = term(emitter, arm, owner, tiles, false, hook)?;
                    let statements = emitter.context_mut().statements.split_off(start);
                    values.push((guard, condition, value, statements));
                }
                let depth = values.iter().map(|(_, _, value, _)| value.depth).min()
                    .expect("checked nonempty choice");
                let result = values[0].2.result.clone();
                let stored = values.iter().any(|(_, _, value, _)| value.stored);
                let mut selected = quote!(::core::unreachable!("checked exhaustive Sequence choice"));
                for (guard, condition, mut value, statements) in values.into_iter().rev() {
                    while value.depth > depth {
                        value = emitter.flatten(value);
                    }
                    let rows = collect_choice_rows(value.rows, depth);
                    let branch = quote!({ #(#statements)* #rows });
                    selected = if *guard == Guard::Else {
                        branch
                    } else {
                        quote!(if #condition { #branch } else { #selected })
                    };
                }
                Value {
                    rows: choice_rows(selected, depth),
                    result,
                    depth,
                    stored,
                    partitioned: false,
                }
            }
        };
        value.partitioned |= child.as_ref().is_some_and(|input| input.partitioned);
        hook(node, emitter, child.as_ref(), &mut value, owner, tiles)?;
        Ok(value)
    }
    // Formal rule T-Pull: Interpret Sequence by the host iterator algebra.
    let mut emitter = Emitter::new(program, analysis);
    emitter.pull = pull;
    if profile {
        emitter.context_mut().enable_profile(&program.structures);
    }
    let mut output = None;
    // Recorded fused arms are emitted inside their prefix row's scope, not
    // prepared eagerly as independent top-level streams.
    let deferred: std::collections::BTreeSet<_> = analysis
        .structures
        .physical
        .fused
        .values()
        .flat_map(|group| group.members.iter().cloned())
        .collect();
    for (position, definition) in program.sequence.definitions.iter().enumerate() {
        let owner = symbol_name(&definition.name);
        if analysis
            .results
            .get(&owner)
            .is_some_and(|result| deferred.contains(result))
        {
            continue;
        }
        emitter.owner = owner.clone();
        let final_output = position + 1 == program.sequence.definitions.len();
        let node = match &definition.term {
            Term::Store { inner, .. }
                if pull
                    && final_output
                    && matches!(
                        program.sequence.output,
                        Some(cq_lang_sequence::OutputProtocol::Count)
                    ) =>
            {
                inner.as_ref()
            }
            node => node,
        };
        let value = term(&mut emitter, node, &owner, 0, false, &mut hook)?;
        output = Some(if final_output {
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

fn collect_choice_rows(rows: TokenStream, depth: usize) -> TokenStream {
    if depth == 0 {
        quote!((#rows).collect::<::std::vec::Vec<_>>())
    } else {
        let part = collect_choice_rows(quote!(::core::iter::IntoIterator::into_iter(__cq_choice_part)), depth - 1);
        quote!((#rows).map(|__cq_choice_part| #part).collect::<::std::vec::Vec<_>>())
    }
}

fn choice_rows(rows: TokenStream, depth: usize) -> TokenStream {
    if depth == 0 {
        quote!((#rows).into_iter())
    } else {
        let part = choice_rows(quote!(__cq_choice_part), depth - 1);
        quote!((#rows).into_iter().map(|__cq_choice_part| #part))
    }
}
