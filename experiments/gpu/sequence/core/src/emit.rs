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


fn prune_kernel_parameters(
    kernels: &mut [TokenStream],
    launches: &mut [TokenStream],
    rust_functions: &[(syn::Ident, TokenStream, syn::Ident)],
) -> syn::Result<()> {
    use syn::visit::Visit;
    use syn::visit_mut::VisitMut;
    struct Uses(BTreeSet<String>);
    impl<'ast> Visit<'ast> for Uses {
        fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
            if let Some(name) = path.path.get_ident() {
                self.0.insert(name.to_string());
            }
            syn::visit::visit_expr_path(self, path);
        }
    }
    let mut live = BTreeMap::new();
    let mut preparations = BTreeMap::new();
    for tokens in kernels {
        let mut kernel: syn::ItemFn = syn::parse2(tokens.clone())?;
        let mut uses = Uses(BTreeSet::new());
        uses.visit_block(&kernel.block);
        let mut keep = Vec::new();
        let mut layouts = Vec::new();
        let mut functions = BTreeSet::new();
        let mut inputs = syn::punctuated::Punctuated::new();
        for input in kernel.sig.inputs {
            let syn::FnArg::Typed(argument) = &input else {
                return Err(error("generated kernel has a receiver"));
            };
            let syn::Pat::Ident(name) = &*argument.pat else {
                return Err(error("generated kernel parameter is not an identifier"));
            };
            let used = uses.0.contains(&name.ident.to_string());
            keep.push(used);
            if used {
                let ty = &argument.ty;
                layouts.push(quote! {
                    bytes = bytes.next_multiple_of(::core::mem::align_of::<#ty>())
                        + ::core::mem::size_of::<#ty>();
                });
                if let syn::Type::Path(path) = ty.as_ref() {
                    if let Some(name) = path.path.get_ident() {
                        functions.insert(name.to_string());
                    }
                }
                inputs.push(input);
            }
        }
        let mut witnesses = Vec::new();
        for (ty, bound, argument) in rust_functions {
            if functions.contains(&ty.to_string()) {
                kernel.sig.generics.params.push(syn::parse2(quote!(#ty: Copy + #bound))?);
                witnesses.push(argument.clone());
            }
        }
        if !witnesses.is_empty() {
            preparations.insert(format!("prepare_{}", kernel.sig.ident), witnesses);
        }
        kernel.block.stmts.insert(0, syn::parse_quote! {
            const {
                let mut bytes = 0usize;
                #(#layouts)*
                assert!(bytes < 4096, "BE-Resource: kernel parameter area must be below 4096 bytes");
            }
        });
        live.insert(kernel.sig.ident.to_string(), keep);
        kernel.sig.inputs = inputs;
        *tokens = quote!(#kernel);
    }
    struct Launches<'a> {
        live: &'a BTreeMap<String, Vec<bool>>,
        preparations: &'a BTreeMap<String, Vec<syn::Ident>>,
    }
    impl VisitMut for Launches<'_> {
        fn visit_expr_method_call_mut(&mut self, call: &mut syn::ExprMethodCall) {
            syn::visit_mut::visit_expr_method_call_mut(self, call);
            if let Some(witnesses) = self.preparations.get(&call.method.to_string()) {
                call.method = format_ident!("{}_for", call.method);
                let mut arguments = syn::punctuated::Punctuated::new();
                for witness in witnesses {
                    arguments.push(syn::parse_quote!(&#witness));
                }
                arguments.extend(std::mem::take(&mut call.args));
                call.args = arguments;
            }
            if let Some(keep) = self.live.get(&call.method.to_string()) {
                // The loaded-module ABI prepends stream and prepared launch.
                call.args = std::mem::take(&mut call.args)
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, arg)| (index < 2 || keep[index - 2]).then_some(arg))
                    .collect();
            }
        }
    }
    for tokens in launches {
        let mut block: syn::Block = syn::parse2(quote!({#tokens}))?;
        Launches { live: &live, preparations: &preparations }.visit_block_mut(&mut block);
        let statements = block.stmts;
        *tokens = quote!(#(#statements)*);
    }
    Ok(())
}

#[derive(Clone)]
enum Operation {
    Map(String, String),
    Lookup {
        provider: String,
        keys: Vec<String>,
        membership: bool,
        negated: bool,
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
                let mut chains = self.fold(
                    self.definitions
                        .get(&name)
                        .ok_or_else(|| error("Sequence reference lacks definition"))?,
                )?;
                self.rehead(&name, &mut chains);
                Ok(chains)
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
            Term::Compact(name, inner) if matches!(inner.as_ref(), Term::Lookup { .. }) => {
                // Compact's checked Physical operator consumes existence, even
                // when the same provider also supplies payloads to another read.
                // The provider-wide capability is not this constructor's meaning.
                let Term::Lookup {
                    provider,
                    keys,
                    inner,
                } = inner.as_ref()
                else {
                    unreachable!()
                };
                let result = symbol_name(name);
                let definition = self
                    .program
                    .structures
                    .physical
                    .program
                    .definitions
                    .iter()
                    .find(|definition| symbol_name(&definition.name) == result)
                    .ok_or_else(|| error("compact lacks checked Physical row provenance"))?;
                let negated = matches!(definition.operator, Operator::Anti(_, _));
                let mut keys = keys.iter().map(symbol_name).collect::<Vec<_>>();
                keys.sort();
                let mut chains = self.fold(inner)?;
                for chain in &mut chains {
                    chain.operations.push(Operation::Lookup {
                        provider: self.rows.provider_name(&symbol_name(provider), &keys)?,
                        keys: keys.clone(),
                        membership: true,
                        negated,
                    });
                    chain.result = result.clone();
                }
                Ok(chains)
            }
            Term::Group(_, _) => Err(error(
                "T-Gpu: group requires an explicit host publication boundary",
            )),
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
                        provider: self.rows.provider_name(&provider, &keys)?,
                        keys: keys.clone(),
                        membership,
                        negated: false,
                    });
                }
                Ok(chains)
            }
            Term::PartitionK { .. } => {
                Err(error("T-Gpu: key partition requires a routed-pool target"))
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

    /// All device carriers use the same attribute-indexed slots. The checker
    /// may give equivalent choice arms one named result; interpreting that
    /// provenance changes no slots and must not re-evaluate its Physical union.
    fn rehead(&self, name: &str, chains: &mut [Chain]) {
        if let Some(result) = self.analysis.results.get(name) {
            for chain in chains {
                chain.result.clone_from(result);
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
                } else if self.analysis.stores.contains_key(name)
                    && self
                        .analysis
                        .levels
                        .get(name)
                        .is_some_and(|level| level.last().is_some_and(|level| level == "host"))
                {
                    // Host stores have the same checked Physical provenance as
                    // their uploaded source, but need not share its spelling.
                    // Read the published count, not a pre-publication estimate.
                    let result = self
                        .analysis
                        .results
                        .get(name)
                        .ok_or_else(|| error("host store lacks checked row provenance"))?;
                    self.rows.source_len(result)?
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
            Guard::WorkersLt(_) => {
                return Err(error(
                    "GPU Sequence does not support target-wide worker guards",
                ));
            }
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
        if self.rows.run_provider(provider) {
            if order.len() < keys.len() || order[..keys.len()].iter().any(|key| !keys.contains(key)) {
                return Err(error("GPU-Runs: lookup keys are not a checked leading prefix"));
            }
            let run = format_ident!("__cq_run_{provider}");
            let parts = order.iter().take(keys.len()).map(|key| {
                let slot = self.slot(key)?;
                Ok(quote!(__row[#slot]))
            }).collect::<syn::Result<Vec<_>>>()?;
            let count = keys.len() as u32;
            let prefix = quote!(let __prefix: [i64; #count as usize] = [#(#parts),*];);
            return Ok(if membership {
                quote! {
                    #prefix
                    let mut __member_found = false;
                    if __live {
                        for #run in 0..#view.run_count {
                            // DECODE V: P4 run membership prefix_range reconstructs packed signed fields through compare_prefix solely for key comparison.
                            let (__begin, __end) = ::gpu_datalog_runtime::device::prefix_range(#view, #run, __prefix.as_ptr(), #count);
                            __member_found |= __begin < __end;
                        }
                    }
                }
            } else {
                quote! {
                    #prefix
                    let (__begin, __end) = if __live {
                        // DECODE V: P4 run lookup prefix_range reconstructs packed signed fields through compare_prefix instead of comparing encoded keys.
                        ::gpu_datalog_runtime::device::prefix_range(#view, #run, __prefix.as_ptr(), #count)
                    } else { (0u32, 0u32) };
                }
            });
        }
        if order.is_empty() && keys.is_empty() {
            return Ok(quote! {
                let (mut __begin, mut __end) = (0u32, #view.len);
                let __member_found = __begin < __end;
            });
        }
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
                    __member_found = ::cq_gpu_sequence_runtime::device::contains_u32(
                        *#view.payload.add(#index),__begin,__end,__row[#slot] as u32);
                });
            } else {
                membership_checks.extend(quote! {
                    let __key=__row[#slot] as i64;
                    let __column=*#view.payload.add(#index);
                    let (mut __low,mut __high)=(__begin,__end);
                    while __low<__high {
                        let __middle=__low+(__high-__low)/2;
                        // DECODE V: P4 lower-bound key search reconstructs FOR values solely for ordered relational comparison.
                        if ::cq_gpu_sequence_runtime::device::decode(__column,__middle as u64)<__key {
                            __low=__middle+1;
                        } else {__high=__middle;}
                    }
                    __begin=__low;
                    __high=__end;
                    while __low<__high {
                        let __middle=__low+(__high-__low)/2;
                        // DECODE V: P4 upper-bound key search reconstructs FOR values solely for ordered relational comparison.
                        if ::cq_gpu_sequence_runtime::device::decode(__column,__middle as u64)<=__key {
                            __low=__middle+1;
                        } else {__high=__middle;}
                    }
                    __end=__low;
                });
            }
        }
        Ok(quote! {
            let mut __member_found = false;
            let (mut __begin, mut __end) = (0u32, 0u32);
            if __live && (__row[#slot] as i64) >= 0 && (__row[#slot] as u64) < #view.domain as u64 {
                (__begin, __end) = ::cq_gpu_sequence_runtime::device::range(#view.offsets, __row[#slot] as u32);
                __member_found = __begin < __end;
            }
            #membership_checks
        })
    }

    fn payload(&self, provider: &str, position: TokenStream) -> syn::Result<TokenStream> {
        let (view, payload, _) = self.rows.provider(provider)?;
        if self.rows.run_provider(provider) {
            let run = format_ident!("__cq_run_{provider}");
            // DECODE V: P4 run-provider payload unpack widens keys and code-only fields before relational consumers.
            let assignments = payload.iter().enumerate().map(|(index,column)| {
                let slot = self.slot(column)?;
                Ok(quote! {
                    __row[#slot] = ::gpu_datalog_runtime::device::decode(
                        #view, #run, (#position) as u32, #index as u32);
                })
            }).collect::<syn::Result<Vec<_>>>()?;
            return Ok(quote!(#(#assignments)*));
        }
        // DECODE V: P6 provider payload reconstructs FOR values without restricting decode to rust_reads columns.
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
    shared: crate::layout::SharedLayout,
    virtual_range: Option<String>,
    packed_columns: Option<Vec<String>>,
    arena_packing: Option<&'c crate::arena::Packing>,
    publication_columns: BTreeSet<usize>,
    sink: Option<TokenStream>,
    scalar_output: bool,
    combine: Option<TokenStream>,
}

struct StripeInput {
    items: usize,
    inherited: DeviceRow,
    prelude: TokenStream,
    load: TokenStream,
}

impl Kernel<'_, '_> {
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
                Operation::Map(name, input) => {
                    // Backward liveness: a slot the map writes is not live
                    // before it; only the map's other reads are.
                    let map = self
                        .compiler
                        .rows
                        .map(name, input, quote!(__row), quote!(__live))?;
                    let writes = DeviceRow::writes(map.clone())?;
                    live.retain(|slot| !writes.contains(slot));
                    live.extend(DeviceRow::uses(map)?.difference(&writes));
                }
                Operation::Lookup {
                    provider,
                    keys,
                    membership,
                    ..
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
                let map = self
                    .compiler
                    .rows
                    .map(name, input, quote!(__row), quote!(__live))?;
                // A child may read parent-owned shared codes, but its computed
                // bindings must be lane-local: siblings can share one owner.
                let mut local = row.clone();
                let mut declarations = TokenStream::new();
                for slot in DeviceRow::writes(map.clone())? {
                    let id = format_ident!("__map_code_{slot}");
                    let ty = self.code_type(slot);
                    declarations.extend(quote!(let mut #id: #ty = 0;));
                    local.0.insert(slot, quote!(#id));
                }
                let map = local.rewrite(map)?;
                let rest = self.scalar_operations(rest, &local)?;
                Ok(quote! { { #declarations #map #rest } })
            }
            Operation::Lookup {
                provider,
                keys,
                membership,
                negated,
            } => {
                let lookup = row.rewrite(self.compiler.lookup(provider, keys, *membership)?)?;
                if *membership {
                    let rest = self.scalar_operations(rest, row)?;
                    // Formal rule E-Neg: anti membership keeps precisely the rows
                    // whose provider probe finds no witness; dead lanes stay dead.
                    return Ok(
                        quote!(#lookup __live = __live && (__member_found != #negated); #rest),
                    );
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
                // BE-Resource: a key still live after the lookup is carried as
                // a parent, even where the provider row also stores it.
                let key_slots = keys
                    .iter()
                    .map(|column| self.compiler.slot(column))
                    .collect::<syn::Result<BTreeSet<_>>>()?;
                let payload_columns = self
                    .compiler
                    .rows
                    .provider(provider)?
                    .1
                    .iter()
                    .map(|column| self.compiler.slot(column))
                    .collect::<syn::Result<BTreeSet<_>>>()?
                    .difference(&key_slots)
                    .copied()
                    .collect::<BTreeSet<_>>();
                let (begin_offset, offsets_offset) = self.shared.payload()?;
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
                    let at = self.shared.parent(bytes)?;
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
                let expansion = quote! {
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
                };
                if self.compiler.rows.run_provider(provider) {
                    let view = self.compiler.rows.provider(provider)?.0;
                    let run = format_ident!("__cq_run_{provider}");
                    // Run bounds are uniform descriptor metadata: every lane
                    // executes every run's existing segmented collective.
                    Ok(quote! { for #run in 0..#view.run_count { #expansion } })
                } else {
                    Ok(expansion)
                }
            }
            Operation::Fold(_) => Err(error("scalar row template cannot cross a reduction scope")),
            _ => self.scalar_operations(rest, row),
        }
    }

    /// Formal rule GPU-FoldCompletion: whole-input folds emit block partials;
    /// the recorded combine step alone publishes the logical fold result.
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
        let offset = self.shared.fold()?;
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
        let publish = quote! {
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
            let declare = &reduction.declare;
            let combine_value = if matches!(monoid, Monoid::Or) {
                quote!(#accumulator |= *__partials.add(__partial_base + __part);)
            } else {
                quote! {
                    match #accumulator.checked_add(*__partials.add(__partial_base + __part)) {
                        Some(value) => #accumulator = value,
                        None => ::cq_gpu_sequence_runtime::device::flag_overflow(__meta.add(2)),
                    }
                }
            };
            self.combine = Some(quote! {
                #declare
                let mut __part = __tid;
                while __part < __blocks as usize {
                    #combine_value
                    __part += 256;
                }
                #publish
            });
            let partial = quote! {
                let mut __fold_result = 0i64;
                let mut __live = false;
                #finish
                if __live {
                    *__partials.add(__partial_base + ::cuda_device::thread::blockIdx_x() as usize) = __fold_result;
                }
            };
            Ok((reduction.declare, nested_partitions(tiles, body), partial, true))
        } else {
            Ok((
                TokenStream::new(),
                partition_body(tiles, scope, reduction.declare, body, publish),
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
        if let Some(packing) = self.arena_packing {
            let slots = self.compiler.rows.output_columns.iter()
                .map(|column| self.compiler.slot(column))
                .collect::<syn::Result<Vec<_>>>()?;
            return Ok(packing.write(&slots));
        }
        let width = self.compiler.rows.width;
        let write = if let Some(columns) = &self.packed_columns {
            let parts = columns
                .iter()
                .enumerate()
                .map(|(i, column)| {
                    let slot = self.compiler.slot(column)?;
                    let base = format_ident!("__cq_pack_base_{i}");
                    let shift = format_ident!("__cq_pack_shift_{i}");
                    let value = if self.compiler.rows.dense.contains(&slot) && !self.scalar_output {
                        quote!((__row[#slot] as u32).wrapping_sub(#base as u32) as u64)
                    } else {
                        quote!(__row[#slot].wrapping_sub(#base) as u64)
                    };
                    Ok(quote! {
                        let value = #value;
                        let lane = (#shift / 64) as usize;
                        let local = #shift % 64;
                        if lane < __cq_key_lanes {
                            *__packed.add(lane) |= value << local;
                            if local != 0 && lane + 1 < __cq_key_lanes {
                                *__packed.add(lane + 1) |= value >> (64 - local);
                            }
                        }
                    })
                })
                .collect::<syn::Result<Vec<_>>>()?;
            quote! {
                let __packed = __output_keys.add(__destination as usize * __cq_key_lanes);
                for lane in 0..__cq_key_lanes { *__packed.add(lane) = 0; }
                #(#parts)*
            }
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

fn host_union<'a>(
    term: &'a Term,
    definitions: &BTreeMap<String, &'a Term>,
) -> Option<(&'a [Term], bool)> {
    match term {
        Term::Union(arms) => Some((arms, false)),
        Term::Store { inner, .. } => host_union(inner, definitions),
        Term::Canon(inner) => host_union(inner, definitions).map(|(arms, _)| (arms, true)),
        Term::Ref(name) => host_union(definitions.get(&symbol_name(name))?, definitions),
        _ => None,
    }
}

fn grouped_boundary<'a>(
    term: &'a Term,
    definitions: &BTreeMap<String, &'a Term>,
) -> Option<(&'a syn::Ident, &'a Term)> {
    match term {
        Term::Group(name, input) => Some((name, input)),
        Term::Store { inner, .. } | Term::Canon(inner) => grouped_boundary(inner, definitions),
        Term::Ref(name) => grouped_boundary(definitions.get(&symbol_name(name))?, definitions),
        _ => None,
    }
}

pub(crate) fn packed_offset(
    keys: TokenStream,
    row: TokenStream,
    lanes: TokenStream,
    shift: TokenStream,
    bits: TokenStream,
) -> TokenStream {
    quote! {{
        if #bits == 0 { 0u64 } else {
            let index = (#row) * (#lanes) + (#shift / 64) as usize;
            let local = #shift % 64;
            let mut value = unsafe { *(#keys).add(index) } >> local;
            if local + #bits > 64 {
                value |= unsafe { *(#keys).add(index + 1) } << (64 - local);
            }
            value & if #bits == 64 { u64::MAX } else { (1u64 << #bits) - 1 }
        }
    }}
}

fn canon_type<'a>(analysis: &'a Analysis, publication: &str) -> syn::Result<&'a syn::Type> {
    analysis
        .obligations
        .iter()
        .find_map(|obligation| match obligation {
            cq_lang_sequence::Obligation::Canon {
                result,
                ty: Some(ty),
            } if result == publication => Some(ty),
            _ => None,
        })
        .or_else(|| {
            analysis
                .results
                .get(publication)
                .and_then(|result| analysis.structures.dedup.get(result))
        })
        .ok_or_else(|| error("GPU canon publication requires its recorded provider type"))
}

fn publish_host(
    compiler: &mut Compiler<'_>,
    result: &str,
    host_downloads: &mut BTreeMap<usize, syn::Ident>,
    launches: &mut Vec<TokenStream>,
    codecs: &crate::runs::Codecs,
) -> syn::Result<()> {
    let program = compiler.program;
    let analysis = compiler.analysis;
                let bindings: BTreeMap<_, _> = compiler
                    .stored
                    .iter()
                    .enumerate()
                    .map(|(prior, stored)| {
                        (
                            stored.result.clone(),
                            format_ident!("__cq_downloaded_{prior}"),
                        )
                    })
                    .collect();
                let dependencies = compiler.rows.bound_host_dependencies(result, &bindings);
                for (prior, stored) in compiler.stored.iter().enumerate() {
                    if !dependencies.contains(&stored.result)
                        || bindings[&stored.result] != format_ident!("__cq_downloaded_{prior}")
                    {
                        continue;
                    }
                    if stored.range.is_some() {
                        return Err(error(
                            "host consumer requires a materialized row publication",
                        ));
                    }
                    let downloaded = format_ident!("__cq_downloaded_{prior}");
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        host_downloads.entry(prior)
                    {
                        let length = format_ident!("__cq_store_length_{prior}");
                        let rows = format_ident!("__cq_store_rows_{prior}");
                        let owner = format_ident!("__cq_canonical_{prior}");
                        let fields = format_ident!("__cq_fields_{prior}");
                        let mut values = Vec::new();
                        let mut types = Vec::new();
                        for column in &analysis.structures.layouts[&stored.result] {
                            let ty =
                                &analysis.structures.physical.rows[&stored.result].types[column];
                            types.push(ty);
                            let code = if let Some(columns) = &stored.packed {
                                let field = columns
                                    .iter()
                                    .position(|name| name == column)
                                    .ok_or_else(|| {
                                        error("host consumer column absent from packed publication")
                                    })?;
                                let offset = packed_offset(
                                    quote!(__cq_downloaded_codes.as_ptr()),
                                    quote!(__position),
                                    quote!(__lanes),
                                    quote!(field.shift),
                                    quote!(field.bits),
                                );
                                // DECODE V: P6 intermediate host publication reconstructs packed FOR fields for the whole stored layout.
                                quote! {{
                                    let field = #fields[#field];
                                    let offset = #offset;
                                    field.base.wrapping_add(offset as i64)
                                }}
                            } else {
                                let width = compiler.rows.width;
                                let slot = compiler.slot(column)?;
                                quote!(__cq_downloaded_codes[__position * #width + #slot])
                            };
                            // DECODE V: P6 intermediate host fallback resolves symbols and logical values before more engine operators, not final answers.
                            values.push(if let Some(codec) = codecs.get(ty) {
                                quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::decode(
                                    u32::try_from(#code).map_err(|_| "GPU symbol code exceeds u32")?
                                ))
                            } else {
                                quote!(::cq_gpu_runtime::decode_value::<#ty>(#code, &__cq_dictionary)?)
                            });
                        }
                        let (download, key) = if stored.packed.is_some() {
                            (
                                quote!(#owner.keys().to_host()?),
                                quote! {
                                    let __lanes = #owner.lanes();
                                },
                            )
                        } else {
                            (quote!(#rows.to_host()?), TokenStream::new())
                        };
                        launches.push(quote! {
                            let #downloaded: ::std::vec::Vec<(#(#types,)*)> = {
                                let __cq_downloaded_codes = #download;
                                let mut rows = ::std::vec::Vec::with_capacity(#length);
                                for __position in 0..#length {
                                    #key
                                    rows.push((#(#values,)*));
                                }
                                rows
                            };
                        });
                        entry.insert(downloaded.clone());
                    }
                }
                launches.push(
                    compiler
                        .rows
                        .publish_host(program, analysis, result, &bindings)?,
                );
    Ok(())
}

// DESIGN translator-gpu
// Owns only translation of checked Sequence facts (translator-decides-nothing, T-Gpu).
// Source: doc/PLANNING-REDESIGN.md Architecture; doc/formal/CONFORMANCE.md GPU emitter rows.
// Launch, scan, fold completion, capture and publication facts must be recorded before printing.
// DO NOT replan joins, move predicates, guess from generated names, or select codecs/memory policy.
// Known deviation: terminal Group rerouting still bypasses an authored store (CONFORMANCE).
// Removing that unrecorded grouped refinement belongs to remaining work in Plan P8.
/// Translate only checked scheduling facts; no planner is consulted here.
// Formal rule T-Gpu: Interpret checked Sequence constructors at their recorded GPU grades.
pub fn translate(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    translate_impl(program, analysis, None)
}

pub(crate) fn translate_impl(
    program: &Program,
    analysis: &Analysis,
    arena: Option<&syn::Expr>,
) -> syn::Result<syn::Expr> {
    if program
        .graded
        .as_ref()
        .is_none_or(|graded| graded.machine != "gpu")
    {
        return Err(error(
            "GPU Sequence translation requires checked gpu grading",
        ));
    }
    if arena.is_none()
        && program.sequence.output == Some(OutputProtocol::ReservePrevious)
    {
        return Err(error("reserve_previous requires the GPU arena terminal"));
    }
    for (_, _, level) in &analysis.collectives {
        if level.last().is_some_and(|level| level == "device") {
            return Err(error(
                "GPU Sequence fold requires a collaborative block or sequential lane, not device",
            ));
        }
    }
    if let Some(directory) = std::env::var_os("CQ_GPU_SEQUENCE_DUMP") {
        // Dump before row translation so an unrealizable program is still visible.
        let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&quote!(#program).to_string(), &mut fingerprint);
        let path = std::path::PathBuf::from(directory)
            .join(format!("{:016x}.input.sequence", std::hash::Hasher::finish(&fingerprint)));
        std::fs::write(&path, quote!(#program).to_string())
            .map_err(|err| error(format!("cannot dump Sequence program to {}: {err}", path.display())))?;
    }
    let rows = Rows::new(program, analysis)?;
    let device_sources = crate::rows::device_source_results(analysis);
    let codecs = crate::runs::Codecs::check(&program.structures)?;
    let arena_packing = arena.map(|_| rows.output_types().and_then(crate::arena::Packing::new)).transpose()?;
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
    let mut host_downloads = BTreeMap::<usize, syn::Ident>::new();
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
    let final_group = compiler
        .definitions
        .get(final_name)
        .and_then(|term| grouped_boundary(term, &compiler.definitions));
    if arena.is_some() && (final_group.is_some() || final_fold.is_some()) {
        return Err(error("GPU arena terminal requires raw candidate rows, not a group or fold"));
    }
    let mut packing = Vec::new();
    for (i, column) in compiler
        .rows
        .output_columns
        .iter()
        .enumerate()
        .filter(|_| final_group.is_none() && arena.is_none())
    {
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
            __cq_fields.push(::cq_gpu_sequence_runtime::canon::PackedField {base:#base,shift:#shift,bits:__cq_width});
        });
    }
    // CUDA exports are not scoped by the surrounding Rust expression. Give
    // each expansion its own module, including repeated identical programs.
    static NEXT_MODULE: AtomicUsize = AtomicUsize::new(0);
    let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
    quote!(#program).to_string().hash(&mut fingerprint);
    let expansion = NEXT_MODULE.fetch_add(1, Ordering::Relaxed);
    let module_name = format_ident!("__cq_sequence_kernels_{:016x}_{}", fingerprint.finish(), expansion);
    // Host LLVM inlines a once-called closure into its caller; one program of
    // many holes then becomes one function whose register allocation dominates
    // the build. A non-inlined runner keeps each hole a separate function, and
    // its module selects a separate codegen unit.
    let partition = format_ident!("partition_{}", expansion % 16);
    if let Some(directory) = std::env::var_os("CQ_GPU_SEQUENCE_DUMP") {
        let path = std::path::PathBuf::from(directory).join(format!("{module_name}.sequence"));
        std::fs::write(&path, quote!(#program).to_string())
            .map_err(|err| error(format!("cannot dump Sequence program to {}: {err}", path.display())))?;
    }
    for definition in &program.sequence.definitions {
        let publication = symbol_name(&definition.name);
        // Lowering can give the terminal group its own store, followed by a
        // final store alias. Both name the same grouped publication, which is
        // realized by the collective below, not a CPU group/download/reupload.
        // Only transparent wrappers around that terminal group are deferred;
        // ordinary host publications still follow their recorded boundaries.
        if publication != final_name
            && final_group.is_some_and(|(final_group, _)| {
                grouped_boundary(&definition.term, &compiler.definitions)
                    .is_some_and(|(group, _)| group == final_group)
            })
        {
            continue;
        }
        if publication == final_name
            && let Some((name, input)) = grouped_boundary(&definition.term, &compiler.definitions)
        {
            let mut chains = compiler.fold(input)?;
            let [chain] = chains.as_mut_slice() else {
                return Err(error(
                    "GPU group requires one canonical valuation publication",
                ));
            };
            let result = symbol_name(name);
            let physical = program
                .structures
                .physical
                .program
                .definitions
                .iter()
                .find(|definition| symbol_name(&definition.name) == result)
                .ok_or_else(|| error("GPU group lacks checked Physical provenance"))?;
            let Operator::Group(group) = &physical.operator else {
                return Err(error("GPU group provenance is not a Physical group"));
            };
            let mut host_input = chain.stored.is_none() && !compiler.rows.has_source(&chain.source);
            for operation in &chain.operations {
                let Operation::Map(name, input) = operation else { continue };
                for name in compiler.rows.capture_definitions(name, input)? {
                    let definition = program.structures.physical.program.definitions.iter()
                        .find(|definition| symbol_name(&definition.name) == name)
                        .ok_or_else(|| error("GPU group input lacks checked Physical provenance"))?;
                    host_input |= analysis.rust_requires_host(&definition.operator);
                }
            }
            if host_input {
                // The host-graded input is evaluated at its recorded boundary,
                // using published device operands rather than replaying joins.
                // Only the numeric grouped-fold representation runs on device.
                let input = symbol_name(&group.input);
                if !compiler.rows.has_source(&input) {
                    publish_host(&mut compiler, &input, &mut host_downloads, &mut launches, &codecs)?;
                }
                chain.source = input.clone();
                chain.result = input;
                chain.stored = None;
                chain.operations.clear();
            }
            let canonical = chain
                .stored
                .and_then(|index| (chain.operations.is_empty()).then_some(index))
                .and_then(|index| {
                    compiler.stored[index]
                        .packed
                        .as_ref()
                        .map(|columns| (index, columns))
                });
            let direct = if canonical.is_none() {
                let mut load = TokenStream::new();
                let length = if let Some(index) = chain.stored {
                    let stored = &compiler.stored[index];
                    if stored.packed.is_some() || stored.range.is_some() {
                        return Err(error(
                            "GPU group row chain needs a flat resident publication",
                        ));
                    }
                    let rows = format_ident!("__cq_store_rows_{index}");
                    let width = compiler.rows.width;
                    for column in &analysis.structures.physical.headings[&stored.result] {
                        let slot = compiler.slot(column)?;
                        load.extend(quote!(__row[#slot] = *#rows.add(position * #width + #slot);));
                    }
                    let length = format_ident!("__cq_store_length_{index}");
                    quote!(#length)
                } else {
                    load.extend(compiler.rows.source(
                        &chain.source,
                        quote!(position),
                        quote!(__row),
                    )?);
                    compiler.rows.source_len(&chain.source)?
                };
                let mut used = BTreeSet::new();
                for key in &group.keys {
                    used.insert(compiler.slot(&symbol_name(key))?);
                }
                for aggregate in &group.aggregates {
                    if let Some(expression) = &aggregate.expression
                        && let Some(name) = cq_lang_structures::syntax::rust::variable(expression)
                    {
                        used.insert(compiler.slot(&symbol_name(name))?);
                    }
                }
                for operation in &chain.operations {
                    match operation {
                        Operation::Map(name, input) => {
                            let code =
                                compiler
                                    .rows
                                    .map(name, input, quote!(__row), quote!(__live))?;
                            used.extend(DeviceRow::uses(code.clone())?);
                            load.extend(code);
                        }
                        Operation::Store | Operation::Flatten => {}
                        _ => {
                            return Err(error(
                                "GPU group row chain requires a checked flat publication before expansion",
                            ));
                        }
                    }
                }
                let mut row = DeviceRow::default();
                let mut declarations = TokenStream::new();
                for slot in used {
                    let id = format_ident!("__group_code_{slot}");
                    declarations.extend(quote!(let mut #id = 0i64;));
                    row.0.insert(slot, quote!(#id));
                }
                let load = row.rewrite(load)?;
                let mut parameters: Vec<_> = compiler
                    .rows
                    .parameters
                    .iter()
                    .map(|(name, ty, _)| quote!(#name: #ty))
                    .collect();
                let mut capture_definitions = Vec::new();
                for operation in &chain.operations {
                    if let Operation::Map(name, input) = operation {
                        capture_definitions.extend(compiler.rows.capture_definitions(name, input)?);
                    }
                }
                let mut arguments = compiler.rows.capture_arguments(&[
                    (capture_definitions, quote!((#length) != 0))
                ]);
                if let Some(index) = chain.stored {
                    let rows = format_ident!("__cq_store_rows_{index}");
                    parameters.push(quote!(#rows: *const i64));
                    arguments.push(quote!(#rows.ptr()));
                }
                Some(crate::reduce::GroupInput {
                    parameters,
                    arguments,
                    length,
                    load: quote!(#declarations #load),
                    columns: compiler
                        .rows
                        .columns
                        .iter()
                        .filter_map(|(name, slot)| {
                            row.0.get(slot).map(|value| (name.clone(), value.clone()))
                        })
                        .collect(),
                })
            } else {
                None
            };
            let (prior, columns) = canonical
                .map(|(index, columns)| (index, columns.as_slice()))
                .unwrap_or((0, &[]));
            let kernel = format_ident!("__cq_group_arguments_{module_name}");
            let (code, publication) = crate::reduce::group(
                group,
                columns,
                &compiler.rows.output_columns,
                &analysis.structures.physical.rows[&result].types,
                prior,
                &kernel,
                direct,
                &codecs,
            )?;
            kernels.push(code);
            output = Some(publication);
            continue;
        }
        let dispatch = analysis.dispatches.iter().find(|dispatch| {
            dispatch.inner_stores.iter().rev().find(|name| {
                compiler
                    .definitions
                    .get(*name)
                    .is_some_and(|term| matches!(term, Term::Store { .. }))
            }) == Some(&publication)
        });
        let index = compiler.stored.len();
        let Some(dispatch) = dispatch else {
            if arena.is_some() && publication == final_name {
                return Err(error("GPU arena terminal requires a recorded device row dispatch"));
            }
            if analysis
                .levels
                .get(&publication)
                .is_some_and(|level| level.last().is_some_and(|level| level == "host"))
                && matches!(definition.term, Term::Store { .. })
                && let Some((arms, canonical)) = host_union(&definition.term, &compiler.definitions)
            {
                let result = &analysis.results[&publication];
                let width = compiler.rows.width;
                let store_rows = format_ident!("__cq_store_rows_{index}");
                let store_length = format_ident!("__cq_store_length_{index}");
                let store_begin = format_ident!("__cq_store_begin_{index}");
                let store_offsets = format_ident!("__cq_store_offsets_{index}");
                let store_meta = format_ident!("__cq_store_meta_{index}");
                let mut lengths = Vec::new();
                let mut copies = Vec::new();
                for arm in arms {
                    let chains = compiler.fold(arm)?;
                    for chain in chains {
                        if !chain.operations.is_empty() {
                            return Err(error("host union operands must be published stores"));
                        }
                        let prior = chain
                            .stored
                            .ok_or_else(|| error("host union operand has not published"))?;
                        let source = &compiler.stored[prior];
                        if source.range.is_some() {
                            return Err(error(
                                "host union operand requires a materialized row publication",
                            ));
                        }
                        let length = format_ident!("__cq_store_length_{prior}");
                        lengths.push(quote!(#length));
                        let source_rows = format_ident!("__cq_store_rows_{prior}");
                        if let Some(columns) = &source.packed {
                            let owner = format_ident!("__cq_canonical_{prior}");
                            let fields = format_ident!("__cq_fields_{prior}");
                            let arm_index = copies.len();
                            let kernel = format_ident!(
                                "__cq_union_decode_{index}_{arm_index}_{module_name}"
                            );
                            let prepare = format_ident!("prepare_{kernel}");
                            let mut params = Vec::new();
                            let mut args = Vec::new();
                            let mut writes = Vec::new();
                            for (field, column) in columns.iter().enumerate() {
                                let slot = compiler.slot(column)?;
                                let base = format_ident!("base_{field}");
                                let shift = format_ident!("shift_{field}");
                                let bits = format_ident!("bits_{field}");
                                params.extend([
                                    quote!(#base:i64),
                                    quote!(#shift:u32),
                                    quote!(#bits:u32),
                                ]);
                                args.extend([
                                    quote!(#fields[#field].base),
                                    quote!(#fields[#field].shift),
                                    quote!(#fields[#field].bits),
                                ]);
                                let offset = packed_offset(
                                    quote!(keys),
                                    quote!(position),
                                    quote!(lanes),
                                    quote!(#shift),
                                    quote!(#bits),
                                );
                                // DECODE I: union arms expand FOR fields into a shared carrier then repack; avoidable with encoded-domain rebasing.
                                writes.push(quote! {
                                    let value = #offset;
                                    *rows.add(position * #width + #slot) = #base.wrapping_add(value as i64);
                                });
                            }
                            kernels.push(quote! {
                                #[kernel]
                                #[launch_bounds(256)]
                                #[launch_contract(domain=1,block=(256,1,1))]
                                pub fn #kernel(keys:*const u64, rows:*mut i64, length:usize, lanes:usize, #(#params),*) {
                                    unsafe {
                                        let mut position = ::cuda_device::thread::index_1d().get() as usize;
                                        while position < length {
                                            #(#writes)*
                                            position += ::cuda_device::thread::gridDim_x() as usize * 256;
                                        }
                                    }
                                }
                            });
                            copies.push(quote! {
                                if #length != 0 {
                                    let prepared = __cq_module.#prepare(::cuda_core::LaunchConfig1D::new((#length.div_ceil(256)) as u32,256,0))?;
                                    __cq_module.#kernel(__cq_pool.stream(), &prepared, #owner.keys().ptr(),
                                        unsafe { #store_rows.ptr().add(__cq_union_offset * #width) }, #length, #owner.lanes(), #(#args),*)?;
                                }
                                __cq_union_offset += #length;
                            });
                        } else {
                            copies.push(quote! {
                                if #length != 0 {
                                    unsafe { ::cuda_core::simt::memory::memcpy_dtod_async(
                                        #store_rows.cu_deviceptr() + (__cq_union_offset * #width * ::std::mem::size_of::<i64>()) as u64,
                                        #source_rows.cu_deviceptr(),
                                        #length * #width * ::std::mem::size_of::<i64>(),
                                        __cq_pool.stream().cu_stream())?; }
                                }
                                __cq_union_offset += #length;
                            });
                        }
                    }
                }
                launches.push(quote! {
                    let #store_length = 0usize #(.checked_add(#lengths).ok_or("Sequence union length overflow")?)*;
                    if #store_length > u32::MAX as usize { return Err("Sequence union exceeds u32 positions".into()); }
                    let #store_rows = unsafe { ::cq_gpu_runtime::Buffer::<i64>::uninitialized(
                        __cq_pool.clone(), #store_length.checked_mul(#width).ok_or("Sequence union allocation overflow")?)? };
                    let mut __cq_union_offset = 0usize;
                    #(#copies)*
                    let #store_begin = ::cq_gpu_runtime::Buffer::<u32>::zeroed(__cq_pool.clone(),0)?;
                    let #store_offsets = ::cq_gpu_runtime::Buffer::<u32>::zeroed(__cq_pool.clone(),0)?;
                    let #store_meta = ::cq_gpu_runtime::Buffer::<u32>::from_host(__cq_pool.clone(), &[#store_length as u32,0,0,0])?;
                });
                let final_output = publication == final_name;
                let packed_columns = if final_output {
                    compiler.rows.output_columns.clone()
                } else {
                    analysis.structures.physical.headings[result].clone()
                };
                if canonical || final_output {
                    let provider = canonical
                        .then(|| canon_type(analysis, &publication))
                        .transpose()?;
                    let mut fields = Vec::new();
                    let mut params = Vec::new();
                    let mut args = Vec::new();
                    let mut parts = Vec::new();
                    for (field, column) in packed_columns.iter().enumerate() {
                        let bounds = compiler.rows.bounds(column)?;
                        let base = format_ident!("base_{field}");
                        let shift = format_ident!("shift_{field}");
                        let slot = compiler.slot(column)?;
                        fields.push(quote! {
                            let (#base, bits) = match (#bounds) {
                                ::cq_gpu_runtime::Bounds::Empty => (0i64,0u32),
                                ::cq_gpu_runtime::Bounds::Inclusive {min,max} => (min,64-((max as i128-min as i128) as u64).leading_zeros()),
                            };
                            let #shift = __cq_key_bits;
                            __cq_key_bits = __cq_key_bits.checked_add(bits).ok_or("packed Sequence key width overflow")?;
                            __cq_fields.push(::cq_gpu_sequence_runtime::canon::PackedField {base:#base,shift:#shift,bits});
                        });
                        params.extend([quote!(#base:i64), quote!(#shift:u32)]);
                        args.extend([quote!(#base), quote!(#shift)]);
                        parts.push(quote! {
                            let value = (*rows.add(position * #width + #slot)).wrapping_sub(#base) as u64;
                            let lane = (#shift / 64) as usize;
                            let local = #shift % 64;
                            if lane < lanes {
                                *packed.add(lane) |= value << local;
                                if local != 0 && lane + 1 < lanes {
                                    *packed.add(lane + 1) |= value >> (64 - local);
                                }
                            }
                        });
                    }
                    let kernel = format_ident!("__cq_union_pack_{index}_{module_name}");
                    let prepare = format_ident!("prepare_{kernel}");
                    let keys = format_ident!("__cq_store_keys_{index}");
                    kernels.push(quote! {
                        #[kernel]
                        #[launch_bounds(256)]
                        #[launch_contract(domain=1,block=(256,1,1))]
                        pub fn #kernel(rows:*const i64,keys:*mut u64,length:usize,lanes:usize,#(#params),*) {
                            unsafe {
                                let mut position = ::cuda_device::thread::index_1d().get() as usize;
                                while position < length {
                                    let packed = keys.add(position * lanes);
                                    for lane in 0..lanes { *packed.add(lane) = 0; }
                                    #(#parts)*
                                    position += ::cuda_device::thread::gridDim_x() as usize * 256;
                                }
                            }
                        }
                    });
                    launches.push(quote! {
                        let mut __cq_key_bits = 0u32;
                        let mut __cq_fields = ::std::vec::Vec::new();
                        #(#fields)*
                        let __cq_key_lanes = ::cq_gpu_sequence_runtime::canon::WideCanon::key_lanes(__cq_key_bits)?;
                        let #keys = unsafe { ::cq_gpu_runtime::Buffer::<u64>::uninitialized(__cq_pool.clone(),#store_length * __cq_key_lanes)? };
                        if #store_length != 0 {
                            let prepared = __cq_module.#prepare(::cuda_core::LaunchConfig1D::new(#store_length.div_ceil(256) as u32,256,0))?;
                            __cq_module.#kernel(__cq_pool.stream(),&prepared,#store_rows.ptr(),#keys.ptr(),#store_length,__cq_key_lanes,#(#args),*)?;
                        }
                    });
                    let owner = format_ident!("__cq_canonical_{index}");
                    let saved_fields = format_ident!("__cq_fields_{index}");
                    let canonicalize = quote! {
                        let mut canon = <#provider>::new(&__cq_pool,#store_length)?;
                        let #owner = unsafe { canon.canonicalize(&#keys,#store_length,__cq_key_bits)? };
                    };
                    if final_output {
                        output = Some(if canonical {
                            quote! {
                                // DECODE V: P6 canonical union output expands FOR fields on device before host download; retain packed publication.
                                #canonicalize
                                let __cq_result = __cq_output_adapter.borrow_mut().unpack_columns(#owner,&__cq_fields,__cq_dictionary,__cq_statuses)?;
                            }
                        } else {
                            // DECODE V: P6 noncanonical union output expands FOR fields on device before host download.
                            quote! { let __cq_result = __cq_output_adapter.borrow_mut().packed_columns(#keys,#store_length,&__cq_fields,__cq_dictionary,__cq_statuses)?; }
                        });
                    } else if canonical {
                        launches.push(quote! {
                            #canonicalize
                            let #store_length = #owner.logical_count()?;
                            let #saved_fields = __cq_fields;
                        });
                    }
                }
                if !final_output {
                    launches.push(compiler.rows.publish_provider(
                        analysis,
                        program,
                        result,
                        &publication,
                        index,
                        canonical.then_some(packed_columns.as_slice()),
                    )?);
                }
                compiler.stored.push(Stored {
                    name: publication,
                    result: result.clone(),
                    range: None,
                    packed: canonical.then_some(packed_columns),
                });
            } else if matches!(definition.term, Term::Store { .. })
                && analysis
                    .levels
                    .get(&publication)
                    .is_some_and(|level| level.last().is_some_and(|level| level == "host"))
                && let Some(result) = analysis.results.get(&publication)
                && device_sources.contains(result)
                && !compiler.rows.has_source(result)
            {
                publish_host(&mut compiler, result, &mut host_downloads, &mut launches, &codecs)?;
            }
            continue;
        };
        let term = compiler
            .definitions
            .get(&publication)
            .ok_or_else(|| error("checked dispatch names an absent publication"))?;
        let Term::Store { capacity, inner } = term else {
            return Err(error(
                "GPU dispatch publication must be an explicit outer store",
            ));
        };
        if !analysis.levels.contains_key(&publication)
            || !analysis.shapes.contains_key(&publication)
            || !analysis.lengths.contains_key(&publication)
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
        let mut chains = compiler.fold(inner)?;
        if chains.is_empty() {
            return Err(error("GPU dispatch has no source"));
        }
        compiler.rehead(&publication, &mut chains);
        let canonical = chains.iter().all(|chain| {
            chain
                .operations
                .iter()
                .any(|op| matches!(op, Operation::Canon))
        });
        if arena.is_some() && final_output && canonical {
            return Err(error("GPU arena terminal publishes raw candidates; terminal canon requires a separate publication"));
        }
        let provider = canonical
            .then(|| canon_type(analysis, &publication))
            .transpose()?;
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
        let execution = dispatch.gpu.as_ref()
            .ok_or_else(|| error("GPU dispatch lacks checked launch refinement"))?;
        if execution.block_threads != 256 {
            return Err(error("GPU-Launch: this collective template requires 256 lanes"));
        }
        let virtual_range = match &execution.publication {
            cq_lang_sequence::gpu::Publication::Rows => None,
            cq_lang_sequence::gpu::Publication::DeferredRange { provider, keys } =>
                Some(compiler.rows.provider_name(provider, keys)?),
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
                    __cq_fields.push(::cq_gpu_sequence_runtime::canon::PackedField {base:#base,shift:#shift,bits:__cq_width});
                });
            }
        }
        let kernel_name = format_ident!("__cq_sequence_dispatch_{index}_{module_name}");
        let prepare_name = format_ident!("prepare_{kernel_name}");
        let mut parameters = Vec::new();
        let mut capture_domains = Vec::new();
        for (chain_index, chain) in chains.iter().enumerate() {
            let mut definitions = Vec::new();
            for operation in &chain.operations {
                if let Operation::Map(name, input) = operation {
                    definitions.extend(compiler.rows.capture_definitions(name, input)?);
                }
            }
            let domain = format_ident!("__cq_domain_{chain_index}");
            let enabled = format_ident!("__cq_enabled_{chain_index}");
            capture_domains.push((definitions, quote!(#enabled && #domain != 0)));
        }
        let mut arguments = compiler.rows.capture_arguments(&capture_domains);
        for (name, ty, _) in &compiler.rows.parameters {
            parameters.push(quote!(#name:#ty));
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
                let lanes = format_ident!("__cq_stored_lanes_{prior}");
                parameters.push(quote!(#lanes:usize));
                arguments.push(quote!(#owner.lanes()));
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
        parameters.push(quote!(__cq_key_lanes:usize));
        arguments.push(quote!(__cq_key_lanes));
        let mut generator = Kernel {
            compiler: &compiler,
            shared: crate::layout::SharedLayout::new(),
            virtual_range: virtual_range.clone(),
            packed_columns: packed_columns.clone(),
            arena_packing: if final_output { arena_packing.as_ref() } else { None },
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
            combine: None,
        };
        let mut bodies = Vec::new();
        let mut host_domains = Vec::new();
        let mut combine_bodies = Vec::new();
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
                    let lanes = format_ident!("__cq_stored_lanes_{prior}");
                    let mut unpack = TokenStream::new();
                    for (field, column) in columns.iter().enumerate() {
                        let slot = compiler.slot(column)?;
                        let base = format_ident!("__cq_stored_base_{prior}_{field}");
                        let shift = format_ident!("__cq_stored_shift_{prior}_{field}");
                        let bits = format_ident!("__cq_stored_bits_{prior}_{field}");
                        let offset = packed_offset(
                            quote!(#keys),
                            quote!(__position as usize),
                            quote!(#lanes),
                            quote!(#shift),
                            quote!(#bits),
                        );
                        // DECODE V: P6 stored intermediates reconstruct FOR fields before downstream relational operators instead of retaining codes.
                        unpack.extend(quote! {
                            let __offset = #offset;
                            __row[#slot]=#base.wrapping_add(__offset as i64);
                        });
                    }
                    quote! {
                        #unpack
                    }
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
            let tile = match execution.partition {
                cq_lang_sequence::gpu::Partition::Tiles { rows } => quote!(#rows),
                cq_lang_sequence::gpu::Partition::One => quote!((#domain as u64).max(1)),
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
            if whole {
                let combine = generator.combine.take().expect("whole-fold combine");
                combine_bodies.push(quote! {
                    if #enabled {
                        let __partial_base = #chain_index * __blocks as usize;
                        #combine
                    }
                });
            }
            // Every block finishes one branch before entering the next. Distinct
            // cursor words avoid cross-branch reset races in multi-block launches.
            let cursor = 4 + chain_index;
            bodies.push(quote! {
                if #enabled {
                    let __partial_base = #chain_index * __blocks as usize;
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
        let shared = generator.shared.bytes()?;
        if let Some(resource) = analysis.resources.get(&publication)
            && shared != resource.required_bytes
        {
            return Err(error(format!(
                "T-Gpu: publication {publication} shared layout differs from BE-Resource: emitted {shared} bytes, checked {} bytes",
                resource.required_bytes
            )));
        }
        let shared_u32 =
            u32::try_from(shared).map_err(|_| error("Sequence shared memory exceeds u32"))?;
        kernels.push(quote! {
            #[kernel]
            #[launch_bounds(256)]
            #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(#shared_u32,#shared_u32))]
            pub fn #kernel_name(#(#parameters,)* __output_rows:*mut i64,__output_begin:*mut u32,
                __output_offsets:*mut u32,__output_keys:*mut u64,__capacity:u32,__meta:*mut u32,
                __partials:*mut i64,__blocks:u32)
            {
                unsafe {
                    let __tid=::cuda_device::thread::threadIdx_x() as usize;
                    let __shared=::cuda_device::DynamicSharedArray::<u8,16>::get_raw();
                    let __scratch=__shared.cast::<u32>();
                    let __reservation=__shared.add(32).cast::<u32>();
                    let __tile_slot=__shared.add(36).cast::<u32>();
                    let __abort_slot=__shared.add(40).cast::<u32>();
                    #(#bodies)*
                }
            }
        });
        let combine_kernel = format_ident!("__cq_sequence_combine_{index}_{module_name}");
        let prepare_combine = format_ident!("prepare_{combine_kernel}");
        let whole_reduction = matches!(execution.fold, cq_lang_sequence::gpu::FoldCompletion::BlockPartials { .. });
        if whole_reduction {
            kernels.push(quote! {
                #[kernel]
                #[launch_bounds(256)]
                #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(#shared_u32,#shared_u32))]
                pub fn #combine_kernel(#(#parameters,)* __output_rows:*mut i64,__output_begin:*mut u32,
                    __output_offsets:*mut u32,__output_keys:*mut u64,__capacity:u32,__meta:*mut u32,
                    __partials:*mut i64,__blocks:u32)
                {
                    unsafe {
                        let __tid=::cuda_device::thread::threadIdx_x() as usize;
                        let __shared=::cuda_device::DynamicSharedArray::<u8,16>::get_raw();
                        let __scratch=__shared.cast::<u32>();
                        let __reservation=__shared.add(32).cast::<u32>();
                        let __abort_slot=__shared.add(40).cast::<u32>();
                        #(#combine_bodies)*
                    }
                }
            });
        }
        let store_rows = format_ident!("__cq_store_rows_{index}");
        let store_begin = format_ident!("__cq_store_begin_{index}");
        let store_offsets = format_ident!("__cq_store_offsets_{index}");
        let store_meta = format_ident!("__cq_store_meta_{index}");
        let store_length = format_ident!("__cq_store_length_{index}");
        let output_keys = format_ident!("__cq_store_keys_{index}");
        let meta_words = 4 + chains.len();
        let parts = match execution.partition {
            cq_lang_sequence::gpu::Partition::One => quote!(1usize),
            cq_lang_sequence::gpu::Partition::Tiles { rows } => {
                let domains = (0..chains.len()).map(|i| {
                    let domain = format_ident!("__cq_domain_{i}");
                    quote!((#domain as usize).div_ceil(#rows))
                });
                quote!(0usize #(.max(#domains))*)
            }
        };
        let initial = match (dispatch.concatenation.clone(), capacity) {
            (OutputProtocol::Count, _) => quote!(0usize),
            // Formal rule GPU-ReservePrevious: a recorded previous-count hint is
            // not a bound; validate full count and failure flags, then replay any
            // undersized attempt against unchanged inputs before publication.
            (OutputProtocol::ReservePrevious, _) if arena.is_some() && final_output =>
                quote!(__cq_arena.previous_count()),
            (OutputProtocol::ReservePrevious, _) =>
                return Err(error("reserve_previous requires a final GPU arena dispatch")),
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
        let complete_offsets = match execution.offsets {
            cq_lang_sequence::gpu::OffsetScan::None => quote!(__cq_required),
            cq_lang_sequence::gpu::OffsetScan::DeviceExclusive => quote!(
                ::cq_gpu_sequence_runtime::offsets::scan(&__cq_pool, &#store_offsets, __cq_required, &#store_meta)?
            ),
        };
        let key_allocation = if arena.is_some() && final_output {
            quote!(let #output_keys = __cq_arena.reserve(__cq_arena_packing.clone(), __cq_capacity)?;)
        } else {
            quote!(let #output_keys=unsafe {::cq_gpu_runtime::Buffer::<u64>::uninitialized(__cq_pool.clone(),__cq_capacity*#key_width*__cq_key_lanes)?};)
        };
        let key_pointer = if arena.is_some() && final_output {
            quote!(#output_keys.buffer().ptr())
        } else {
            quote!(#output_keys.ptr())
        };
        // The final arena dispatch publishes only its reserved keys: no later
        // stage reads its row, range-begin or offset stores, and its packing is
        // the arena's. Allocate only stores a consumer can observe.
        let arena_final = arena.is_some() && final_output;
        let packing_setup = if arena_final {
            quote!()
        } else if let Some(packing) = arena_packing.as_ref().filter(|_| final_output) {
            let bits = &packing.bits;
            quote! {
                let __cq_key_bits = #bits;
                let __cq_key_lanes = __cq_key_bits.div_ceil(64).max(1) as usize;
            }
        } else {
            quote! {
                let mut __cq_key_bits=0u32;
                let mut __cq_fields=::std::vec::Vec::<::cq_gpu_sequence_runtime::canon::PackedField>::new();
                #(#local_packing)*
                let __cq_key_lanes=::cq_gpu_sequence_runtime::canon::WideCanon::key_lanes(__cq_key_bits)?;
            }
        };
        let record_readback = (arena.is_some() || compiler.rows.has_runs())
            .then(|| quote!(::gpu_datalog_runtime::record_readback();));
        let trace_dispatch = (arena.is_some() || compiler.rows.has_runs()).then(|| quote! {
            if ::gpu_datalog_runtime::trace_kernels() {
                ::std::eprintln!("gpu_sequence_dispatch\tmodule={}\tkernel={}\tcapacity={}\tcandidates={}",
                    stringify!(#module_name), stringify!(#kernel_name), __cq_capacity, __cq_required);
            }
        });
        let remember_count = (dispatch.concatenation == OutputProtocol::ReservePrevious)
            .then(|| quote!(__cq_arena.record_count(#store_length);));
        let (stores, allocations, pointers) = if arena_final && row_width == 0 && range_width == 0 {
            (
                quote!(),
                quote!(),
                quote!(::core::ptr::null_mut::<i64>(),::core::ptr::null_mut::<u32>(),::core::ptr::null_mut::<u32>(),),
            )
        } else {
            (
                quote!(#store_rows,#store_begin,#store_offsets,),
                quote! {
                    let #store_rows=unsafe {::cq_gpu_runtime::Buffer::<i64>::uninitialized(__cq_pool.clone(),
                        __cq_capacity.checked_mul(#row_width).ok_or("Sequence row allocation overflow")?)?};
                    let #store_begin=unsafe {::cq_gpu_runtime::Buffer::<u32>::uninitialized(__cq_pool.clone(),__cq_capacity*#range_width)?};
                    let #store_offsets=unsafe {::cq_gpu_runtime::Buffer::<u32>::uninitialized(__cq_pool.clone(),
                        __cq_capacity.checked_add(1).ok_or("Sequence offset allocation overflow")?*#range_width)?};
                },
                quote!(#store_rows.ptr(),#store_begin.ptr(),#store_offsets.ptr(),),
            )
        };
        let partial_count = if whole_reduction { chains.len() } else { 0 };
        let combine_launch = whole_reduction.then(|| quote! {
            let prepared=__cq_module.#prepare_combine(::cuda_core::LaunchConfig1D::new(1,256,#shared_u32))?;
            __cq_module.#combine_kernel(__cq_pool.stream(), &prepared, #(#arguments,)*
                #pointers #key_pointer, __cq_capacity as u32, #store_meta.ptr(),
                __cq_partials.ptr(), __cq_grid)?;
        });
        let cq_lang_sequence::gpu::Grid::KernelOccupancy = execution.grid;
        let count_readback = execution.synchronization.iter().find_map(|point| match point {
            cq_lang_sequence::gpu::Synchronization::CountAndFailure =>
                Some(quote!(let __cq_counts=#store_meta.to_host()?;)),
            cq_lang_sequence::gpu::Synchronization::RangeOffsets => None,
        }).ok_or_else(|| error("GPU-Launch: output protocol lacks a count/failure synchronization"))?;
        launches.push(quote! {
            #packing_setup
            #(#host_domains)*
            let __cq_probe=__cq_module.#prepare_name(::cuda_core::LaunchConfig1D::new(1,256,#shared_u32))?;
            let __cq_grid=::cq_gpu_runtime::launch_grid_for_kernel(
                __cq_probe.function(), #parts, 256, #shared_u32)?;
            let __cq_prepared=__cq_module.#prepare_name(::cuda_core::LaunchConfig1D::new(__cq_grid,256,#shared_u32))?;
            let __cq_partials=::cq_gpu_runtime::Buffer::<i64>::zeroed(
                __cq_pool.clone(), __cq_grid as usize * #partial_count)?;
            let mut __cq_capacity=#initial;
            let (#stores #output_keys,#store_meta,#store_length)=loop {
                if __cq_capacity>u32::MAX as usize {return Err("Sequence output exceeds u32 positions".into());}
                #allocations
                #key_allocation
                let #store_meta=::cq_gpu_runtime::Buffer::<u32>::zeroed(__cq_pool.clone(),#meta_words)?;
                __cq_module.#kernel_name(__cq_pool.stream(),&__cq_prepared,#(#arguments,)*
                    #pointers #key_pointer,
                    __cq_capacity as u32,#store_meta.ptr(),__cq_partials.ptr(),__cq_grid)?;
                #combine_launch
                #count_readback
                #record_readback
                if __cq_counts[2]!=0 {return Err("Sequence device count or fold overflow".into());}
                let __cq_required=__cq_counts[0] as usize;
                #trace_dispatch
                if __cq_required>__cq_capacity {
                    __cq_capacity=__cq_required;
                    continue;
                }
                let __cq_logical = #complete_offsets;
                break (#stores #output_keys,#store_meta,__cq_logical);
            };
            #remember_count
        });
        if final_output {
            output = Some(if arena.is_some() {
                quote! {
                    let __cq_result = unsafe { #output_keys.finish(#store_length, __cq_run_owners)? };
                }
            } else if canonical {
                quote! {
                    let mut __cq_canon=<#provider>::new(&__cq_pool,#store_length)?;
                    let __cq_keys=unsafe {__cq_canon.canonicalize(&#output_keys,#store_length,__cq_key_bits)?};
                    // DECODE V: P6 canonical output adapter reconstructs device i64 columns before final host publication.
                    let __cq_result=__cq_output_adapter.borrow_mut().unpack_columns(__cq_keys,&__cq_fields,__cq_dictionary,__cq_statuses)?;
                }
            } else {
                quote! {
                    // DECODE V: P6 packed output adapter reconstructs device i64 columns before final host publication.
                    let __cq_result=__cq_output_adapter.borrow_mut().packed_columns(
                        #output_keys,#store_length,&__cq_fields,__cq_dictionary,__cq_statuses)?;
                }
            });
        }
        if canonical && !final_output {
            let canonical_owner = format_ident!("__cq_canonical_{index}");
            let fields = format_ident!("__cq_fields_{index}");
            launches.push(quote! {
                let mut __cq_canon=<#provider>::new(&__cq_pool,#store_length)?;
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
                &publication,
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
    if output.is_none() {
        fn canonical_publication(term: &Term, definitions: &BTreeMap<String, &Term>) -> bool {
            match term {
                Term::Canon(_) => true,
                Term::Store { inner, .. } => canonical_publication(inner, definitions),
                Term::Ref(name) => definitions
                    .get(&symbol_name(name))
                    .is_some_and(|term| canonical_publication(term, definitions)),
                _ => false,
            }
        }
        let canonical = compiler
            .definitions
            .get(final_name)
            .is_some_and(|term| canonical_publication(term, &compiler.definitions));
        let provider = canonical
            .then(|| canon_type(analysis, final_name))
            .transpose()?;
        // A checked host publication may be the complete result. Its transfer
        // to the device is a storage adapter, not an additional row dispatch.
        let mut name = final_name;
        let source = loop {
            if analysis.stores.contains_key(name)
                && analysis
                    .levels
                    .get(name)
                    .is_some_and(|level| level.last().is_some_and(|level| level == "host"))
            {
                break analysis.results.get(name).map(String::as_str);
            }
            let Some(Term::Store { inner, .. }) = compiler.definitions.get(name).copied() else {
                break None;
            };
            let Term::Ref(parent) = inner.as_ref() else {
                break None;
            };
            let Some((parent, _)) = compiler.definitions.get_key_value(&symbol_name(parent)) else {
                break None;
            };
            name = parent;
        };
        if let Some(source) = source {
            let length = compiler.rows.source_len(source)?;
            let load = compiler
                .rows
                .source(source, quote!(__position), quote!(__row))?;
            let mut row = DeviceRow::default();
            let mut declarations = TokenStream::new();
            for column in &compiler.rows.output_columns {
                let slot = compiler.slot(column)?;
                let code = format_ident!("__code_{slot}");
                declarations.extend(quote!(let mut #code = 0i64;));
                row.0.insert(slot, quote!(#code));
            }
            let load = row.rewrite(load)?;
            let mut parameters = Vec::new();
            let mut arguments = compiler.rows.capture_arguments(&[]);
            for (name, ty, _) in &compiler.rows.parameters {
                parameters.push(quote!(#name:#ty));
            }
            for i in 0..compiler.rows.output_columns.len() {
                let base = format_ident!("__cq_pack_base_{i}");
                let shift = format_ident!("__cq_pack_shift_{i}");
                parameters.extend([quote!(#base:i64), quote!(#shift:u32)]);
                arguments.extend([quote!(#base), quote!(#shift)]);
            }
            let generator = Kernel {
                compiler: &compiler,
                shared: crate::layout::SharedLayout::new(),
                virtual_range: None,
                packed_columns: Some(compiler.rows.output_columns.clone()),
                arena_packing: None,
                publication_columns: BTreeSet::new(),
                sink: None,
                scalar_output: false,
                combine: None,
            };
            let write = row.rewrite(generator.output_write()?)?;
            let kernel = format_ident!("__cq_host_output_{module_name}");
            let prepare = format_ident!("prepare_{kernel}");
            kernels.push(quote! {
                #[kernel]
                #[launch_bounds(256)]
                #[launch_contract(domain=1,block=(256,1,1))]
                pub fn #kernel(#(#parameters,)* __output_keys:*mut u64,
                    __length:usize,__cq_key_lanes:usize)
                {
                    unsafe {
                        let __position = ::cuda_device::thread::index_1d().get() as usize;
                        if __position < __length {
                            #declarations
                            #load
                            let __destination = __position;
                            #write
                        }
                    }
                }
            });
            launches.push(quote! {
                let __cq_host_output_length = #length;
                let mut __cq_key_bits = 0u32;
                let mut __cq_fields = ::std::vec::Vec::new();
                #(#packing)*
                let __cq_key_lanes = ::cq_gpu_sequence_runtime::canon::WideCanon::key_lanes(__cq_key_bits)?;
                let __cq_host_output_keys = unsafe {
                    ::cq_gpu_runtime::Buffer::<u64>::uninitialized(__cq_pool.clone(),
                        __cq_host_output_length.checked_mul(__cq_key_lanes).ok_or("Sequence output allocation overflow")?)?
                };
                if __cq_host_output_length != 0 {
                    let grid = u32::try_from(__cq_host_output_length.div_ceil(256))
                        .map_err(|_| "Sequence output exceeds CUDA grid")?;
                    let prepared = __cq_module.#prepare(::cuda_core::LaunchConfig1D::new(grid,256,0))?;
                    __cq_module.#kernel(__cq_pool.stream(), &prepared, #(#arguments,)*
                        __cq_host_output_keys.ptr(),__cq_host_output_length,__cq_key_lanes)?;
                }
            });
            output = Some(if canonical {
                quote! {
                    let mut canon = <#provider>::new(
                        &__cq_pool,__cq_host_output_length)?;
                    let keys = unsafe { canon.canonicalize(
                        &__cq_host_output_keys,__cq_host_output_length,__cq_key_bits)? };
                    // DECODE V: P6 host-origin canonical output is reuploaded then expanded on device, rather than decoded at host publication.
                    let __cq_result = __cq_output_adapter.borrow_mut().unpack_columns(
                        keys,&__cq_fields,__cq_dictionary,__cq_statuses)?;
                }
            } else {
                quote! {
                    // DECODE V: P6 host-origin packed output expands device columns before the user's host download.
                    let __cq_result = __cq_output_adapter.borrow_mut().packed_columns(
                        __cq_host_output_keys,__cq_host_output_length,&__cq_fields,__cq_dictionary,__cq_statuses)?;
                }
            });
        }
    }
    launches.push(
        output.ok_or_else(|| error("checked GPU output has no recorded publication adapter"))?,
    );
    prune_kernel_parameters(&mut kernels, &mut launches, &compiler.rows.rust_functions)?;
    let output = launches.pop().expect("output launch");
    let setup = &compiler.rows.setup;
    let build = &compiler.rows.build;
    let arena_setup = arena.zip(arena_packing.as_ref()).map(|(expression, packing)| packing.setup(expression));
    let mut used = BTreeSet::new();
    identifiers(quote!(#setup #arena_setup #build #(#launches)* #output), &mut used);
    let support = compiler.rows.support.iter().filter(|(name, ..)| used.contains(&name.to_string()));
    let (support_before, support_after): (Vec<_>, Vec<_>) =
        support.map(|(_, before, after)| (before, after)).unzip();
    // Formal rule GPU-Arena-Timing: optional measurements alone add a terminal fence;
    // checked dispatch metadata and retained stream-ordered owners suffice otherwise.
    let timing_enabled = if arena.is_some() {
        quote!(::cq_gpu_runtime::arena_timing_enabled())
    } else {
        quote!(true)
    };
    syn::parse2(quote!({
        #[::cuda_device::cuda_module]
        mod #module_name {
            use super::*;
            use ::cuda_device::{kernel,launch_bounds,launch_contract};
            #(#kernels)*
        }
        ::cq_gpu_sequence_runtime::__codegen::#partition::run(#[inline] || -> ::cq_gpu_sequence_runtime::Result<_> {
            #(#support_before)*
            #setup
            #(#support_after)*
            #arena_setup
            ::std::thread_local! {
                static __CQ_SEQUENCE_MODULE: ::std::cell::RefCell<Option<(
                    ::std::sync::Arc<::cuda_core::CudaContext>,
                    #module_name::LoadedModule,
                )>> = const { ::std::cell::RefCell::new(None) };
            }
            ::cq_gpu_sequence_runtime::register_query_cleanup(|| {
                __CQ_SEQUENCE_MODULE.with(|cache| cache.borrow_mut().take());
            });
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
            let __cq_timing_enabled = #timing_enabled;
            let __cq_timing_wall = __cq_timing_enabled.then(::std::time::Instant::now);
            let __cq_timing_start = if __cq_timing_enabled {
                Some(__cq_pool.stream().record_event(
                    Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?)
            } else { None };
            #build
            #(#launches)*
            let __cq_timing_expanded = if __cq_timing_enabled {
                Some(__cq_pool.stream().record_event(
                    Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?)
            } else { None };
            #output
            if let (Some(wall), Some(start), Some(built), Some(expanded)) =
                (__cq_timing_wall, __cq_timing_start, __cq_timing_built, __cq_timing_expanded)
            {
                let finished = __cq_pool.stream().record_event(
                    Some(::cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
                __cq_pool.stream().synchronize()?;
                ::cq_gpu_runtime::publish_timings(::cq_gpu_runtime::Timings {
                    build_ms: start.elapsed_ms(&built)? as f64,
                    kernel_ms: built.elapsed_ms(&expanded)? as f64,
                    dedup_ms: expanded.elapsed_ms(&finished)? as f64,
                    total_ms: start.elapsed_ms(&finished)? as f64,
                    wall_ms: wall.elapsed().as_secs_f64()*1000.0,
                    ..::cq_gpu_runtime::Timings::default()
                });
            }
            Ok(__cq_result)
        }).expect("GPU Sequence execution failed")
    }))
}

fn identifiers(tokens: TokenStream, names: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Ident(name) => {
                names.insert(name.to_string());
            }
            proc_macro2::TokenTree::Group(group) => identifiers(group.stream(), names),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    #[test]
    fn datalog_key_projections_preserve_fresh_column_binding_spans() {
        use syn::visit::Visit;

        // Fork's Batch key holes use mixed-site variables, the same names in
        // their declared output columns, and owned dereference projections.
        // Distinct source locations make rebuilding a call-site binder fail
        // deterministically even outside a running procedural macro.
        let mut keys = [
            syn::parse_str::<syn::Ident>("__cq_datalog_column_0").unwrap(),
            syn::parse_str::<syn::Ident>("__cq_datalog_column_1").unwrap(),
        ];
        for key in &mut keys {
            key.set_span(Span::mixed_site().located_at(key.span()));
        }
        let [key0, key1] = &keys;
        for (predicate, selected) in [
            (quote!(select r if (#key0 > 0)), quote!(compact filtered flatten tile 256 input)),
            (quote!(guard r if (*#key0 > 0 && *#key1 > 0)), quote!(map filtered flatten tile 256 input)),
        ] {
            // Physical output bindings have distinct temporary attributes;
            // the declared head names still coincide with the input variables.
            let program: Program = syn::parse2(quote! {
                physical {
                    relation R(a:i32,b:i32) = (&data.r);
                    relation answer(#key0:i32,#key1:i32);
                    r = pipe rename R@0 {a -> #key0,b -> #key1};
                    filtered = pipe #predicate;
                    first = pipe bind filtered with out0:i32 = (*#key0);
                    second = pipe bind first with out1:i32 = (*#key1);
                    result = pipe canon project second keep {out0,out1};
                    return result as answer(out0,out1);
                }
                structures {
                    r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i32,i32)> [];
                    result canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                }
                sequence {
                    input = store rows r;
                    selected = #selected;
                    output_rows = store canon map result selected;
                    output count;
                }
                graded gpu {
                    input at host;
                    selected at host.device.block;
                    output_rows at host.device;
                }
            }).unwrap();
            let program = crate::runs::record_representation(&program).unwrap();
            let analysis =
                cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
            let generated = translate(&program, &analysis).unwrap();

            struct Bindings<'a> {
                keys: &'a [syn::Ident; 2],
                counts: [usize; 2],
            }
            impl<'ast> Visit<'ast> for Bindings<'_> {
                fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
                    for (index, key) in self.keys.iter().enumerate() {
                        if pattern.ident == *key {
                            assert_eq!(pattern.ident.span().start(), key.span().start());
                            assert_eq!(pattern.ident.span().end(), key.span().end());
                            self.counts[index] += 1;
                        }
                    }
                    syn::visit::visit_pat_ident(self, pattern);
                }
            }
            let mut bindings = Bindings { keys: &keys, counts: [0; 2] };
            bindings.visit_expr(&generated);
            assert!(bindings.counts.iter().all(|count| *count > 0));
        }
    }

    #[test]
    fn parameter_budget_is_checked_by_rustc_layout() {
        let names: Vec<_> = (0..513).map(|index| format_ident!("p{index}")).collect();
        let mut kernels = vec![quote! {
            fn oversized(#(#names: *const u64),*) {
                #(let _ = #names;)*
            }
        }];
        prune_kernel_parameters(&mut kernels, &mut [], &[]).unwrap();
        let emitted = kernels[0].to_string();
        assert!(emitted.contains("size_of"));
        assert!(emitted.contains("align_of"));
        assert!(emitted.contains("BE-Resource"));
    }

    #[test]
    fn pruning_preserves_signature_and_launch_order() {
        let mut kernels = vec![quote! {
            fn kernel(unused: u32, a: u32, b: *const i64) { let _ = (b, a); }
        }];
        let mut launches =
            vec![quote! { module.kernel(stream, prepared, unused(), first(), second())?; }];
        prune_kernel_parameters(&mut kernels, &mut launches, &[]).unwrap();
        let function: syn::ItemFn = syn::parse2(kernels[0].clone()).unwrap();
        assert_eq!(
            function.sig.inputs.to_token_stream().to_string(),
            quote!(a: u32, b: *const i64).to_string()
        );
        assert_eq!(
            launches[0].to_string(),
            quote!(module.kernel(stream, prepared, first(), second())?;).to_string()
        );
    }

    #[test]
    fn rust_function_parameters_use_inferred_prepare_witnesses() {
        let mut kernels = vec![quote! {
            fn kernel(unused: Unused, evaluate: Evaluate, x: i64) {
                let _ = evaluate(&x);
            }
        }];
        let mut launches = vec![quote! {
            let prepared = module.prepare_kernel(config)?;
            module.kernel(stream, &prepared, unused, evaluate, x)?;
        }];
        let functions = vec![
            (format_ident!("Unused"), quote!(Fn() -> bool), format_ident!("unused")),
            (format_ident!("Evaluate"), quote!(Fn(&i64) -> bool), format_ident!("evaluate")),
        ];
        prune_kernel_parameters(&mut kernels, &mut launches, &functions).unwrap();
        let function: syn::ItemFn = syn::parse2(kernels[0].clone()).unwrap();
        assert_eq!(
            function.sig.generics.to_token_stream().to_string(),
            quote!(<Evaluate: Copy + Fn(&i64) -> bool>).to_string()
        );
        assert_eq!(launches[0].to_string(), quote! {
            let prepared = module.prepare_kernel_for(&evaluate, config)?;
            module.kernel(stream, &prepared, evaluate, x)?;
        }.to_string());
    }

    fn uploaded_host_program(canonical: bool) -> Program {
        let canon = canonical.then(|| quote!(canon));
        let program = syn::parse_quote! {
            physical {
                relation Resident(a:i64) = (&data.resident);
                relation Host(a:i64) = (&data.host);
                resident = let rename Resident@0 {a -> a};
                host_input = pipe rename Host@1 {a -> a};
                host = pipe canon semi eager [a] host_input,resident;
                return host as answer(a);
            }
            structures {
                resident host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                host_input host scan = ::core::iter::Map<_,_> [];
                resident = ::cq_storage::SortedRun<_,_> [a];
                host canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                resident_rows = store rows resident;
                host_rows = rows host_input;
                published_host = store #canon compact host lookup resident [a] host_rows;
                result = store published_host;
                output count;
            }
            graded gpu {
                resident_rows at host;
                host_rows at host;
                published_host at host;
                result at host.device;
            }
        };
        crate::runs::record_representation(&program).unwrap()
    }

    #[test]
    fn terminal_group_store_alias_emits_one_collective_without_host_reupload() {
        for keyed in [false, true] {
            let keys = keyed.then(|| quote!(key));
            let output_key = keyed.then(|| quote!(key,));
            let accumulator = keyed.then(|| quote!(
                grouped group = ::std::collections::HashMap<_, (i64, usize)>;
            ));
            let program: Program = syn::parse2(quote! {
                physical {
                    relation R(id:i64,key:i32,value:i64) = (&data.r);
                    r = pipe rename R@0 {id -> id,key -> key,value -> value};
                    valuation = pipe canon project r keep {id,key,value};
                    grouped = pipe group valuation by [#keys] {
                        total:i64=sum(value); rows:usize=count();
                    };
                    return grouped as answer(#output_key total,rows);
                }
                structures {
                    r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i32,i64)> [];
                    valuation canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                    #accumulator
                }
                sequence {
                    input = store rows r;
                    feed = canon map valuation flatten tile 256 input;
                    valuations = store feed;
                    groups = store group grouped valuations;
                    result = store groups;
                    output count;
                }
                graded gpu {
                    input at host;
                    feed at host.device.block;
                    valuations at host;
                    groups at host;
                    result at host.device;
                }
            })
            .unwrap();
            let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
            let generated = translate(&program, &analysis)
                .unwrap()
                .to_token_stream()
                .to_string();
            assert_eq!(generated.matches("group :: fold").count(), 1, "{generated}");
            assert!(
                !generated.contains("__cq_sequence_uploaded_grouped"),
                "{generated}"
            );
            assert!(!generated.contains("__cq_downloaded_"), "{generated}");
        }
    }

    #[test]
    fn keyed_group_reads_resident_columns_without_valuation_publication() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(id:i64,key:i32,value:i64) = (&data.r);
                r = pipe rename R@0 {id -> id,key -> key,value -> value};
                valuation = pipe canon project r keep {id,key,value};
                grouped = pipe group valuation by [key] { total:i64=sum(value); rows:usize=count(); };
                return grouped as answer(key,total,rows);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i32,i64)> [];
                valuation canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                grouped group = ::std::collections::HashMap<_, (i64, usize)>;
            }
            sequence {
                input = rows r;
                valuations = map valuation input;
                groups = store group grouped valuations;
                output count;
            }
            graded gpu { input at host; valuations at host; groups at host; }
            evidence { R(id) key; }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let generated = translate(&program, &analysis)
            .unwrap()
            .to_token_stream()
            .to_string();
        assert_eq!(generated.matches("group :: fold").count(), 1);
        assert!(generated.contains("__cq_group_live"));
        assert!(!generated.contains("packed_columns"));
        assert!(!generated.contains("__cq_canonical_"));
    }

    #[test]
    fn grouped_host_rust_input_is_materialized_before_device_reduction() {
        for provider in [
            quote!(::core::iter::Map<_,_>),
            quote!(::gpu_datalog_runtime::DeviceLayout<
                (i64,), (::gpu_datalog_runtime::Symbol<i32, TestCodec>,)>),
        ] {
            let program: Program = syn::parse2(quote! {
                physical {
                    relation R(id:i64,raw:i32) = (&data.r);
                    r = pipe rename R@0 {id -> id,raw -> raw};
                    bound = pipe bind r with value:i64 = (*raw as i64);
                    valuation = pipe canon project bound keep {id,value};
                    grouped = pipe group valuation by [] { total:i64=sum(value); rows:usize=count(); };
                    return grouped as answer(total,rows);
                }
                structures {
                    r host scan = #provider [];
                    valuation canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                }
                sequence {
                    input = rows r;
                    valuations = map valuation input;
                    groups = store group grouped valuations;
                    output count;
                }
                graded gpu { input at host; valuations at host; groups at host; }
                evidence { R(id) key; }
            }).unwrap();
            let program = crate::runs::record_representation(&program).unwrap();
            let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
            let generated = translate(&program, &analysis).unwrap().to_token_stream().to_string();
            let upload = generated.find("let __cq_sequence_uploaded_valuation").unwrap();
            let reduction = generated.find("group :: fold").unwrap();
            assert!(upload < reduction, "{generated}");
            assert!(generated.contains("* raw as i64"), "{generated}");
            assert_eq!(generated.matches("group :: fold").count(), 1);
        }
    }

    #[test]
    fn host_guard_lengths_use_the_uploaded_publication_count() {
        let program = uploaded_host_program(false);
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let mut compiler = Compiler {
            program: &program,
            analysis: &analysis,
            rows: Rows::new(&program, &analysis).unwrap(),
            stored: Vec::new(),
            definitions: BTreeMap::new(),
        };
        for length in [
            Len::Input("published_host".into()),
            Len::Scan("published_host".into()),
        ] {
            assert_eq!(
                compiler.length(&length).unwrap().to_string(),
                "__cq_sequence_uploaded_length_host"
            );
        }
        compiler.stored.push(Stored {
            name: "published_host".into(),
            result: "host".into(),
            range: None,
            packed: None,
        });
        assert_eq!(
            compiler
                .length(&Len::Scan("published_host".into()))
                .unwrap()
                .to_string(),
            "__cq_store_length_0"
        );
    }

    #[test]
    fn host_output_transfer_publishes_without_a_row_dispatch() {
        for canonical in [false, true] {
            let program = uploaded_host_program(canonical);
            let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
            assert!(analysis.dispatches.is_empty());
            let expression = translate(&program, &analysis).unwrap();
            let generated = quote!(#expression).to_string();
            assert!(generated.contains("__cq_sequence_uploaded_length_host"));
            assert!(generated.contains("let __cq_host_output_length"));
            assert!(generated.contains("__cq_host_output_"));
            assert!(!generated.contains("__cq_sequence_dispatch_"));
            assert_eq!(generated.contains("canon . canonicalize"), canonical);
            assert!(!generated.contains("__row"));
        }
    }

    #[test]
    fn host_read_of_a_device_only_provider_requires_a_checked_placement_change() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i64) = (&data.r);
                relation P(a:i64) = (&data.p);
                r = pipe rename R@0 {a -> a};
                p = let rename P@1 {a -> a};
                present = let canon semi eager [a] r,p;
                return present as answer(a);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                p host scan = ::core::iter::Map<_,_> [];
                p = ::cq_gpu_sequence_runtime::RangeProvider [a];
                present rows = ::std::vec::Vec<_>;
                present canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                input = store rows r;
                index = store rows p;
                host_rows = store canon compact present lookup p [a] input;
                result = store host_rows;
                output count;
            }
            graded gpu {
                input at host;
                index at host;
                host_rows at host;
                result at host.device;
            }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let error = translate(&program, &analysis).unwrap_err().to_string();
        assert!(
            error.contains("host publication present reads device-only provider p"),
            "{error}"
        );
    }

    #[test]
    fn deferred_host_map_reads_the_published_rows_not_the_original_producer() {
        for canonical in [false, true] {
            let canon = canonical.then(|| quote!(canon));
            let canon_provider = canonical.then(|| {
                quote!(
                    filtered canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                )
            });
            let program: Program = syn::parse2(quote! {
                physical {
                    relation R(a:i64) = (&data.r);
                    r = pipe rename R@0 {a -> a};
                    filtered = let #canon select r if (a > 0);
                    guarded = pipe canon guard filtered if (a % 2 == 0);
                    return guarded as answer(a);
                }
                structures {
                    r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                    filtered rows = ::std::vec::Vec<_>;
                    guarded canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
                    #canon_provider
                }
                sequence {
                    input = store rows r;
                    feed = #canon compact filtered flatten tile 256 input;
                    device_rows = store feed;
                    host_rows = store canon map guarded device_rows;
                    result = store host_rows;
                    output count;
                }
                graded gpu {
                    input at host;
                    feed at host.device.block;
                    device_rows at host;
                    host_rows at host;
                    result at host.device;
                }
            })
            .unwrap();
            let program = crate::runs::record_representation(&program).unwrap();
            let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
            let expression = translate(&program, &analysis).unwrap();
            let generated = quote!(#expression).to_string();
            let publication = generated.find("let (__cq_store_rows_0").unwrap();
            let download = generated.find("let __cq_downloaded_0").unwrap();
            let upload = generated
                .find("let __cq_sequence_uploaded_guarded")
                .unwrap();
            assert!(publication < download && download < upload);
            assert!(generated.contains("__cq_downloaded_0 . iter"));
            assert!(!generated.contains("__cq_leaf_0 . to_host"));
            assert!(generated.contains("__cq_host_output_length"));
        }
    }

    #[test]
    fn emitter_rejects_drift_from_the_checked_shared_layout() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i64) = (&data.r);
                r = pipe canon rename R@0 {a -> a};
                return r as answer(a);
            }
            structures {
                r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                r canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
            }
            sequence {
                input = store rows r;
                feed = flatten tile 256 input;
                result = store canon feed;
                output count;
            }
            graded gpu {
                input at host;
                feed at host.device.block;
                result at host.device;
            }
        };
        let mut analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        analysis.resources.get_mut("result").unwrap().required_bytes += 16;
        let error = translate(&program, &analysis).unwrap_err().to_string();
        assert!(
            error.contains("shared layout differs from BE-Resource"),
            "{error}"
        );
    }

    #[test]
    fn checked_shared_layout_accounts_for_every_choice_chain() {
        for chains in [1, 64, 128] {
            let shape = cq_lang_sequence::DispatchShape {
                publication: "result".into(),
                level: vec!["host".into(), "device".into(), "block".into()],
                width: Some(256),
                tile: Some(256),
                items: 1,
                chains: (0..chains)
                    .map(|_| vec![cq_lang_sequence::ResourceAllocation::Fold])
                    .collect(),
            };
            let bytes = crate::layout::shared_bytes(&shape).unwrap();
            assert_eq!(bytes, 48 + chains * 256 * 8);
            assert_eq!(bytes > 101376, chains > 1);
        }
    }

    #[test]
    fn shared_layout_keeps_i64_payload_alignment() {
        let mut layout = crate::layout::SharedLayout::new();
        assert_eq!(layout.payload().unwrap(), (48, 1072));
        assert_eq!(layout.parent(8).unwrap(), 2104);
        assert_eq!(layout.parent(8).unwrap(), 4152);
        assert_eq!(layout.fold().unwrap(), 6200);
        assert_eq!(layout.bytes().unwrap(), 8256);
    }

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
                structures {
                    r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(#ty,),::cq_gpu_runtime::For> [];
                    r canon = Canon;
                }
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
                        shared: crate::layout::SharedLayout::new(),
                        virtual_range: None,
                        packed_columns: Some(vec!["a".into()]),
                        arena_packing: None,
                        publication_columns: BTreeSet::new(),
                        sink: None,
                        scalar_output: false,
                        combine: None,
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
