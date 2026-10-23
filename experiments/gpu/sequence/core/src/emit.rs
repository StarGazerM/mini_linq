use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

use cq_lang_sequence::{Analysis, Capability, Guard, Len, Monoid, OutputProtocol, Program, Term};
use cq_lang_structures::physical::{Operator, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

use crate::rows::{DeviceRow, Rows};

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(Span::call_site(), message)
}

#[derive(Clone)]
enum Operation {
    Map(String, String),
    Lookup {
        provider: String,
        keys: Vec<String>,
        membership: bool,
    },
    Flatten,
    Tile {
        n: usize,
        items: usize,
    },
    Store,
    Fold(Monoid),
    Canon,
}

fn local_tiles(operations: &[Operation]) -> Vec<(usize, usize)> {
    operations
        .iter()
        .take_while(|operation| {
            !matches!(
                operation,
                Operation::Lookup {
                    membership: false,
                    ..
                } | Operation::Fold(_)
            )
        })
        .filter_map(|operation| {
            if let Operation::Tile { n, items } = operation {
                Some((*n, *items))
            } else {
                None
            }
        })
        .collect()
}

fn nested_partitions(tiles: &[(usize, usize)], mut body: TokenStream) -> TokenStream {
    for &(size, _) in tiles.iter().rev() {
        body = quote! {{
            let mut __part_cursor=__partition_begin;
            while __part_cursor<__partition_end {
                {
                    let __partition_begin=__part_cursor;
                    let __partition_end=(__part_cursor+#size as u64).min(__partition_end);
                    #body
                }
                __part_cursor+=#size as u64;
            }
        }};
    }
    body
}

fn active_tile(operations: &[Operation]) -> Option<usize> {
    let mut partitions = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        match operation {
            Operation::Tile { .. } => partitions.push(Some(index)),
            Operation::Lookup {
                membership: false, ..
            } => partitions.push(None),
            Operation::Flatten => {
                partitions.pop();
            }
            _ => {}
        }
    }
    partitions.last().copied().flatten()
}

fn partition_body(
    tiles: &[(usize, usize)],
    scope: usize,
    declare: TokenStream,
    rows: TokenStream,
    finish: TokenStream,
) -> TokenStream {
    let split = scope.saturating_sub(1).min(tiles.len());
    let rows = nested_partitions(&tiles[split..], rows);
    nested_partitions(&tiles[..split], quote! {#declare #rows #finish})
}

#[derive(Clone)]
struct Chain {
    source: String,
    stored: Option<usize>,
    result: String,
    operations: Vec<Operation>,
    condition: TokenStream,
}

struct Stored {
    name: String,
    result: String,
    range: Option<String>,
    packed: Option<Vec<String>>,
}

struct Compiler<'a> {
    program: &'a Program,
    analysis: &'a Analysis,
    rows: Rows,
    stored: Vec<Stored>,
    definitions: BTreeMap<String, &'a Term>,
}

impl<'a> Compiler<'a> {
    fn fold(&self, term: &Term) -> syn::Result<Vec<Chain>> {
        match term {
            Term::Rows(name) => Ok(vec![Chain {
                source: symbol_name(name),
                stored: None,
                result: symbol_name(name),
                operations: Vec::new(),
                condition: quote!(true),
            }]),
            Term::Ref(name) => {
                let name = symbol_name(name);
                if let Some((index, stored)) =
                    self.stored.iter().enumerate().find(|(_, s)| s.name == name)
                {
                    return Ok(vec![Chain {
                        source: name,
                        stored: Some(index),
                        result: stored.result.clone(),
                        operations: Vec::new(),
                        condition: quote!(true),
                    }]);
                }
                if self
                    .analysis
                    .levels
                    .get(&name)
                    .is_some_and(|level| level.last().is_some_and(|level| level == "host"))
                    && self.analysis.stores.contains_key(&name)
                {
                    let result = self
                        .analysis
                        .results
                        .get(&name)
                        .ok_or_else(|| error("host store lacks checked row provenance"))?
                        .clone();
                    return Ok(vec![Chain {
                        source: result.clone(),
                        stored: None,
                        result,
                        operations: Vec::new(),
                        condition: quote!(true),
                    }]);
                }
                self.fold(
                    self.definitions
                        .get(&name)
                        .ok_or_else(|| error("Sequence reference lacks definition"))?,
                )
            }
            Term::Union(arms) => {
                let mut chains = Vec::new();
                for arm in arms {
                    chains.extend(self.fold(arm)?);
                }
                Ok(chains)
            }
            Term::Choose(arms) => {
                let mut chains = Vec::new();
                let mut previous = quote!(false);
                for (guard, arm) in arms {
                    let selected = self.guard(guard)?;
                    for mut chain in self.fold(arm)? {
                        let old = chain.condition;
                        chain.condition = quote!((#old) && !(#previous) && (#selected));
                        chains.push(chain);
                    }
                    previous = quote!((#previous) || (#selected));
                }
                Ok(chains)
            }
            Term::Map(name, inner) | Term::Compact(name, inner) => {
                let mut chains = self.fold(inner)?;
                for chain in &mut chains {
                    chain
                        .operations
                        .push(Operation::Map(symbol_name(name), chain.result.clone()));
                    chain.result = symbol_name(name);
                }
                Ok(chains)
            }
            Term::Lookup {
                provider,
                keys,
                inner,
            } => {
                let mut chains = self.fold(inner)?;
                let provider = symbol_name(provider);
                let mut keys = keys.iter().map(symbol_name).collect::<Vec<_>>();
                keys.sort();
                let capability = self
                    .analysis
                    .capabilities
                    .get(&(provider.clone(), keys.clone()))
                    .ok_or_else(|| error("Sequence lookup has no checked capability"))?;
                let membership = *capability == Capability::Membership;
                if !matches!(
                    capability,
                    Capability::Range | Capability::Membership | Capability::Exact
                ) {
                    return Err(error(
                        "GPU Sequence requires a recorded range or membership provider",
                    ));
                }
                for chain in &mut chains {
                    if !membership {
                        let result = self
                            .program
                            .structures
                            .physical
                            .program
                            .definitions
                            .iter()
                            .find_map(|definition| match &definition.operator {
                                Operator::Probe(_, keyed) | Operator::Merge(_, keyed)
                                    if symbol_name(&keyed.right) == provider
                                        && symbol_name(&keyed.left) == chain.result =>
                                {
                                    let mut read =
                                        keyed.columns.iter().map(symbol_name).collect::<Vec<_>>();
                                    read.sort();
                                    (read == keys).then(|| symbol_name(&definition.name))
                                }
                                _ => None,
                            })
                            .ok_or_else(|| {
                                error("Sequence lookup lacks checked Physical row provenance")
                            })?;
                        chain.result = result;
                    }
                    chain.operations.push(Operation::Lookup {
                        provider: provider.clone(),
                        keys: keys.clone(),
                        membership,
                    });
                }
                Ok(chains)
            }
            Term::Flatten(inner)
            | Term::Canon(inner)
            | Term::Store { inner, .. }
            | Term::Tile { inner, .. }
            | Term::Fold(_, inner) => {
                let mut chains = self.fold(inner)?;
                let operation = match term {
                    Term::Flatten(_) => Operation::Flatten,
                    Term::Canon(_) => Operation::Canon,
                    Term::Store { .. } => Operation::Store,
                    Term::Tile { n, items, .. } => Operation::Tile {
                        n: *n,
                        items: items.unwrap_or(n.div_ceil(256)),
                    },
                    Term::Fold(monoid, _) => Operation::Fold(monoid.clone()),
                    _ => unreachable!(),
                };
                for chain in &mut chains {
                    chain.operations.push(operation.clone());
                }
                Ok(chains)
            }
        }
    }

    fn length(&self, len: &Len) -> syn::Result<TokenStream> {
        Ok(match len {
            Len::Lit(n) => quote!(#n),
            Len::Input(name) | Len::Scan(name) => {
                if let Some((index, _)) = self
                    .stored
                    .iter()
                    .enumerate()
                    .find(|(_, s)| &s.name == name)
                {
                    let count = format_ident!("__cq_store_length_{index}");
                    quote!(#count)
                } else if let Some(len) = self.analysis.lengths.get(name).filter(|l| *l != len) {
                    self.length(len)?
                } else {
                    self.rows.source_len(name)?
                }
            }
            Len::Add(a, b) => {
                let a = self.length(a)?;
                let b = self.length(b)?;
                quote!((#a).checked_add(#b).expect("Sequence length overflow"))
            }
            Len::CeilDiv(a, n) => {
                let a = self.length(a)?;
                quote!((#a).div_ceil(#n))
            }
            Len::Unknown(Some(n)) => quote!(#n),
            Len::Unknown(None) => {
                return Err(error(
                    "dynamic guard length needs a published Sequence store",
                ));
            }
        })
    }

    fn guard(&self, guard: &Guard) -> syn::Result<TokenStream> {
        Ok(match guard {
            Guard::Else => quote!(true),
            Guard::Lt(len, n) => {
                let len = self.length(len)?;
                quote!((#len) < #n)
            }
            Guard::Ge(len, n) => {
                let len = self.length(len)?;
                quote!((#len) >= #n)
            }
        })
    }

    fn slot(&self, column: &str) -> syn::Result<usize> {
        self.rows
            .columns
            .get(column)
            .copied()
            .ok_or_else(|| error(format!("missing device column {column}")))
    }

    fn lookup(
        &self,
        provider: &str,
        keys: &[String],
        membership: bool,
    ) -> syn::Result<TokenStream> {
        let (view, payload, order) = self.rows.provider(provider)?;
        let first = order
            .first()
            .ok_or_else(|| error("range provider has no directory key"))?;
        if !keys.contains(first) {
            return Err(error("lookup does not bind the recorded directory key"));
        }
        let slot = self.slot(first)?;
        if order.len() < keys.len() || order[..keys.len()].iter().any(|key| !keys.contains(key)) {
            return Err(error(
                "lookup keys are not a prefix of the checked provider order",
            ));
        }
        let mut membership_checks = TokenStream::new();
        for (depth, key) in order.iter().take(keys.len()).enumerate().skip(1) {
            let index = payload
                .iter()
                .position(|column| column == key)
                .ok_or_else(|| error("lookup key missing from sorted provider payload"))?;
            let slot = self.slot(key)?;
            if membership && depth + 1 == keys.len() {
                membership_checks.extend(quote! {
                    __live=__live && ::cq_gpu_sequence_runtime::device::contains_u32(
                        *#view.payload.add(#index),__begin,__end,__row[#slot] as u32);
                });
            } else {
                membership_checks.extend(quote! {
                    let __key=__row[#slot] as i64;
                    let __column=*#view.payload.add(#index);
                    let (mut __low,mut __high)=(__begin,__end);
                    while __low<__high {
                        let __middle=__low+(__high-__low)/2;
                        if ::cq_gpu_sequence_runtime::device::decode(__column,__middle as u64)<__key {
                            __low=__middle+1;
                        } else {__high=__middle;}
                    }
                    __begin=__low;
                    __high=__end;
                    while __low<__high {
                        let __middle=__low+(__high-__low)/2;
                        if ::cq_gpu_sequence_runtime::device::decode(__column,__middle as u64)<=__key {
                            __low=__middle+1;
                        } else {__high=__middle;}
                    }
                    __end=__low;
                });
            }
        }
        Ok(quote! {
            let (mut __begin, mut __end) = (0u32, 0u32);
            if __live && (__row[#slot] as i64) >= 0 && (__row[#slot] as u64) < #view.domain as u64 {
                (__begin, __end) = ::cq_gpu_sequence_runtime::device::range(#view.offsets, __row[#slot] as u32);
            }
            #membership_checks
        })
    }

    fn payload(&self, provider: &str, position: TokenStream) -> syn::Result<TokenStream> {
        let (view, payload, _) = self.rows.provider(provider)?;
        let assignments = payload.iter().enumerate().map(|(index,column)| {
            let slot=self.slot(column)?;
            let decode = if self.rows.dense.contains(&slot) { format_ident!("decode_u32") } else { format_ident!("decode") };
            Ok(quote!(__row[#slot] = ::cq_gpu_sequence_runtime::device::#decode(*#view.payload.add(#index), (#position) as u64) as i64;))
        }).collect::<syn::Result<Vec<_>>>()?;
        Ok(quote!(#(#assignments)*))
    }
}

struct Kernel<'c, 'p> {
    compiler: &'c Compiler<'p>,
    shared_bytes: usize,
    virtual_range: Option<String>,
    packed_columns: Option<Vec<String>>,
    publication_columns: BTreeSet<usize>,
    sink: Option<TokenStream>,
    scalar_output: bool,
}

struct StripeInput {
    items: usize,
    inherited: DeviceRow,
    prelude: TokenStream,
    load: TokenStream,
}

impl Kernel<'_, '_> {
    fn allocate(&mut self, bytes: usize, align: usize) -> usize {
        self.shared_bytes = self.shared_bytes.div_ceil(align) * align;
        let at = self.shared_bytes;
        self.shared_bytes += bytes;
        at
    }

    fn output_slots(&self) -> syn::Result<BTreeSet<usize>> {
        if let Some(columns) = &self.packed_columns {
            columns
                .iter()
                .map(|column| self.compiler.slot(column))
                .collect()
        } else {
            Ok(self.publication_columns.clone())
        }
    }

    /// Backwards column liveness follows the checked row functions and provider
    /// payload headings. No scheduling or provider choice is made here.
    fn live_columns(&self, operations: &[Operation]) -> syn::Result<BTreeSet<usize>> {
        let mut live = if let Some(sink) = &self.sink {
            DeviceRow::uses(sink.clone())?
        } else {
            self.output_slots()?
        };
        for operation in operations.iter().rev() {
            match operation {
                Operation::Map(name, input) => live.extend(DeviceRow::uses(
                    self.compiler
                        .rows
                        .map(name, input, quote!(__row), quote!(__live))?,
                )?),
                Operation::Lookup {
                    provider,
                    keys,
                    membership,
                } => {
                    if !membership {
                        for column in self.compiler.rows.provider(provider)?.1 {
                            live.remove(&self.compiler.slot(&column)?);
                        }
                    }
                    for column in keys {
                        live.insert(self.compiler.slot(column)?);
                    }
                }
                Operation::Fold(monoid) => {
                    live.clear();
                    if let Monoid::Sum(column) = monoid {
                        live.insert(self.compiler.slot(&symbol_name(column))?);
                    }
                }
                _ => {}
            }
        }
        Ok(live)
    }

    fn code_type(&self, slot: usize) -> TokenStream {
        if self.compiler.rows.dense.contains(&slot) {
            quote!(u32)
        } else {
            quote!(i64)
        }
    }

    fn scalar_stripes(
        &mut self,
        operations: &[Operation],
        items: usize,
        inherited: DeviceRow,
        prelude: TokenStream,
        load: TokenStream,
    ) -> syn::Result<TokenStream> {
        let mut row = inherited;
        let mut declarations = TokenStream::new();
        for slot in self.live_columns(operations)? {
            if let std::collections::btree_map::Entry::Vacant(entry) = row.0.entry(slot) {
                let id = format_ident!("__code_{slot}");
                let ty = self.code_type(slot);
                declarations.extend(quote!(let mut #id: #ty = 0;));
                entry.insert(quote!(#id));
            }
        }
        let load = row.rewrite(load)?;
        let terminal = self.sink.is_none()
            && operations.iter().enumerate().all(|(at, operation)| {
                if let Operation::Lookup {
                    provider,
                    membership: false,
                    ..
                } = operation
                {
                    self.virtual_range.as_deref() == Some(provider)
                        && operations[at + 1..]
                            .iter()
                            .all(|op| matches!(op, Operation::Flatten | Operation::Store))
                } else {
                    true
                }
            });
        let mut gather_declare = TokenStream::new();
        let mut emit = TokenStream::new();
        let previous_sink = self.sink.clone();
        if terminal {
            let mut gathered = DeviceRow::default();
            let mut collect = TokenStream::new();
            for slot in self.output_slots()? {
                let id = format_ident!("__found_code_{slot}");
                let ty = self.code_type(slot);
                gather_declare.extend(quote!(let mut #id = [0 as #ty; #items];));
                collect.extend(quote!(#id[__item] = __row[#slot] as #ty;));
                gathered.0.insert(slot, quote!(#id[__item]));
            }
            let mut range_load = TokenStream::new();
            if self.virtual_range.is_some() {
                gather_declare.extend(quote! {
                    let mut __found_begin = [0u32; #items];
                    let mut __found_end = [0u32; #items];
                });
                collect.extend(quote! {
                    __found_begin[__item] = __begin;
                    __found_end[__item] = __end;
                });
                range_load = quote! {
                    let __begin = __found_begin[__item];
                    let __end = __found_end[__item];
                };
            }
            self.sink = Some(quote! {
                __found[__item] = __live;
                if __live { #collect __found_count += 1; }
            });
            let write = gathered.rewrite(self.output_write()?)?;
            emit = Self::reserve(
                quote!(__found_count),
                quote! {
                    let mut __destination = (*__reservation).wrapping_add(__prefix);
                    let mut __item = 0usize;
                    while __item < #items {
                        if __found[__item] {
                            if __destination < __capacity { #range_load #write }
                            __destination = __destination.wrapping_add(1);
                        }
                        __item += 1;
                    }
                },
            );
            gather_declare.extend(quote! {
                let mut __found = [false; #items];
                let mut __found_count = 0u32;
            });
        }
        let body = self.scalar_operations(operations, &row)?;
        self.sink = previous_sink;
        Ok(quote! {
            let mut __batch = __partition_begin;
            while __batch < __partition_end {
                #gather_declare
                let mut __item = 0usize;
                while __item < #items {
                    let __position = __batch + (__item * 256 + __tid) as u64;
                    let mut __live = __position < __partition_end;
                    #prelude
                    #declarations
                    if __live { #load }
                    #body
                    __item += 1;
                }
                #emit
                __batch += (#items as u64) * 256;
            }
        })
    }

    fn scalar_operations(
        &mut self,
        operations: &[Operation],
        row: &DeviceRow,
    ) -> syn::Result<TokenStream> {
        let Some((operation, rest)) = operations.split_first() else {
            return row.rewrite(self.finish()?);
        };
        match operation {
            Operation::Map(name, input) => {
                let map = row.rewrite(self.compiler.rows.map(
                    name,
                    input,
                    quote!(__row),
                    quote!(__live),
                )?)?;
                let rest = self.scalar_operations(rest, row)?;
                Ok(quote!(#map #rest))
            }
            Operation::Lookup {
                provider,
                keys,
                membership,
            } => {
                let lookup = row.rewrite(self.compiler.lookup(provider, keys, *membership)?)?;
                if *membership {
                    let rest = self.scalar_operations(rest, row)?;
                    return Ok(quote!(#lookup __live = __live && __begin < __end; #rest));
                }
                if self.virtual_range.as_deref() == Some(provider)
                    && rest
                        .iter()
                        .all(|op| matches!(op, Operation::Flatten | Operation::Store))
                {
                    let finish = row.rewrite(self.finish()?)?;
                    return Ok(quote!(#lookup __live = __live && __begin < __end; #finish));
                }
                if !rest.iter().any(|op| matches!(op, Operation::Flatten)) {
                    return Err(error(
                        "range lookup must be flattened or published as offsets",
                    ));
                }
                let payload_columns = self
                    .compiler
                    .rows
                    .provider(provider)?
                    .1
                    .iter()
                    .map(|column| self.compiler.slot(column))
                    .collect::<syn::Result<BTreeSet<_>>>()?;
                let begin_offset = self.allocate(256 * 4, 4);
                let offsets_offset = self.allocate(257 * 4, 4);
                let begins = format_ident!("__begins_{begin_offset}");
                let offsets = format_ident!("__offsets_{offsets_offset}");
                let owner = format_ident!("__owner_{begin_offset}");
                let mut shared = TokenStream::new();
                let mut child = DeviceRow::default();
                for slot in self.live_columns(rest)?.difference(&payload_columns) {
                    let ty = self.code_type(*slot);
                    let bytes = if self.compiler.rows.dense.contains(slot) {
                        4
                    } else {
                        8
                    };
                    let at = self.allocate(256 * bytes, bytes);
                    let id = format_ident!("__parent_{at}");
                    let value = row
                        .0
                        .get(slot)
                        .ok_or_else(|| error("live parent column lacks checked provenance"))?;
                    shared.extend(quote! {
                        let #id = __shared.add(#at).cast::<#ty>();
                        *#id.add(__tid) = #value;
                    });
                    child.0.insert(*slot, quote!(*#id.add(#owner)));
                }
                let tiles = local_tiles(rest);
                let tile = tiles.first().map_or(256, |tile| tile.0);
                let items = tiles.last().map_or(1, |tile| tile.1);
                let payload = self.compiler.payload(provider, quote!(__child_position))?;
                let (_, partition, _, _) = self.partition_rows(rest, false,
                    tiles.get(1..).unwrap_or_default(), StripeInput {
                        items,
                        inherited: child,
                        prelude: quote! {
                            let #owner = if __live {
                                ::cq_gpu_sequence_runtime::device::owner(#offsets, 256, __position as u32) as usize
                            } else { 0 };
                        },
                        load: quote! {
                            let __child_position = *#begins.add(#owner) + (__position as u32 - *#offsets.add(#owner));
                            #payload
                        },
                    })?;
                Ok(quote! {
                    #lookup
                    #shared
                    let #begins = __shared.add(#begin_offset).cast::<u32>();
                    let #offsets = __shared.add(#offsets_offset).cast::<u32>();
                    *#begins.add(__tid) = __begin;
                    let (__offset, __children) = ::cq_gpu_sequence_runtime::device::checked_scan(
                        if __live { __end - __begin } else { 0 }, __scratch, __meta.add(2));
                    *#offsets.add(__tid) = __offset;
                    if __tid == 0 { *#offsets.add(256) = __children; }
                    ::cuda_device::thread::sync_threads();
                    if __tid == 0 { *__abort_slot = *__meta.add(2); }
                    ::cuda_device::thread::sync_threads();
                    if *__abort_slot != 0 { return; }
                    let mut __child_base = 0u64;
                    while __child_base < __children as u64 {
                        let __partition_begin = __child_base;
                        let __partition_end = (__child_base + #tile as u64).min(__children as u64);
                        #partition
                        __child_base += #tile as u64;
                    }
                    ::cuda_device::thread::sync_threads();
                })
            }
            Operation::Fold(_) => Err(error("scalar row template cannot cross a reduction scope")),
            _ => self.scalar_operations(rest, row),
        }
    }

    fn partition_rows(
        &mut self,
        operations: &[Operation],
        source: bool,
        tiles: &[(usize, usize)],
        input_rows: StripeInput,
    ) -> syn::Result<(TokenStream, TokenStream, TokenStream, bool)> {
        let Some(at) = operations
            .iter()
            .position(|operation| matches!(operation, Operation::Fold(_)))
        else {
            let rows = self.scalar_stripes(
                operations,
                input_rows.items,
                input_rows.inherited,
                input_rows.prelude,
                input_rows.load,
            )?;
            return Ok((
                TokenStream::new(),
                nested_partitions(tiles, rows),
                TokenStream::new(),
                false,
            ));
        };
        let target = active_tile(&operations[..at]);
        let next_lookup = operations
            .iter()
            .position(|operation| {
                matches!(
                    operation,
                    Operation::Lookup {
                        membership: false,
                        ..
                    }
                )
            })
            .unwrap_or(at)
            .min(at);
        let current = active_tile(&operations[..next_lookup]);
        if !(source && target.is_none()) && (target != current || target.is_none()) {
            let rows = self.scalar_stripes(
                operations,
                input_rows.items,
                input_rows.inherited,
                input_rows.prelude,
                input_rows.load,
            )?;
            return Ok((
                TokenStream::new(),
                nested_partitions(tiles, rows),
                TokenStream::new(),
                false,
            ));
        }
        let Operation::Fold(monoid) = &operations[at] else {
            unreachable!()
        };
        let offset = self.allocate(256 * 8, 8);
        let accumulator = format_ident!("__cq_fold_accumulator_{offset}");
        let input = match monoid {
            Monoid::Sum(column) => Some(self.compiler.slot(&symbol_name(column))?),
            _ => None,
        };
        let output = self.compiler.slot(&self.compiler.rows.output_columns[0])?;
        let reduction = crate::reduce::build(monoid, input, output, offset, &accumulator)?;
        let previous = self.sink.replace(reduction.update);
        let body = self.scalar_stripes(
            &operations[..at],
            input_rows.items,
            input_rows.inherited,
            input_rows.prelude,
            input_rows.load,
        )?;
        self.sink = previous;
        let mut output_row = DeviceRow::default();
        output_row.0.insert(output, quote!(__fold_result));
        // A reduction result is an i64 value, not a dense code inherited
        // from the physical column whose output slot it reuses.
        let previous_scalar = std::mem::replace(&mut self.scalar_output, true);
        let suffix = self.scalar_operations(&operations[at + 1..], &output_row)?;
        self.scalar_output = previous_scalar;
        let finish = output_row.rewrite(reduction.finish)?;
        let finish = quote! {
            let mut __fold_result = 0i64;
            let mut __live = false;
            #finish
            #suffix
        };
        let scope = target.map_or(0, |target| {
            operations[..=target]
                .iter()
                .filter(|operation| matches!(operation, Operation::Tile { .. }))
                .count()
        });
        if target.is_none() {
            Ok((
                reduction.declare,
                nested_partitions(tiles, body),
                finish,
                true,
            ))
        } else {
            Ok((
                TokenStream::new(),
                partition_body(tiles, scope, reduction.declare, body, finish),
                TokenStream::new(),
                false,
            ))
        }
    }

    fn finish(&self) -> syn::Result<TokenStream> {
        if let Some(sink) = &self.sink {
            return Ok(sink.clone());
        }
        let write = self.output_write()?;
        Ok(Self::reserve(
            quote!(__live as u32),
            quote! {
                let __destination=(*__reservation).wrapping_add(__prefix);
                if __live && __destination < __capacity { #write }
            },
        ))
    }

    fn output_write(&self) -> syn::Result<TokenStream> {
        let width = self.compiler.rows.width;
        let write = if let Some(columns) = &self.packed_columns {
            let parts=columns.iter().enumerate().map(|(i,column)| {
                let slot=self.compiler.slot(column)?;
                let base=format_ident!("__cq_pack_base_{i}");
                let shift=format_ident!("__cq_pack_shift_{i}");
                if self.compiler.rows.dense.contains(&slot) && !self.scalar_output {
                    Ok(quote!((if #shift==64 {0u64} else {((__row[#slot] as u32).wrapping_sub(#base as u32) as u64) << #shift})))
                } else {
                    Ok(quote!((if #shift==64 {0u64} else {(__row[#slot].wrapping_sub(#base) as u64) << #shift})))
                }
            }).collect::<syn::Result<Vec<_>>>()?;
            quote!(*__output_keys.add(__destination as usize) = 0u64 #(| #parts)*;)
        } else {
            let range = if self.virtual_range.is_some() {
                quote! {
                    *__output_begin.add(__destination as usize) = __begin;
                    *__output_offsets.add(__destination as usize) = __end - __begin;
                }
            } else {
                TokenStream::new()
            };
            let columns = (0..width).map(|slot| quote! {
                *__output_rows.add(__destination as usize * #width + #slot) = __row[#slot] as i64;
            });
            quote! {
                #(#columns)*
                #range
            }
        };
        Ok(write)
    }

    fn reserve(count: TokenStream, emit: TokenStream) -> TokenStream {
        quote! {
            let (__prefix,__total) = ::cq_gpu_sequence_runtime::device::checked_scan(#count, __scratch, __meta.add(2));
            if __tid == 0 {
                let __base = if __total == 0 { 0 } else {
                    ::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta).fetch_add(__total, ::cuda_device::atomic::AtomicOrdering::Relaxed)
                };
                if __base.checked_add(__total).is_none() {
                    ::cq_gpu_sequence_runtime::device::flag_overflow(__meta.add(2));
                }
                *__reservation = __base;
            }
            ::cuda_device::thread::sync_threads();
            #emit
            ::cuda_device::thread::sync_threads();
        }
    }
}

/// Translate only checked scheduling facts; no planner is consulted here.
// Formal rule T-Gpu: Interpret checked Sequence constructors at their recorded GPU grades.
pub fn translate(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    if program
        .graded
        .as_ref()
        .is_none_or(|graded| graded.machine != "gpu")
    {
        return Err(error(
            "GPU Sequence translation requires checked gpu grading",
        ));
    }
    for (_, _, level) in &analysis.collectives {
        if level.last().is_some_and(|level| level == "device") {
            return Err(error(
                "GPU Sequence fold requires a collaborative block or sequential lane, not device",
            ));
        }
    }
    let rows = Rows::new(program, analysis)?;
    let mut compiler = Compiler {
        program,
        analysis,
        rows,
        stored: Vec::new(),
        definitions: program
            .sequence
            .definitions
            .iter()
            .map(|d| (symbol_name(&d.name), &d.term))
            .collect(),
    };
    let mut kernels = Vec::new();
    let mut launches = Vec::new();
    let mut output = None;
    let final_name = analysis
        .output
        .as_ref()
        .map(|(name, _)| name.as_str())
        .or_else(|| {
            program
                .sequence
                .definitions
                .last()
                .map(|definition| {
                    // Borrow the key owned by the checked analysis instead of a temporary Ident spelling.
                    analysis
                        .levels
                        .keys()
                        .find(|name| **name == symbol_name(&definition.name))
                        .map(String::as_str)
                })
                .flatten()
        })
        .ok_or_else(|| error("GPU Sequence lacks a checked output"))?;
    fn output_fold<'a>(
        term: &'a Term,
        definitions: &BTreeMap<String, &'a Term>,
    ) -> Option<&'a Monoid> {
        match term {
            Term::Fold(monoid, _) => Some(monoid),
            Term::Canon(inner) | Term::Flatten(inner) | Term::Store { inner, .. } => {
                output_fold(inner, definitions)
            }
            Term::Ref(name) => definitions
                .get(&symbol_name(name))
                .and_then(|term| output_fold(term, definitions)),
            _ => None,
        }
    }
    let final_fold = compiler
        .definitions
        .get(final_name)
        .and_then(|term| output_fold(term, &compiler.definitions));
    if final_fold.is_some() && compiler.rows.output_columns.len() != 1 {
        return Err(error(
            "GPU scalar fold publication requires one output column",
        ));
    }
    let mut packing = Vec::new();
    for (i, column) in compiler.rows.output_columns.iter().enumerate() {
        let bounds = match final_fold {
            Some(Monoid::Count) => quote!(::cq_gpu_runtime::Bounds::Inclusive {
                min: 0,
                max: i64::MAX
            }),
            Some(Monoid::Or) => quote!(::cq_gpu_runtime::Bounds::Inclusive { min: 0, max: 1 }),
            Some(Monoid::Sum(_)) => quote!(::cq_gpu_runtime::Bounds::Inclusive {
                min: i64::MIN,
                max: i64::MAX
            }),
            None => compiler.rows.bounds(column)?,
        };
        let base = format_ident!("__cq_pack_base_{i}");
        let shift = format_ident!("__cq_pack_shift_{i}");
        packing.push(quote! {
            let (#base, __cq_width)=match (#bounds) {
                ::cq_gpu_runtime::Bounds::Empty => (0i64,0u32),
                ::cq_gpu_runtime::Bounds::Inclusive {min,max} => {
                    let difference=(max as i128-min as i128) as u64;
                    (min,64-difference.leading_zeros())
                }
            };
            let #shift=__cq_key_bits;
            __cq_key_bits=__cq_key_bits.checked_add(__cq_width).ok_or("packed Sequence key width overflow")?;
            if __cq_key_bits>64 {return Err("packed Sequence equality key exceeds 64 bits".into());}
            __cq_fields.push(::cq_gpu_sequence_runtime::canon::PackedField {base:#base,shift:#shift,bits:__cq_width});
        });
    }
    // CUDA exports are not scoped by the surrounding Rust expression. Give
    // each expansion its own module, including repeated identical programs.
    static NEXT_MODULE: AtomicUsize = AtomicUsize::new(0);
    let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
    quote!(#program).to_string().hash(&mut fingerprint);
    let module_name = format_ident!(
        "__cq_sequence_kernels_{:016x}_{}",
        fingerprint.finish(),
        NEXT_MODULE.fetch_add(1, Ordering::Relaxed)
    );
    for (index, dispatch) in analysis.dispatches.iter().enumerate() {
        let publication = dispatch
            .inner_stores
            .iter()
            .rev()
            .find(|name| {
                compiler
                    .definitions
                    .get(*name)
                    .is_some_and(|term| matches!(term, Term::Store { .. }))
            })
            .ok_or_else(|| error("checked dispatch has no explicit publication store"))?;
        let term = compiler
            .definitions
            .get(publication)
            .ok_or_else(|| error("checked dispatch names an absent publication"))?;
        let Term::Store { capacity, inner } = term else {
            return Err(error(
                "GPU dispatch publication must be an explicit outer store",
            ));
        };
        if !analysis.levels.contains_key(publication)
            || !analysis.shapes.contains_key(publication)
            || !analysis.lengths.contains_key(publication)
        {
            return Err(error(
                "GPU publication lacks checked level, shape, or length facts",
            ));
        }
        if dispatch
            .producer_level
            .last()
            .is_none_or(|level| level != "block" && level != "lane")
        {
            return Err(error(
                "GPU dispatch producer must execute on blocks or lanes",
            ));
        }
        let final_output = publication == final_name;
        let chains = compiler.fold(inner)?;
        if chains.is_empty() {
            return Err(error("GPU dispatch has no source"));
        }
        let canonical = chains.iter().all(|chain| {
            chain
                .operations
                .iter()
                .any(|op| matches!(op, Operation::Canon))
        });
        if !canonical
            && chains.iter().any(|chain| {
                chain
                    .operations
                    .iter()
                    .any(|op| matches!(op, Operation::Canon))
            })
        {
            return Err(error(
                "GPU union/choice publication requires one common recorded canon boundary",
            ));
        }
        for chain in &chains {
            let mut partitions = 0usize;
            for (at, operation) in chain.operations.iter().enumerate() {
                match operation {
                    Operation::Tile { .. }
                    | Operation::Lookup {
                        membership: false, ..
                    } => partitions += 1,
                    Operation::Flatten => partitions = partitions.saturating_sub(1),
                    Operation::Canon if partitions != 0 => {
                        return Err(error(
                            "GPU nested canon requires a segmented canonical provider; PackedCanon only certifies a complete flat publication",
                        ));
                    }
                    _ => {}
                }
                if matches!(operation, Operation::Canon)
                    && chain.operations[at + 1..].iter().any(|operation| {
                        !matches!(operation, Operation::Flatten | Operation::Canon)
                    })
                {
                    return Err(error(
                        "GPU Sequence canon inside a dispatch has no separated publication where its runtime provider can execute",
                    ));
                }
                if matches!(operation, Operation::Fold(_))
                    && compiler.rows.output_columns.len() != 1
                {
                    return Err(error(
                        "GPU scalar fold publication requires one output column",
                    ));
                }
                if matches!(operation, Operation::Fold(_)) {
                    let ranges = chain.operations[..at]
                        .iter()
                        .enumerate()
                        .filter_map(|(index, operation)| {
                            matches!(
                                operation,
                                Operation::Lookup {
                                    membership: false,
                                    ..
                                }
                            )
                            .then_some(index)
                        })
                        .collect::<Vec<_>>();
                    if ranges.len() > 1
                        && active_tile(&chain.operations[..at]).is_some_and(|tile| tile > ranges[0])
                    {
                        return Err(error(
                            "GPU fold over a later fan-out partition requires its complete materialized parent extent, not streamed parent stripes",
                        ));
                    }
                }
            }
        }
        let provider_store = analysis.structures.built.contains_key(&chains[0].result);
        let virtual_range = if !final_output && !canonical && !provider_store && chains.len() == 1 {
            let ops = &chains[0].operations;
            let last = ops.iter().rposition(|op| {
                matches!(
                    op,
                    Operation::Lookup {
                        membership: false,
                        ..
                    }
                )
            });
            last.and_then(|at| {
                if ops[at + 1..]
                    .iter()
                    .all(|op| matches!(op, Operation::Flatten | Operation::Store))
                    && ops[at + 1..]
                        .iter()
                        .any(|op| matches!(op, Operation::Flatten))
                {
                    if let Operation::Lookup { provider, .. } = &ops[at] {
                        Some(provider.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        } else {
            None
        };
        let publication_fold = output_fold(inner, &compiler.definitions);
        let packed_columns = if final_output {
            Some(compiler.rows.output_columns.clone())
        } else if canonical {
            if publication_fold.is_some() {
                Some(vec![compiler.rows.output_columns[0].clone()])
            } else {
                Some(
                    analysis
                        .structures
                        .physical
                        .headings
                        .get(&chains[0].result)
                        .ok_or_else(|| error("canonical publication has no checked row heading"))?
                        .clone(),
                )
            }
        } else {
            None
        };
        let mut local_packing = if final_output {
            packing.clone()
        } else {
            Vec::new()
        };
        if canonical && !final_output {
            for (i, column) in packed_columns
                .as_ref()
                .expect("canonical row layout")
                .iter()
                .enumerate()
            {
                let bounds = match publication_fold {
                    Some(Monoid::Count) => quote!(::cq_gpu_runtime::Bounds::Inclusive {
                        min: 0,
                        max: i64::MAX
                    }),
                    Some(Monoid::Or) => {
                        quote!(::cq_gpu_runtime::Bounds::Inclusive { min: 0, max: 1 })
                    }
                    Some(Monoid::Sum(_)) => quote!(::cq_gpu_runtime::Bounds::Inclusive {
                        min: i64::MIN,
                        max: i64::MAX
                    }),
                    None => compiler.rows.bounds(column)?,
                };
                let base = format_ident!("__cq_pack_base_{i}");
                let shift = format_ident!("__cq_pack_shift_{i}");
                local_packing.push(quote! {
                    let (#base,__cq_width)=match (#bounds) {
                        ::cq_gpu_runtime::Bounds::Empty=>(0i64,0u32),
                        ::cq_gpu_runtime::Bounds::Inclusive {min,max}=>
                            (min,64-((max as i128-min as i128) as u64).leading_zeros()),
                    };
                    let #shift=__cq_key_bits;
                    __cq_key_bits=__cq_key_bits.checked_add(__cq_width).ok_or("Sequence equality width overflow")?;
                    if __cq_key_bits>64 {return Err("packed Sequence equality key exceeds 64 bits".into());}
                    __cq_fields.push(::cq_gpu_sequence_runtime::canon::PackedField {base:#base,shift:#shift,bits:__cq_width});
                });
            }
        }
        let kernel_name = format_ident!("__cq_sequence_dispatch_{index}_{module_name}");
        let prepare_name = format_ident!("prepare_{kernel_name}");
        let mut parameters = Vec::new();
        let mut arguments = Vec::new();
        for (name, ty, value) in &compiler.rows.parameters {
            parameters.push(quote!(#name:#ty));
            arguments.push(value.clone());
        }
        for prior in 0..compiler.stored.len() {
            let rows = format_ident!("__cq_store_rows_{prior}");
            let begin = format_ident!("__cq_store_begin_{prior}");
            let offsets = format_ident!("__cq_store_offsets_{prior}");
            let meta = format_ident!("__cq_store_meta_{prior}");
            parameters.extend([
                quote!(#rows:*const i64),
                quote!(#begin:*const u32),
                quote!(#offsets:*const u32),
                quote!(#meta:*const u32),
            ]);
            arguments.extend([
                quote!(#rows.ptr()),
                quote!(#begin.ptr()),
                quote!(#offsets.ptr()),
                quote!(#meta.ptr()),
            ]);
            if let Some(columns) = &compiler.stored[prior].packed {
                let keys = format_ident!("__cq_store_packed_{prior}");
                let owner = format_ident!("__cq_canonical_{prior}");
                let fields = format_ident!("__cq_fields_{prior}");
                parameters.push(quote!(#keys:*const u64));
                arguments.push(quote!(#owner.keys().ptr()));
                for field in 0..columns.len() {
                    let base = format_ident!("__cq_stored_base_{prior}_{field}");
                    let shift = format_ident!("__cq_stored_shift_{prior}_{field}");
                    let bits = format_ident!("__cq_stored_bits_{prior}_{field}");
                    parameters.extend([quote!(#base:i64), quote!(#shift:u32), quote!(#bits:u32)]);
                    arguments.extend([
                        quote!(#fields[#field].base),
                        quote!(#fields[#field].shift),
                        quote!(#fields[#field].bits),
                    ]);
                }
            }
        }
        for i in 0..packed_columns.as_ref().map_or(0, Vec::len) {
            let base = format_ident!("__cq_pack_base_{i}");
            let shift = format_ident!("__cq_pack_shift_{i}");
            parameters.extend([quote!(#base:i64), quote!(#shift:u32)]);
            arguments.extend([quote!(#base), quote!(#shift)]);
        }
        let mut generator = Kernel {
            compiler: &compiler,
            shared_bytes: 48,
            virtual_range: virtual_range.clone(),
            packed_columns: packed_columns.clone(),
            publication_columns: chains
                .iter()
                .flat_map(|chain| {
                    analysis
                        .structures
                        .physical
                        .headings
                        .get(&chain.result)
                        .into_iter()
                        .flatten()
                })
                .map(|column| compiler.slot(column))
                .collect::<syn::Result<_>>()?,
            sink: None,
            scalar_output: false,
        };
        let mut bodies = Vec::new();
        let mut host_domains = Vec::new();
        let mut whole_reduction = false;
        let width = compiler.rows.width;
        for (chain_index, chain) in chains.iter().enumerate() {
            let domain = format_ident!("__cq_domain_{chain_index}");
            let enabled = format_ident!("__cq_enabled_{chain_index}");
            let condition = &chain.condition;
            let source_length = if let Some((prior, _)) = compiler
                .stored
                .iter()
                .enumerate()
                .find(|(_, store)| store.name == dispatch.outer_store)
            {
                let length = format_ident!("__cq_store_length_{prior}");
                quote!(#length)
            } else {
                let source = analysis
                    .results
                    .get(&dispatch.outer_store)
                    .map(String::as_str)
                    .unwrap_or(&dispatch.outer_store);
                compiler.rows.source_len(source)?
            };
            parameters.extend([quote!(#domain:u32), quote!(#enabled:bool)]);
            arguments.extend([quote!(#domain), quote!(#enabled)]);
            host_domains.push(quote! {
                let #domain=u32::try_from(#source_length).map_err(|_|"Sequence source exceeds u32 positions")?;
                let #enabled=#condition;
            });
            let source = if let Some(prior) = chain.stored {
                let rows = format_ident!("__cq_store_rows_{prior}");
                let begin = format_ident!("__cq_store_begin_{prior}");
                let offsets = format_ident!("__cq_store_offsets_{prior}");
                let meta = format_ident!("__cq_store_meta_{prior}");
                if let Some(columns) = &compiler.stored[prior].packed {
                    let keys = format_ident!("__cq_store_packed_{prior}");
                    let mut unpack = TokenStream::new();
                    for (field, column) in columns.iter().enumerate() {
                        let slot = compiler.slot(column)?;
                        let base = format_ident!("__cq_stored_base_{prior}_{field}");
                        let shift = format_ident!("__cq_stored_shift_{prior}_{field}");
                        let bits = format_ident!("__cq_stored_bits_{prior}_{field}");
                        unpack.extend(quote! {
                            let __mask=if #bits==64 {u64::MAX} else {(1u64<<#bits)-1};
                            let __offset=if #bits==0 {0} else {(__key>>#shift)&__mask};
                            __row[#slot]=#base.wrapping_add(__offset as i64);
                        });
                    }
                    quote! {let __key=*#keys.add(__position as usize);#unpack}
                } else if let Some(provider) = &compiler.stored[prior].range {
                    let payload = compiler.payload(provider, quote!(__source_position))?;
                    let copy = (0..width).map(|slot| {
                        quote! {
                            __row[#slot] = *#rows.add(__parent * #width + #slot);
                        }
                    });
                    quote! {
                        let __parent=::cq_gpu_sequence_runtime::device::owner(#offsets,*#meta,__position as u32) as usize;
                        #(#copy)*
                        let __source_position=*#begin.add(__parent)+(__position as u32-*#offsets.add(__parent));
                        #payload
                    }
                } else {
                    let copy = (0..width).map(|slot| {
                        quote! {
                            __row[#slot] = *#rows.add(__position as usize * #width + #slot);
                        }
                    });
                    quote!(#(#copy)*)
                }
            } else {
                compiler
                    .rows
                    .source(&chain.source, quote!(__position), quote!(__row))?
            };
            let tiles = local_tiles(&chain.operations);
            let tile = if let Some(width) = dispatch.parts {
                if width == 0 {
                    return Err(error("zero-width dispatch tile"));
                }
                quote!(#width)
            } else {
                // No distributing tile denotes one whole input part, not an
                // emitter-selected width or an implicit multi-block schedule.
                quote!((#domain as u64).max(1))
            };
            let items = tiles.last().map_or(1, |tile| tile.1);
            let (outside_declare, partition, outside_finish, whole) = generator.partition_rows(
                &chain.operations,
                true,
                tiles.get(1..).unwrap_or_default(),
                StripeInput {
                    items,
                    inherited: DeviceRow::default(),
                    prelude: TokenStream::new(),
                    load: source,
                },
            )?;
            whole_reduction |= whole;
            // Every block finishes one branch before entering the next. Distinct
            // cursor words avoid cross-branch reset races in multi-block launches.
            let cursor = 4 + chain_index;
            bodies.push(quote! {
                if #enabled {
                    #outside_declare
                    loop {
                        if __tid==0 {
                            *__tile_slot=::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(#cursor))
                                .fetch_add(1,::cuda_device::atomic::AtomicOrdering::Relaxed);
                        }
                        ::cuda_device::thread::sync_threads();
                        let __tile=*__tile_slot as u64;
                        if __tile*#tile as u64>=#domain as u64 {break;}
                        let __partition_begin=__tile*#tile as u64;
                        let __partition_end=(__partition_begin+#tile as u64).min(#domain as u64);
                        #partition
                    }
                    #outside_finish
                }
            });
        }
        let finalize = if virtual_range.is_some() {
            quote! {
                ::cuda_device::thread::sync_threads();
                if __tid==0 {
                    let __count=(*__meta).min(__capacity);
                    let mut __sum=0u32;
                    let mut __index=0u32;
                    while __index<__count {
                        let __length=*__output_offsets.add(__index as usize);
                        *__output_offsets.add(__index as usize)=__sum;
                        match __sum.checked_add(__length) {
                            Some(next)=>__sum=next,
                            None=>::cq_gpu_sequence_runtime::device::flag_overflow(__meta.add(2)),
                        }
                        __index+=1;
                    }
                    *__output_offsets.add(__count as usize)=__sum;
                    *__meta.add(3)=__sum;
                }
            }
        } else {
            TokenStream::new()
        };
        let shared = generator.shared_bytes.div_ceil(16) * 16;
        let shared_u32 =
            u32::try_from(shared).map_err(|_| error("Sequence shared memory exceeds u32"))?;
        kernels.push(quote! {
            #[kernel]
            #[launch_bounds(256)]
            #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(#shared_u32,#shared_u32))]
            pub fn #kernel_name(#(#parameters,)* __output_rows:*mut i64,__output_begin:*mut u32,
                __output_offsets:*mut u32,__output_keys:*mut u64,__capacity:u32,__meta:*mut u32)
            {
                unsafe {
                    let __tid=::cuda_device::thread::threadIdx_x() as usize;
                    let __shared=::cuda_device::DynamicSharedArray::<u8,16>::get_raw();
                    let __scratch=__shared.cast::<u32>();
                    let __reservation=__shared.add(32).cast::<u32>();
                    let __tile_slot=__shared.add(36).cast::<u32>();
                    let __abort_slot=__shared.add(40).cast::<u32>();
                    #(#bodies)*
                    #finalize
                }
            }
        });
        let store_rows = format_ident!("__cq_store_rows_{index}");
        let store_begin = format_ident!("__cq_store_begin_{index}");
        let store_offsets = format_ident!("__cq_store_offsets_{index}");
        let store_meta = format_ident!("__cq_store_meta_{index}");
        let store_length = format_ident!("__cq_store_length_{index}");
        let output_keys = format_ident!("__cq_store_keys_{index}");
        let meta_words = 4 + chains.len();
        let grid = if dispatch.parts.is_none() || virtual_range.is_some() || whole_reduction {
            1u32
        } else {
            2272u32
        };
        let initial = match (dispatch.concatenation.clone(), capacity) {
            (OutputProtocol::Count, _) => quote!(0usize),
            (OutputProtocol::Bounded, Some(capacity)) => quote!(#capacity),
            (OutputProtocol::Bounded, None) => {
                return Err(error("bounded dispatch requires its recorded capacity"));
            }
            (OutputProtocol::Reserve, Some(capacity)) => quote!(#capacity),
            (OutputProtocol::Reserve, None) => quote!(256usize),
        };
        let row_width = if packed_columns.is_some() { 0 } else { width };
        let range_width = usize::from(virtual_range.is_some());
        let key_width = usize::from(packed_columns.is_some());
        let logical = if virtual_range.is_some() {
            3usize
        } else {
            0usize
        };
        launches.push(quote! {
            let mut __cq_key_bits=0u32;
            let mut __cq_fields=::std::vec::Vec::<::cq_gpu_sequence_runtime::canon::PackedField>::new();
            #(#local_packing)*
            #(#host_domains)*
            let __cq_prepared=__cq_module.#prepare_name(::cuda_core::LaunchConfig1D::new(#grid,256,#shared_u32))?;
            let mut __cq_capacity=#initial;
            let (#store_rows,#store_begin,#store_offsets,#output_keys,#store_meta,#store_length)=loop {
                if __cq_capacity>u32::MAX as usize {return Err("Sequence output exceeds u32 positions".into());}
                let #store_rows=unsafe {::cq_gpu_runtime::Buffer::<i64>::uninitialized(__cq_pool.clone(),
                    __cq_capacity.checked_mul(#row_width).ok_or("Sequence row allocation overflow")?)?};
                let #store_begin=unsafe {::cq_gpu_runtime::Buffer::<u32>::uninitialized(__cq_pool.clone(),__cq_capacity*#range_width)?};
                let #store_offsets=unsafe {::cq_gpu_runtime::Buffer::<u32>::uninitialized(__cq_pool.clone(),
                    __cq_capacity.checked_add(1).ok_or("Sequence offset allocation overflow")?*#range_width)?};
                let #output_keys=unsafe {::cq_gpu_runtime::Buffer::<u64>::uninitialized(__cq_pool.clone(),__cq_capacity*#key_width)?};
                let #store_meta=::cq_gpu_runtime::Buffer::<u32>::zeroed(__cq_pool.clone(),#meta_words)?;
                __cq_module.#kernel_name(__cq_pool.stream(),&__cq_prepared,#(#arguments,)*
                    #store_rows.ptr(),#store_begin.ptr(),#store_offsets.ptr(),#output_keys.ptr(),
                    __cq_capacity as u32,#store_meta.ptr())?;
                let __cq_counts=#store_meta.to_host()?;
                if __cq_counts[2]!=0 {return Err("Sequence device count or fold overflow".into());}
                let __cq_required=__cq_counts[0] as usize;
                if __cq_required>__cq_capacity {
                    __cq_capacity=__cq_required;
                    continue;
                }
                break (#store_rows,#store_begin,#store_offsets,#output_keys,#store_meta,__cq_counts[#logical] as usize);
            };
        });
        if final_output {
            output = Some(if canonical {
                quote! {
                    let mut __cq_canon=::cq_gpu_sequence_runtime::canon::PackedCanon::new(&__cq_pool,#store_length)?;
                    let __cq_keys=unsafe {__cq_canon.canonicalize(&#output_keys,#store_length,__cq_key_bits)?};
                    let __cq_result=__cq_output_adapter.borrow_mut().unpack_columns(__cq_keys,&__cq_fields,__cq_dictionary,__cq_statuses)?;
                }
            } else {
                quote! {
                    let __cq_result=__cq_output_adapter.borrow_mut().packed_columns(
                        #output_keys,#store_length,&__cq_fields,__cq_dictionary,__cq_statuses)?;
                }
            });
        }
        if canonical && !final_output {
            let canonical_owner = format_ident!("__cq_canonical_{index}");
            let fields = format_ident!("__cq_fields_{index}");
            launches.push(quote! {
                let mut __cq_canon=::cq_gpu_sequence_runtime::canon::PackedCanon::new(&__cq_pool,#store_length)?;
                let #canonical_owner=unsafe {__cq_canon.canonicalize(&#output_keys,#store_length,__cq_key_bits)?};
                let #store_length=#canonical_owner.logical_count()?;
                let #fields=__cq_fields;
                drop(#output_keys);
            });
        }
        if !final_output {
            launches.push(compiler.rows.publish_provider(
                analysis,
                program,
                &chains[0].result,
                publication,
                index,
                if canonical {
                    packed_columns.as_deref()
                } else {
                    None
                },
            )?);
        }
        compiler.stored.push(Stored {
            name: publication.clone(),
            result: chains[0].result.clone(),
            range: virtual_range,
            packed: if canonical && !final_output {
                packed_columns
            } else {
                None
            },
        });
    }
    let output =
        output.ok_or_else(|| error("checked GPU output is not published by a dispatch"))?;
    let setup = &compiler.rows.setup;
    let build = &compiler.rows.build;
    syn::parse2(quote!({
        #[::cuda_device::cuda_module]
        mod #module_name {
            use ::cuda_device::{kernel,launch_bounds,launch_contract};
            #(#kernels)*
        }
        (|| -> ::cq_gpu_sequence_runtime::Result<_> {
            #setup
            ::std::thread_local! {
                static __CQ_SEQUENCE_MODULE: ::std::cell::RefCell<Option<(
                    ::std::sync::Arc<::cuda_core::CudaContext>,
                    #module_name::LoadedModule,
                )>> = const { ::std::cell::RefCell::new(None) };
            }
            let __cq_module=__CQ_SEQUENCE_MODULE.with(|cache| -> ::cq_gpu_runtime::Result<_> {
                let mut cache=cache.borrow_mut();
                if let Some((context,module))=cache.as_ref() {
                    if ::std::sync::Arc::ptr_eq(context,__cq_pool.context()) {
                        return Ok(module.clone());
                    }
                }
                let module=unsafe {#module_name::load(__cq_pool.context())?};
                *cache=Some((__cq_pool.context().clone(),module.clone()));
                Ok(module)
            })?;
            let __cq_timing_wall = ::std::time::Instant::now();
            let __cq_timing_start = __cq_pool.stream().record_event(
                Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
            #build
            #(#launches)*
            let __cq_timing_expanded=__cq_pool.stream().record_event(
                Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
            #output
            let __cq_timing_finished=__cq_pool.stream().record_event(
                Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
            __cq_pool.stream().synchronize()?;
            ::cq_gpu_runtime::publish_timings(::cq_gpu_runtime::Timings {
                build_ms:__cq_timing_start.elapsed_ms(&__cq_timing_built)? as f64,
                kernel_ms:__cq_timing_built.elapsed_ms(&__cq_timing_expanded)? as f64,
                dedup_ms:__cq_timing_expanded.elapsed_ms(&__cq_timing_finished)? as f64,
                total_ms:__cq_timing_start.elapsed_ms(&__cq_timing_finished)? as f64,
                wall_ms:__cq_timing_wall.elapsed().as_secs_f64()*1000.0,
                ..::cq_gpu_runtime::Timings::default()
            });
            Ok(__cq_result)
        })().expect("GPU Sequence execution failed")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduction_templates_keep_the_recorded_scope_and_wide_results() {
        for ty in [syn::parse_quote!(i64), syn::parse_quote!(u32)] {
            let ty: syn::Type = ty;
            let program: Program = syn::parse_quote! {
                physical {
                    relation R(a:#ty) = (&data.r);
                    r = pipe canon rename R@0 {a -> a};
                    return r as answer(a);
                }
                structures { r host scan = Reader []; r canon = Canon; }
                sequence { input = store canon rows r; }
                graded gpu { input at host; }
            };
            let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
            let compiler = Compiler {
                program: &program,
                analysis: &analysis,
                rows: Rows::new(&program, &analysis).unwrap(),
                stored: Vec::new(),
                definitions: BTreeMap::new(),
            };
            for monoid in [Monoid::Count, Monoid::Sum(syn::parse_quote!(a))] {
                for tiled in [false, true] {
                    let mut operations = Vec::new();
                    if tiled {
                        operations.push(Operation::Tile { n: 1024, items: 4 });
                    }
                    operations.push(Operation::Fold(monoid.clone()));
                    let mut kernel = Kernel {
                        compiler: &compiler,
                        shared_bytes: 48,
                        virtual_range: None,
                        packed_columns: Some(vec!["a".into()]),
                        publication_columns: BTreeSet::new(),
                        sink: None,
                        scalar_output: false,
                    };
                    let (declare, body, finish, whole) = kernel
                        .partition_rows(
                            &operations,
                            true,
                            &[],
                            StripeInput {
                                items: 4,
                                inherited: DeviceRow::default(),
                                prelude: TokenStream::new(),
                                load: compiler
                                    .rows
                                    .source("r", quote!(__position), quote!(__row))
                                    .unwrap(),
                            },
                        )
                        .unwrap();
                    let tokens = quote!({#declare #body #finish});
                    syn::parse2::<syn::Block>(tokens.clone()).unwrap();
                    let generated = tokens.to_string();
                    assert_eq!(whole, !tiled);
                    assert!(generated.contains("__cq_fold_accumulator_"));
                    assert!(generated.contains("__fold_result"));
                    assert!(!generated.contains("__row"));
                    assert!(!generated.contains("__fold_result as u32"));
                    if matches!(monoid, Monoid::Sum(_)) {
                        let input_type = if compiler.rows.dense.contains(&0) {
                            "u32"
                        } else {
                            "i64"
                        };
                        assert!(generated.contains(&format!("let mut __code_0 : {input_type}")));
                    }
                }
            }
        }
    }
}
