use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

use cq_lang_sequence::gpu::publication::{
    ColumnBounds, FieldBounds, KeyCodec, KeyField, PublicationRecord, PublicationStorage,
};
use cq_lang_sequence::gpu::realization::{
    Granularity, GroupCompletion, GroupInput, HostEvaluation, HostOutput, Step, TransferSource,
    UnionArm, Launch,
};
use cq_lang_sequence::gpu::{FoldScope, Grid, Strip};
use cq_lang_sequence::{Analysis, Guard, Len, Monoid, OutputProtocol, Program, Term};
use cq_lang_structures::Capability;
use cq_lang_structures::physical::{Operator, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

use crate::realize::grouped_boundary;
use crate::rows::{DeviceRow, Rows};

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(Span::call_site(), message)
}

// GPU-ResidentMeta word layout of the arena's resident words: the last block
// copies the first `RESIDENT_RESULT` dispatch words (count, -, failure, -) to
// the header, `RESIDENT_FINISHED` counts finished blocks, and the dispatch's
// own metadata words start after `RESIDENT_HEADER` header words.
const RESIDENT_RESULT: usize = 4;
const RESIDENT_FINISHED: usize = 4;
const RESIDENT_HEADER: usize = 8;

fn publication_record<'a>(
    analysis: &'a Analysis,
    name: &str,
) -> syn::Result<&'a PublicationRecord> {
    analysis.gpu_publications.get(name).ok_or_else(|| {
        error(format!(
            "GPU-PublicationStorage: `{name}` lacks a recorded storage recipe"
        ))
    })
}

fn packed_columns(storage: &PublicationStorage) -> Option<Vec<String>> {
    storage.codec().map(|codec| {
        codec
            .fields()
            .iter()
            .map(|field| field.column.clone())
            .collect()
    })
}

/// Prints the recorded bounds source; no printer state is consulted, so every
/// packing of one record renders the same domain.
fn packing_bounds(bounds: &FieldBounds) -> TokenStream {
    match bounds {
        FieldBounds::Column(ColumnBounds::Resident { sources }) => {
            let values = sources.iter().map(|(host, index)| {
                let host = format_ident!("__cq_sequence_host_{}", host);
                quote!(#host.storage().bounds[#index])
            });
            quote! {
                [#(#values),*].into_iter().fold(::cq_gpu_runtime::Bounds::Empty, |a,b| match (a,b) {
                    (::cq_gpu_runtime::Bounds::Empty, x) | (x, ::cq_gpu_runtime::Bounds::Empty) => x,
                    (::cq_gpu_runtime::Bounds::Inclusive { min:a, max:b }, ::cq_gpu_runtime::Bounds::Inclusive { min:c, max:d }) =>
                        ::cq_gpu_runtime::Bounds::Inclusive { min:a.min(c), max:b.max(d) },
                })
            }
        }
        FieldBounds::Column(ColumnBounds::Scalar { ty }) => quote! {
            ::cq_gpu_runtime::Bounds::Inclusive {
                min: <#ty as ::gpu_datalog_runtime::Scalar>::MIN as i64,
                max: <#ty as ::gpu_datalog_runtime::Scalar>::MAX as i64,
            }
        },
        FieldBounds::Inclusive { min, max } => {
            quote!(::cq_gpu_runtime::Bounds::Inclusive { min: #min, max: #max })
        }
    }
}

fn packing_field(field: &KeyField, base: &syn::Ident, shift: &syn::Ident) -> TokenStream {
    let bounds = packing_bounds(&field.bounds);
    quote! {
        let (#base, __cq_width) = match (#bounds) {
            ::cq_gpu_runtime::Bounds::Empty => (0i64, 0u32),
            ::cq_gpu_runtime::Bounds::Inclusive { min, max } => {
                (min, 64 - ((max as i128 - min as i128) as u64).leading_zeros())
            }
        };
        let #shift = __cq_key_bits;
        __cq_key_bits = __cq_key_bits.checked_add(__cq_width).ok_or("packed Sequence key width overflow")?;
        __cq_fields.push(::cq_gpu_sequence_runtime::canon::PackedField {
            base: #base, shift: #shift, bits: __cq_width,
        });
    }
}

fn packing_fields(codec: &KeyCodec) -> Vec<TokenStream> {
    match codec {
        KeyCodec::FrameOfReference { fields } => fields
            .iter()
            .enumerate()
            .map(|(i, field)| {
                packing_field(
                    field,
                    &format_ident!("__cq_pack_base_{i}"),
                    &format_ident!("__cq_pack_shift_{i}"),
                )
            })
            .collect(),
    }
}

fn key_lanes(codec: &KeyCodec) -> TokenStream {
    match codec {
        KeyCodec::FrameOfReference { .. } => quote!((__cq_key_bits as usize).div_ceil(64).max(1)),
    }
}

fn set_params(
    record: &cq_lang_sequence::gpu::publication::PublicationRecord,
) -> Option<TokenStream> {
    record.set_build.as_ref().map(|build| {
        let ratio = build.ratio;
        let radix_bits = build.radix_bits;
        let merge_tile_rows = build.merge_tile_rows;
        let block_threads = build.block_threads;
        quote!(::gpu_datalog_runtime::radix_run::RadixRunParams {
            ratio: #ratio,
            radix_bits: #radix_bits,
            merge_tile_rows: #merge_tile_rows,
            block_threads: #block_threads,
        })
    })
}

/// The final generated scalar ABI. Types are opaque payloads: dependencies on
/// callable/word witnesses come exclusively from the row ABI producer.
#[derive(Clone)]
struct AbiParameter {
    name: syn::Ident,
    ty: syn::Type,
    live: bool,
    witness: Option<usize>,
}

#[derive(Clone)]
struct KernelAbi {
    name: syn::Ident,
    attributes: TokenStream,
    body: TokenStream,
    uses: BTreeSet<String>,
    parameters: Vec<AbiParameter>,
    witnesses: Vec<(syn::Ident, TokenStream, syn::Ident)>,
    preparation: syn::Ident,
    launch: Launch,
    shared: u32,
}

/// Only the ABI checker constructs this printable record. Kernel definitions,
/// preparations and calls all consume the same record, never generated Rust.
pub(crate) struct CheckedKernel(KernelAbi);

impl KernelAbi {
    fn check(self, rows: &Rows) -> syn::Result<CheckedKernel> {
        let mut names = BTreeSet::new();
        let mut witnesses = BTreeSet::new();
        for parameter in &self.parameters {
            if !names.insert(parameter.name.to_string())
                || (!parameter.live && self.uses.contains(&parameter.name.to_string()))
            {
                return Err(error("GPU-KernelAbi: duplicate or omitted live parameter"));
            }
            let expected = parameter.live.then(|| rows.parameter_witnesses.get(&parameter.name.to_string())).flatten();
            let actual = parameter.witness.and_then(|index| self.witnesses.get(index));
            if expected != actual.map(|(generic, _, _)| generic) {
                return Err(error("GPU-KernelAbi: parameter has another preparation witness"));
            }
            if let Some((generic, _, argument)) = actual {
                if !rows.type_witnesses.iter().any(|(ty, _, host)| ty == generic && host == argument) {
                    return Err(error("GPU-KernelAbi: preparation has another host witness"));
                }
            }
            if let Some(witness) = parameter.witness {
                if witness >= self.witnesses.len() || !parameter.live {
                    return Err(error("GPU-KernelAbi: invalid parameter witness"));
                }
                witnesses.insert(witness);
            }
        }
        if witnesses.len() != self.witnesses.len() {
            return Err(error("GPU-KernelAbi: unreferenced preparation witness"));
        }
        let expected = if self.witnesses.is_empty() {
            format_ident!("prepare_{}", self.name)
        } else {
            format_ident!("prepare_{}_for", self.name)
        };
        if self.preparation != expected {
            return Err(error("GPU-KernelAbi: preparation does not match witness ABI"));
        }
        Ok(CheckedKernel(self))
    }
}

impl CheckedKernel {
    /// Lower declarations and a body separately, before printing any kernel.
    /// This scans generated value identifiers for liveness, never Rust types,
    /// generic syntax, generated kernels or host method-call names.
    pub(crate) fn lower(
        name: &syn::Ident,
        attributes: TokenStream,
        declarations: TokenStream,
        body: TokenStream,
        rows: &Rows,
        launch: Launch,
        shared: u32,
    ) -> syn::Result<Self> {
        use syn::parse::Parser;
        let declarations = syn::punctuated::Punctuated::<syn::FnArg, syn::Token![,]>
            ::parse_terminated.parse2(declarations)?;
        let mut uses = BTreeSet::new();
        identifiers(body.clone(), &mut uses);
        let mut parameters = Vec::new();
        let mut witnesses = Vec::new();
        for declaration in declarations {
            let syn::FnArg::Typed(parameter) = declaration else {
                return Err(error("GPU-KernelAbi: scalar parameter required"));
            };
            let syn::Pat::Ident(name) = *parameter.pat else {
                return Err(error("GPU-KernelAbi: named scalar parameter required"));
            };
            let live = uses.contains(&name.ident.to_string());
            let witness = if live {
                rows.parameter_witnesses.get(&name.ident.to_string()).map(|generic| -> syn::Result<usize> {
                    if let Some(index) = witnesses.iter().position(
                        |(ty, _, _): &(syn::Ident, TokenStream, syn::Ident)| ty == generic,
                    ) {
                        return Ok(index);
                    }
                    let record = rows.type_witnesses.iter().find(|(ty, _, _)| ty == generic)
                        .ok_or_else(|| error("GPU-KernelAbi: missing recorded witness"))?;
                    witnesses.push(record.clone());
                    Ok(witnesses.len() - 1)
                }).transpose()?
            } else {
                None
            };
            parameters.push(AbiParameter { name: name.ident, ty: *parameter.ty, live, witness });
        }
        let preparation = if witnesses.is_empty() {
            format_ident!("prepare_{name}")
        } else {
            format_ident!("prepare_{name}_for")
        };
        KernelAbi {
            name: name.clone(), attributes, body, uses, parameters, witnesses,
            preparation, launch, shared,
        }.check(rows)
    }

    fn generics(&self) -> TokenStream {
        let parameters = self.0.witnesses.iter().map(|(ty, bound, _)| quote!(#ty: Copy + #bound));
        if self.0.witnesses.is_empty() { quote!() } else { quote!(<#(#parameters),*>) }
    }

    pub(crate) fn definition(&self) -> TokenStream {
        let KernelAbi { name, attributes, body, .. } = &self.0;
        let generics = self.generics();
        let parameters = self.0.parameters.iter().filter(|parameter| parameter.live);
        let declarations = parameters.clone().map(|AbiParameter { name, ty, .. }| quote!(#name: #ty));
        let layouts = parameters.map(|AbiParameter { ty, .. }| quote! {
            bytes = bytes.next_multiple_of(::core::mem::align_of::<#ty>())
                + ::core::mem::size_of::<#ty>();
        });
        quote! {
            #attributes
            pub fn #name #generics (#(#declarations),*) {
                const {
                    let mut bytes = 0usize;
                    #(#layouts)*
                    assert!(bytes < 4096, "BE-Resource: kernel parameter area must be below 4096 bytes");
                }
                #body
            }
        }
    }

    pub(crate) fn prepare(&self, config: TokenStream) -> TokenStream {
        let method = &self.0.preparation;
        let witnesses = self.0.witnesses.iter().map(|(_, _, witness)| witness);
        quote!(__cq_module.#method(#(&#witnesses,)* #config))
    }

    fn cached(&self, cache: &syn::Ident) -> TokenStream {
        let helper = format_ident!("__cq_prepare_program_{}", self.0.name);
        let witnesses = self.0.witnesses.iter().map(|(_, _, witness)| witness);
        quote!(#helper(__cq_module, &mut *#cache, #(&#witnesses),*))
    }

    /// Bind arguments by recorded position. No method-call rewriting or kernel
    /// reparse is involved; this is the only construction path for launch calls.
    pub(crate) fn call(
        &self,
        stream: TokenStream,
        prepared: TokenStream,
        blocks: TokenStream,
        arguments: TokenStream,
    ) -> syn::Result<TokenStream> {
        use syn::parse::Parser;
        let arguments = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>
            ::parse_terminated.parse2(arguments)?;
        if arguments.len() != self.0.parameters.len() {
            return Err(error("GPU-KernelAbi: launch argument count differs from its record"));
        }
        let arguments = arguments.iter().zip(&self.0.parameters)
            .filter_map(|(argument, parameter)| parameter.live.then_some(argument));
        let name = &self.0.name;
        Ok(match self.0.launch {
            Launch::InvocationOccupancy => quote!(__cq_module.#name(#stream, #prepared, #(#arguments),*)),
            Launch::ProgramOne | Launch::ProgramOccupancy => {
                let helper = format_ident!("__cq_launch_program_{name}");
                quote!(unsafe { #helper(#stream, #prepared, #blocks, #(#arguments),*) })
            }
        })
    }

    fn program_helpers(&self, module: &syn::Ident) -> TokenStream {
        let KernelAbi { name, preparation, shared, launch, .. } = &self.0;
        let prepare = format_ident!("__cq_prepare_program_{name}");
        let launcher = format_ident!("__cq_launch_program_{name}");
        let generics = self.generics();
        let bounds = self.0.witnesses.iter().map(|(ty, bound, _)| quote!(#ty: Copy + #bound));
        let witness_parameters = self.0.witnesses.iter().map(|(ty, _, witness)| quote!(#witness: &#ty));
        let witness_names: Vec<_> = self.0.witnesses.iter().map(|(_, _, witness)| witness).collect();
        let resident = match launch {
            Launch::ProgramOne => quote! {
                let prepared = module.#preparation(#(#witness_names,)* ::cuda_core::LaunchConfig1D::new(1,256,#shared))?;
            },
            Launch::ProgramOccupancy => quote! {
                let probe = module.#preparation(#(#witness_names,)* ::cuda_core::LaunchConfig1D::new(1,256,#shared))?;
                let blocks = ::cq_gpu_runtime::launch_grid_for_kernel(probe.function(),usize::MAX,256,#shared)?;
                let prepared = module.#preparation(#(#witness_names,)* ::cuda_core::LaunchConfig1D::new(blocks,256,#shared))?;
            },
            Launch::InvocationOccupancy => return quote!(),
        };
        let parameters: Vec<_> = self.0.parameters.iter().filter(|parameter| parameter.live).collect();
        let declarations = parameters.iter().map(|AbiParameter { name, ty, .. }| quote!(#name: #ty));
        let marshal = parameters.iter().map(|AbiParameter { name, .. }| quote! {
            let mut #name = #name;
            ::cuda_host::push_kernel_scalar(&mut arguments, &mut #name);
        });
        let count = parameters.len();
        quote! {
            fn #prepare <'__cq_launch, #(#bounds),*> (
                module: &#module::LoadedModule,
                cache: &'__cq_launch mut ::cq_gpu_runtime::ProgramLaunchCache,
                #(#witness_parameters,)*
            ) -> ::cq_gpu_runtime::Result<&'__cq_launch ::cq_gpu_runtime::ProgramLaunch> {
                cache.get_or_prepare(|| {
                    #resident
                    ::cq_gpu_runtime::ProgramLaunch::from_prepared(&prepared)
                })
            }
            unsafe fn #launcher #generics (
                stream: &::cuda_core::CudaStream,
                prepared: &::cq_gpu_runtime::ProgramLaunch,
                blocks: u32,
                #(#declarations),*
            ) -> ::cq_gpu_runtime::Result<()> {
                let mut arguments = ::cuda_host::KernelArgs::<#count>::new();
                #(#marshal)*
                unsafe { prepared.launch(stream, blocks, arguments.as_mut_slice()) }
            }
        }
    }
}

#[derive(Clone)]
enum Operation {
    Map(String, String),
    Lookup {
        provider: String,
        keys: Vec<String>,
        membership: bool,
        negated: bool,
        /// The PH-Smallest probe this range read realizes (GPU-Smallest).
        smallest: Option<String>,
    },
    Flatten,
    Tile,
    Store,
    Fold(Monoid),
    Canon,
}

/// Nest the recorded local partitions, outermost first.
fn nested_partitions(tiles: &[usize], mut body: TokenStream) -> TokenStream {
    for &size in tiles.iter().rev() {
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
    tiles: &[usize],
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
                        smallest: None,
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
                    let smallest = (!membership
                        && self
                            .analysis
                            .gpu_bindings
                            .realization
                            .smallest
                            .contains_key(&chain.result))
                    .then(|| chain.result.clone());
                    chain.operations.push(Operation::Lookup {
                        provider: self.rows.provider_name(&provider, &keys)?,
                        keys: keys.clone(),
                        membership,
                        negated: false,
                        smallest,
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
                    Term::Tile { .. } => Operation::Tile,
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
            Guard::Statistic(_) => {
                unreachable!("BE-StatsRead excludes host statistics from device guards")
            }
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

    /// GPU-RadixEncodedProbe of a radix provider over the leading bound key
    /// fields `keys`, encoded once: binds `(__low, __high, __fits)`. A key
    /// named by `bound` reads that expression instead of its row slot.
    fn radix_probe(
        &self,
        provider: &str,
        keys: &[String],
        bound: Option<(&str, TokenStream)>,
    ) -> syn::Result<TokenStream> {
        let (view, _, order) = self.rows.provider(provider)?;
        if order.len() < keys.len() || order[..keys.len()].iter().any(|key| !keys.contains(key)) {
            return Err(error(
                "GPU-RadixProvider: lookup keys are not a checked leading key prefix",
            ));
        }
        let columns = order[..keys.len()]
            .iter()
            .map(|key| match &bound {
                Some((column, value)) if column == key => Ok(value.clone()),
                _ => {
                    let slot = self.slot(key)?;
                    Ok(quote!(__row[#slot] as i64))
                }
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let count = keys.len() as u32;
        Ok(quote! {
            let __values: [i64; #count as usize] = [#(#columns),*];
            let (__low, __high, __fits) = ::gpu_datalog_runtime::radix_run::device::encode_prefix(
                #view, __values.as_ptr(), #count);
        })
    }

    /// GPU-Smallest selection: per bound row, the exact distinct candidate
    /// count of every recorded side (`__counts`) and the first minimum side
    /// (`__choice`). Each side's encoded prefix interval is `__low_j/__high_j`.
    fn smallest_choice(&self, probe: &str) -> syn::Result<TokenStream> {
        let record = &self.analysis.gpu_bindings.realization.smallest[probe];
        let mut sides = TokenStream::new();
        for (index, side) in record.sides.iter().enumerate() {
            let provider = self.rows.provider_name(&side.provider, &{
                let mut keys = side.prefix.clone();
                keys.push(record.column.clone());
                keys
            })?;
            let view = self.rows.provider(&provider)?.0;
            let probe = self.radix_probe(&provider, &side.prefix, None)?;
            let low = format_ident!("__low_{index}");
            let high = format_ident!("__high_{index}");
            let count = format_ident!("__count_{index}");
            sides.extend(quote! {
                let (#low, #high, #count) = {
                    #probe
                    let __count = if __fits {
                        ::gpu_datalog_runtime::radix_run::device::distinct(#view.keys, __low, __high)
                    } else { 0u64 };
                    (__low, __high, __count)
                };
            });
            if index == 0 {
                sides.extend(quote! { let mut __choice = 0u32; let mut __minimum = #count; });
            } else {
                let index = index as u32;
                sides.extend(quote! {
                    if #count < __minimum { __choice = #index; __minimum = #count; }
                });
            }
        }
        Ok(sides)
    }

    /// The recorded position-tag layout of a smallest probe: `(low bits,
    /// choice mask)` (ChoiceCarrier::PositionTag).
    fn smallest_tag(&self, probe: &str) -> (u32, u32) {
        let record = &self.analysis.gpu_bindings.realization.smallest[probe];
        let cq_lang_sequence::gpu::realization::ChoiceCarrier::PositionTag { choice_bits } =
            record.carrier;
        (31 - choice_bits, (1u32 << choice_bits) - 1)
    }

    /// The radix provider name and full key of every recorded side.
    fn smallest_sides(&self, probe: &str) -> syn::Result<Vec<(String, Vec<String>)>> {
        let record = &self.analysis.gpu_bindings.realization.smallest[probe];
        record
            .sides
            .iter()
            .map(|side| {
                let mut keys = side.prefix.clone();
                keys.push(record.column.clone());
                Ok((self.rows.provider_name(&side.provider, &keys)?, keys))
            })
            .collect()
    }

    /// The smallest probe's parent read: counts and the choice once per row;
    /// the child range carries the choice in its position tag
    /// (ChoiceCarrier::PositionTag). A one-run chosen side starts at its
    /// prefix interval's first key slot (direct), otherwise at ordinal 0.
    fn smallest_lookup(&self, probe: &str) -> syn::Result<TokenStream> {
        let choice = self.smallest_choice(probe)?;
        let (low_bits, _) = self.smallest_tag(probe);
        let mut direct = TokenStream::new();
        for (index, (provider, _)) in self.smallest_sides(probe)?.iter().enumerate() {
            let view = self.rows.provider(provider)?.0;
            let low = format_ident!("__low_{index}");
            let high = format_ident!("__high_{index}");
            let index = index as u32;
            direct.extend(quote! {
                if __choice == #index && #view.keys.run_count == 1 {
                    let (__first, _) = ::gpu_datalog_runtime::radix_run::device::interval(
                        ::gpu_datalog_runtime::radix_run::device::run(#view.keys, 0), #low, #high);
                    __tag = (1u32 << 31) | (__choice << #low_bits) | __first as u32;
                }
            });
        }
        Ok(quote! {
            let (mut __begin, mut __end) = (0u32, 0u32);
            if __live {
                #choice
                let mut __tag = __choice << #low_bits;
                #direct
                __begin = __tag;
                __end = __tag + __minimum as u32;
            }
        })
    }

    /// The smallest probe's child load at tagged position `position`: the
    /// chosen side's candidate key slot (direct, or the tagged ordinal's
    /// distinct key), kept iff every other side holds `(prefix, candidate)`.
    fn smallest_payload(&self, probe: &str, position: TokenStream) -> syn::Result<TokenStream> {
        let record = &self.analysis.gpu_bindings.realization.smallest[probe];
        let column = self.slot(&record.column)?;
        let (low_bits, choice_mask) = self.smallest_tag(probe);
        let mut arms = TokenStream::new();
        let providers = self.smallest_sides(probe)?;
        for (index, (provider, _)) in providers.iter().enumerate() {
            let (view, payload, _) = self.rows.provider(provider)?;
            let radix = self
                .rows
                .radix_provider(provider)
                .ok_or_else(|| error("GPU-Smallest: side is not a recorded radix provider"))?;
            let lane_index = payload
                .iter()
                .position(|column| *column == record.column)
                .ok_or_else(|| error("GPU-Smallest: candidate column is not an item lane"))?;
            let lane = &radix.lanes[lane_index];
            let items = &radix.items;
            let lane_index = lane_index as u32;
            let prefix = self.radix_probe(provider, &record.sides[index].prefix, None)?;
            let mut members = TokenStream::new();
            for (other, (provider, keys)) in providers.iter().enumerate() {
                if other == index {
                    continue;
                }
                let other_view = self.rows.provider(provider)?.0;
                let probe =
                    self.radix_probe(provider, keys, Some((&record.column, quote!(__candidate))))?;
                members.extend(quote! {
                    if __live {
                        #probe
                        let _ = __high;
                        __live = __fits && ::gpu_datalog_runtime::radix_run::device::contains(
                            #other_view.keys, __low);
                    }
                });
            }
            let index = index as u32;
            arms.extend(quote! {
                if __choice == #index {
                    let (__run, __slot) = if __direct {
                        (0u32, __local)
                    } else {
                        #prefix
                        let _ = __fits;
                        ::gpu_datalog_runtime::radix_run::device::distinct_at(
                            #view.keys, __low, __high, __local)
                    };
                    let __first = ::gpu_datalog_runtime::radix_run::device::offset(
                        ::gpu_datalog_runtime::radix_run::device::run(#view.keys, __run), __slot);
                    let __candidate = ::gpu_datalog_runtime::radix_run::device::item::<#lane>(
                        #items, __run, #lane_index, __first) as i64;
                    __row[#column] = __candidate;
                    #members
                }
            });
        }
        Ok(quote! {
            let __tagged = (#position) as u32;
            let __direct = (__tagged >> 31) != 0;
            let __choice = (__tagged >> #low_bits) & #choice_mask;
            let __local = (__tagged & ((1u32 << #low_bits) - 1)) as u64;
            #arms
        })
    }

    /// Every side's bound prefix slot: each candidate's tests re-read them.
    fn smallest_carried(&self, probe: &str) -> syn::Result<BTreeSet<usize>> {
        self.analysis.gpu_bindings.realization.smallest[probe]
            .sides
            .iter()
            .flat_map(|side| &side.prefix)
            .map(|column| self.slot(column))
            .collect()
    }

    fn lookup_columns(&self, provider: &str) -> syn::Result<Vec<String>> {
        if let Some(source) = self.rows.whole_lookups.get(provider) {
            return Ok(self.analysis.structures.physical.headings[source]
                .iter()
                .cloned()
                .collect());
        }
        Ok(self.rows.provider(provider)?.1)
    }

    fn whole_load(&self, source: &str, position: TokenStream) -> syn::Result<TokenStream> {
        let Some((prior, stored)) = self
            .stored
            .iter()
            .enumerate()
            .rev()
            .find(|(_, stored)| stored.result == source || stored.name == source)
        else {
            return self.rows.source(source, position, quote!(__row));
        };
        let rows = format_ident!("__cq_store_rows_{prior}");
        if let Some(columns) = &stored.packed {
            let keys = format_ident!("__cq_store_packed_{prior}");
            let lanes = format_ident!("__cq_stored_lanes_{prior}");
            let mut load = TokenStream::new();
            for (field, column) in columns.iter().enumerate() {
                let slot = self.slot(column)?;
                let base = format_ident!("__cq_stored_base_{prior}_{field}");
                let shift = format_ident!("__cq_stored_shift_{prior}_{field}");
                let bits = format_ident!("__cq_stored_bits_{prior}_{field}");
                let offset = packed_offset(
                    quote!(#keys),
                    quote!((#position) as usize),
                    quote!(#lanes),
                    quote!(#shift),
                    quote!(#bits),
                );
                load.extend(quote!(__row[#slot] = #base.wrapping_add((#offset) as i64);));
            }
            return Ok(load);
        }
        let width = self.rows.width;
        let columns = &self.analysis.structures.physical.headings[&stored.result];
        if let Some(provider) = &stored.range {
            let begin = format_ident!("__cq_store_begin_{prior}");
            let offsets = format_ident!("__cq_store_offsets_{prior}");
            let meta = format_ident!("__cq_store_meta_{prior}");
            let payload_columns = self.lookup_columns(provider)?;
            let copy = columns
                .iter()
                .filter(|column| !payload_columns.contains(column))
                .map(|column| {
                    let slot = self.slot(column)?;
                    Ok(quote!(__row[#slot] = *#rows.add(__whole_parent * #width + #slot);))
                })
                .collect::<syn::Result<Vec<_>>>()?;
            let payload = self.payload(provider, &[], quote!(__whole_position))?;
            return Ok(quote! {
                let __whole_parent = ::cq_gpu_sequence_runtime::device::owner(
                    #offsets, *#meta, (#position) as u32) as usize;
                #(#copy)*
                let __whole_position = *#begin.add(__whole_parent)
                    + ((#position) as u32 - *#offsets.add(__whole_parent));
                #payload
            });
        }
        let copy = columns
            .iter()
            .map(|column| {
                let slot = self.slot(column)?;
                Ok(quote!(__row[#slot] = *#rows.add((#position) as usize * #width + #slot);))
            })
            .collect::<syn::Result<Vec<_>>>()?;
        Ok(quote!(#(#copy)*))
    }

    fn lookup(
        &self,
        provider: &str,
        keys: &[String],
        membership: bool,
    ) -> syn::Result<TokenStream> {
        if let Some(source) = self.rows.whole_lookups.get(provider) {
            let length = if let Some((prior, _)) = self
                .stored
                .iter()
                .enumerate()
                .rev()
                .find(|(_, stored)| stored.result == *source || stored.name == *source)
            {
                let length = format_ident!("__cq_store_length_{prior}");
                quote!(#length)
            } else {
                let length = format_ident!("{provider}_length");
                quote!(#length)
            };
            return Ok(quote! {
                let (mut __begin, mut __end) = (0u32, if __live { (#length) as u32 } else { 0 });
                let __member_found = __begin < __end;
            });
        }
        let (view, payload, order) = self.rows.provider(provider)?;
        if self.rows.radix_provider(provider).is_some() {
            let probe = self.radix_probe(provider, keys, None)?;
            let run = format_ident!("__cq_run_{provider}");
            return Ok(if membership {
                quote! {
                    let mut __member_found = false;
                    if __live {
                        #probe
                        if __fits {
                            let mut __index = 0u32;
                            while __index < #view.keys.run_count {
                                let (__first, __last) = ::gpu_datalog_runtime::radix_run::device::interval(
                                    ::gpu_datalog_runtime::radix_run::device::run(#view.keys, __index),
                                    __low, __high);
                                __member_found |= __first < __last;
                                __index += 1;
                            }
                        }
                    }
                }
            } else {
                // Per run: the run-local item interval of the encoded key interval.
                quote! {
                    let (mut __begin, mut __end) = (0u32, 0u32);
                    if __live {
                        #probe
                        if __fits {
                            let (__first, __last) = ::gpu_datalog_runtime::radix_run::device::interval_items(
                                ::gpu_datalog_runtime::radix_run::device::run(#view.keys, #run),
                                __low, __high);
                            (__begin, __end) = (__first as u32, __last as u32);
                        }
                    }
                }
            });
        }
        if self.rows.run_provider(provider) {
            let probe = self.radix_probe(provider, keys, None)?;
            let run = format_ident!("__cq_run_{provider}");
            return Ok(if membership {
                quote! {
                    let mut __member_found = false;
                    if __live {
                        #probe
                        if __fits {
                            for #run in 0..#view.keys.run_count {
                                let (__begin, __end) = ::gpu_datalog_runtime::radix_run::device::interval(
                                    ::gpu_datalog_runtime::radix_run::device::run(#view.keys, #run), __low, __high);
                                __member_found |= __begin < __end;
                            }
                        }
                    }
                }
            } else {
                quote! {
                    let (mut __begin, mut __end) = (0u32, 0u32);
                    if __live {
                        #probe
                        if __fits {
                            let (__first, __last) = ::gpu_datalog_runtime::radix_run::device::interval(
                                ::gpu_datalog_runtime::radix_run::device::run(#view.keys, #run), __low, __high);
                            if __last > u32::MAX as u64 {
                                ::cq_gpu_sequence_runtime::device::flag_overflow(__meta.add(2));
                            } else {
                                (__begin, __end) = (__first as u32, __last as u32);
                            }
                        }
                    }
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
                    let __key = __row[#slot];
                    let __column = *#view.payload.add(#index);
                    let (mut __low, mut __high) = (__begin, __end);
                    while __low < __high {
                        let __middle = __low + (__high - __low) / 2;
                        // DECODE V: P4 membership reconstructs FOR values for full-width comparison, never truncated codes.
                        if ::cq_gpu_sequence_runtime::device::decode(__column, __middle as u64) < __key {
                            __low = __middle + 1;
                        } else {
                            __high = __middle;
                        }
                    }
                    __member_found = __low < __end
                        && ::cq_gpu_sequence_runtime::device::decode(__column, __low as u64) == __key;
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
            let __directory_key = (__row[#slot] as i64).wrapping_sub(#view.base) as u64;
            if __live && (__row[#slot] as i64) >= #view.base && __directory_key < #view.domain as u64 {
                (__begin, __end) = ::cq_gpu_sequence_runtime::device::range(#view.offsets, __directory_key as u32);
                __member_found = __begin < __end;
            }
            #membership_checks
        })
    }

    fn payload(
        &self,
        provider: &str,
        bound: &[String],
        position: TokenStream,
    ) -> syn::Result<TokenStream> {
        if let Some(source) = self.rows.whole_lookups.get(provider) {
            return self.whole_load(source, position);
        }
        let (view, payload, _) = self.rows.provider(provider)?;
        if let Some(radix) = self.rows.radix_provider(provider) {
            // Item lanes hold native values: a positional read, no decode.
            // Bound key columns stay the parent's carried values.
            let run = format_ident!("__cq_run_{provider}");
            let items = &radix.items;
            let assignments = payload
                .iter()
                .zip(&radix.lanes)
                .enumerate()
                .filter(|(_, (column, _))| !bound.contains(column))
                .map(|(index, (column, lane))| {
                    let slot = self.slot(column)?;
                    let index = index as u32;
                    Ok(quote! {
                        __row[#slot] = ::gpu_datalog_runtime::radix_run::device::item::<#lane>(
                            #items, #run, #index, (#position) as u64) as i64;
                    })
                })
                .collect::<syn::Result<Vec<_>>>()?;
            return Ok(quote!(#(#assignments)*));
        }
        if self.rows.run_provider(provider) {
            let run = format_ident!("__cq_run_{provider}");
            // Full-key set rows are decoded from key descriptors, never item lanes.
            // DECODE V: P6 radix relation payload reconstructs demanded key fields for relational consumers without opaque-Rust read authorization.
            let assignments = payload
                .iter()
                .enumerate()
                .filter(|(_, column)| !bound.contains(column))
                .map(|(index, column)| {
                    let slot = self.slot(column)?;
                    Ok(quote! {
                        __row[#slot] = ::gpu_datalog_runtime::radix_run::device::decode_key(
                            #view, #run, (#position) as u64, #index as u32);
                    })
                })
                .collect::<syn::Result<Vec<_>>>()?;
            return Ok(quote!(#(#assignments)*));
        }
        // DECODE V: P6 provider payload reconstructs FOR values without restricting decode to rust_reads columns.
        let assignments = payload.iter().enumerate().map(|(index,column)| {
            let slot=self.slot(column)?;
            Ok(quote!(__row[#slot] = ::cq_gpu_sequence_runtime::device::decode(*#view.payload.add(#index), (#position) as u64);))
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
    combine: Option<TokenStream>,
    expansion_counters: bool,
    expansion_stages: usize,
    /// The current chain's recorded strips, consumed in pre-order.
    strips: std::collections::VecDeque<Strip>,
    /// The current chain's recorded BE-Resource payload parents
    /// (`ResourceAllocation::Payload`), consumed in pre-order per expansion.
    parents: std::collections::VecDeque<Vec<String>>,
    /// GPU-RootShare: metadata word of the per-tile cursors, taken by the
    /// chain's first expansion.
    share: Option<usize>,
}

impl Kernel<'_, '_> {
    fn next_strip(&mut self) -> syn::Result<Strip> {
        self.strips
            .pop_front()
            .ok_or_else(|| error("GPU-Launch: expansion lacks a recorded strip"))
    }
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
                .filter(|column| !self.compiler.rows.is_capture_output(column))
                .map(|column| self.compiler.slot(column))
                .collect()
        } else {
            let mut slots = self.publication_columns.clone();
            if let Some(provider) = &self.virtual_range {
                // A deferred publication owns the parent and range offsets.
                // Provider payloads become live only when that range is read.
                for column in self.compiler.rows.provider(provider)?.1 {
                    slots.remove(&self.compiler.slot(&column)?);
                }
            }
            Ok(slots)
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
                    smallest,
                    ..
                } => {
                    if !membership {
                        for column in self.compiler.lookup_columns(provider)? {
                            live.remove(&self.compiler.slot(&column)?);
                        }
                    }
                    for column in keys {
                        live.insert(self.compiler.slot(column)?);
                    }
                    if let Some(probe) = smallest {
                        live.extend(self.compiler.smallest_carried(probe)?);
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

    fn code_type(&self, _slot: usize) -> TokenStream {
        quote!(i64)
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
        let dead = DeviceRow::writes(load.clone())?
            .difference(&row.0.keys().copied().collect())
            .copied()
            .collect();
        let load = row.rewrite_with_dead(load, &dead)?;
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
                smallest,
            } => {
                let lookup = match smallest {
                    Some(probe) => self.compiler.smallest_lookup(probe)?,
                    None => self.compiler.lookup(provider, keys, *membership)?,
                };
                let lookup = row.rewrite(lookup)?;
                if *membership {
                    let rest = self.scalar_operations(rest, row)?;
                    // Formal rule RL-Neg: anti membership keeps precisely the rows
                    // whose provider probe finds no witness; dead lanes stay dead.
                    return Ok(
                        quote!(#lookup __live = __live && (__member_found != #negated); #rest),
                    );
                }
                if smallest.is_none()
                    && self.virtual_range.as_deref() == Some(provider)
                    && rest
                        .iter()
                        .all(|op| matches!(op, Operation::Flatten | Operation::Store))
                {
                    let finish = row.rewrite(self.finish()?)?;
                    let stage = self.expansion_stages;
                    self.expansion_stages += usize::from(self.expansion_counters);
                    let count = self.expansion_counters.then(|| quote! {
                        let (_, __children) = ::cq_gpu_sequence_runtime::device::checked_scan(
                            if __live { __end - __begin } else { 0 }, __scratch, __meta.add(2));
                        if __tid == 0 {
                            ::cuda_device::atomic::DeviceAtomicU64::from_ptr(__cq_expansions.add(#stage))
                                .fetch_add(__children as u64, ::cuda_device::atomic::AtomicOrdering::Relaxed);
                        }
                        ::cuda_device::thread::sync_threads();
                    });
                    return Ok(quote!(#lookup #count __live = __live && __begin < __end; #finish));
                }
                if !rest.iter().any(|op| matches!(op, Operation::Flatten)) {
                    return Err(error(
                        "range lookup must be flattened or published as offsets",
                    ));
                }
                let (begin_offset, offsets_offset) = self.shared.payload()?;
                let begins = format_ident!("__begins_{begin_offset}");
                let offsets = format_ident!("__offsets_{offsets_offset}");
                let owner = format_ident!("__owner_{begin_offset}");
                let mut shared = TokenStream::new();
                let mut child = DeviceRow::default();
                // BE-Resource: the recorded payload parents (later uses, live
                // keys and smallest prefixes, without provider payload).
                let parents = self
                    .parents
                    .pop_front()
                    .ok_or_else(|| {
                        error("BE-Resource: expansion lacks its recorded payload parents")
                    })?
                    .iter()
                    .map(|column| self.compiler.slot(column))
                    .collect::<syn::Result<BTreeSet<_>>>()?;
                for slot in &parents {
                    let ty = self.code_type(*slot);
                    let at = self.shared.parent(8)?;
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
                let strip = self.next_strip()?;
                // GPU-RootShare: only the chain's first expansion is shared.
                let share = self.share.take();
                // Formal rule SQ-ExpansionCounters: count children before closing
                // operators, once per block collective, not once per lane.
                let stage = self.expansion_stages;
                self.expansion_stages += usize::from(self.expansion_counters);
                let count_children = self.expansion_counters.then(|| {
                    // A helper's replay of a shared tile must not count twice.
                    let owner_only = share.map(|_| quote!(&& !__share_helping));
                    quote! {
                        if __tid == 0 #owner_only {
                            ::cuda_device::atomic::DeviceAtomicU64::from_ptr(__cq_expansions.add(#stage))
                                .fetch_add(__children as u64, ::cuda_device::atomic::AtomicOrdering::Relaxed);
                        }
                    }
                });
                let tile = strip
                    .rows
                    .ok_or_else(|| error("GPU-Launch: expansion strip lacks its batch rows"))?;
                let payload = match smallest {
                    Some(probe) => self
                        .compiler
                        .smallest_payload(probe, quote!(__child_position))?,
                    None => self
                        .compiler
                        .payload(provider, keys, quote!(__child_position))?,
                };
                let (_, partition, _, _) = self.partition_rows(rest, &strip, StripeInput {
                        items: strip.items,
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
                let children_loop = match share {
                    None => quote! {
                        let mut __child_base = 0u64;
                        while __child_base < __children as u64 {
                            let __partition_begin = __child_base;
                            let __partition_end = (__child_base + #tile as u64).min(__children as u64);
                            #partition
                            __child_base += #tile as u64;
                        }
                    },
                    // GPU-RootShare: child batches of this expansion are numbered
                    // after those of the tile's earlier expansions (`__share_base`)
                    // and claimed exactly once from the tile's cursor. The owner
                    // announces each expansion's batches so idle blocks can help.
                    // A claim beyond this expansion stays pending for the next one.
                    Some(word) => quote! {
                        let __share_batches = (__children as u64).div_ceil(#tile as u64);
                        let __share_word = #word + 2 * __tile as usize;
                        if __share_tile && !__share_helping && __tid == 0 && __share_batches != 0 {
                            let __announced = ::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__share_word + 1))
                                .fetch_add(__share_batches as u32, ::cuda_device::atomic::AtomicOrdering::Relaxed);
                            // List a tile once, when it first has more than one batch.
                            if __announced <= 1 && __announced as u64 + __share_batches > 1 {
                                let __list = #word + 2 * __cq_share_tiles as usize;
                                let __at = ::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__list))
                                    .fetch_add(1, ::cuda_device::atomic::AtomicOrdering::Relaxed);
                                ::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__list + 1 + __at as usize))
                                    .fetch_add(__tile as u32 + 1, ::cuda_device::atomic::AtomicOrdering::Relaxed);
                            }
                        }
                        loop {
                            if __tid == 0 && *__claim_slot == u32::MAX {
                                *__claim_slot = if __share_tile {
                                    ::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__share_word))
                                        .fetch_add(1, ::cuda_device::atomic::AtomicOrdering::Relaxed)
                                } else {
                                    __share_next as u32
                                };
                            }
                            ::cuda_device::thread::sync_threads();
                            let __claim = *__claim_slot as u64;
                            if __claim >= __share_base + __share_batches { break; }
                            if !__share_tile { __share_next += 1; }
                            let __partition_begin = (__claim - __share_base) * #tile as u64;
                            let __partition_end = (__partition_begin + #tile as u64).min(__children as u64);
                            #partition
                            ::cuda_device::thread::sync_threads();
                            if __tid == 0 { *__claim_slot = u32::MAX; }
                            ::cuda_device::thread::sync_threads();
                        }
                        __share_base += __share_batches;
                    },
                };
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
                    #count_children
                    #children_loop
                    ::cuda_device::thread::sync_threads();
                };
                if smallest.is_none() && self.compiler.rows.run_provider(provider) {
                    let view = self.compiler.rows.provider(provider)?.0;
                    let run = format_ident!("__cq_run_{provider}");
                    // Run bounds are uniform descriptor metadata: every lane
                    // executes every run's existing segmented collective.
                    Ok(quote! { for #run in 0..#view.keys.run_count { #expansion } })
                } else if smallest.is_none()
                    && self.compiler.rows.radix_provider(provider).is_some()
                {
                    let view = self.compiler.rows.provider(provider)?.0;
                    let run = format_ident!("__cq_run_{provider}");
                    // GPU-RadixCursor: each run's local item interval is one
                    // segmented collective; runs are uniform descriptor metadata.
                    Ok(quote! { for #run in 0..#view.keys.run_count { #expansion } })
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
    /// The fold's implementation scope is the level's recorded strip.
    fn partition_rows(
        &mut self,
        operations: &[Operation],
        strip: &Strip,
        input_rows: StripeInput,
    ) -> syn::Result<(TokenStream, TokenStream, TokenStream, bool)> {
        let tiles = strip.nested.as_slice();
        let (Some(scope), Some(at)) = (
            &strip.fold,
            operations
                .iter()
                .position(|operation| matches!(operation, Operation::Fold(_))),
        ) else {
            if strip.fold.is_some() {
                return Err(error("GPU-Launch: recorded fold scope has no fold"));
            }
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
        let suffix = self.scalar_operations(&operations[at + 1..], &output_row)?;
        let finish = output_row.rewrite(reduction.finish)?;
        let publish = quote! {
            let mut __fold_result = 0i64;
            let mut __live = false;
            #finish
            #suffix
        };
        if let FoldScope::Tiles(scope) = scope {
            return Ok((
                TokenStream::new(),
                partition_body(tiles, *scope, reduction.declare, body, publish),
                TokenStream::new(),
                false,
            ));
        }
        {
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
            Ok((
                reduction.declare,
                nested_partitions(tiles, body),
                partial,
                true,
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
            let values = packing
                .fields()
                .iter()
                .map(|column| self.compiler.rows.output_value(column))
                .collect::<syn::Result<Vec<_>>>()?;
            return Ok(packing.write(&values));
        }
        let width = self.compiler.rows.width;
        let write = if let Some(columns) = &self.packed_columns {
            let parts = columns
                .iter()
                .enumerate()
                .map(|(i, column)| {
                    let value = self.compiler.rows.output_value(column)?;
                    let base = format_ident!("__cq_pack_base_{i}");
                    let shift = format_ident!("__cq_pack_shift_{i}");
                    let value = quote!((#value as i64).wrapping_sub(#base) as u64);
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
            let slots = self.output_slots()?;
            let columns = slots.iter().map(|slot| quote! {
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

/// Block width of a recorded one-thread-per-element auxiliary launch.
fn element_block(grid: &Grid) -> syn::Result<proc_macro2::Literal> {
    match grid {
        Grid::Elements { block_threads: 256 } => Ok(proc_macro2::Literal::u32_unsuffixed(256)),
        _ => Err(error(
            "GPU-Realization: element kernels use their recorded 256-lane template",
        )),
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

/// Realize a recorded host evaluation: bind the recorded completed
/// publications, download those its transfers name (complete rows), then
/// evaluate and upload the host boundary.
fn publish_host(
    compiler: &mut Compiler<'_>,
    result: &str,
    evaluation: &HostEvaluation,
    launches: &mut Vec<TokenStream>,
    codecs: &crate::runs::Codecs,
) -> syn::Result<()> {
    let program = compiler.program;
    let analysis = compiler.analysis;
    let mut bindings = BTreeMap::new();
    for name in &evaluation.bound {
        let prior = compiler
            .stored
            .iter()
            .position(|stored| stored.name == *name)
            .ok_or_else(|| {
                error("GPU-Transfer: host evaluation binds an incomplete publication")
            })?;
        bindings.insert(
            compiler.stored[prior].result.clone(),
            format_ident!("__cq_downloaded_{prior}"),
        );
    }
    for transfer in &evaluation.transfers {
        let TransferSource::Publication(name) = &transfer.source else {
            continue;
        };
        let Granularity::CompleteRows = transfer.granularity;
        let prior = compiler
            .stored
            .iter()
            .position(|stored| stored.name == *name)
            .ok_or_else(|| error("GPU-Transfer: download names an incomplete publication"))?;
        let stored = &compiler.stored[prior];
        if stored.range.is_some() {
            return Err(error(
                "host consumer requires a materialized row publication",
            ));
        }
        let downloaded = format_ident!("__cq_downloaded_{prior}");
        let length = format_ident!("__cq_store_length_{prior}");
        let rows = format_ident!("__cq_store_rows_{prior}");
        let owner = format_ident!("__cq_canonical_{prior}");
        let fields = format_ident!("__cq_fields_{prior}");
        let mut values = Vec::new();
        let mut types = Vec::new();
        for column in &analysis.structures.layouts[&stored.result] {
            let ty = &analysis.structures.physical.rows[&stored.result].types[column];
            types.push(ty);
            let code = if let Some(columns) = &stored.packed {
                let field = columns
                    .iter()
                    .position(|name| name == column)
                    .ok_or_else(|| error("host consumer column absent from packed publication"))?;
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
            values.push(if let Some(codec) = codecs.column(column) {
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
    }
    launches.push(
        compiler
            .rows
            .publish_host(program, analysis, result, evaluation, &bindings)?,
    );
    Ok(())
}

// DESIGN translator-gpu
// Owns only translation of checked Sequence facts (translator-decides-nothing, T-Gpu).
// Source: doc/PLANNING-REDESIGN.md Architecture; doc/formal/CONFORMANCE.md GPU emitter rows.
// Launch, strip, scan, fold completion, capture, source, transfer, terminal-group and
// publication facts are recorded before printing (GPU-Launch, GPU-Realization).
// DO NOT replan joins, move predicates, guess from generated names, or select codecs/memory policy.
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
    translate_observed(
        program,
        analysis,
        arena,
        std::env::var_os("CQ_EXPANSION_COUNTERS").is_some(),
    )
}

fn translate_observed(
    program: &Program,
    analysis: &Analysis,
    arena: Option<&syn::Expr>,
    expansion_counters: bool,
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
    // The target-owned Structures stage and terminal enforce the same boundary.
    crate::check_structures(&program.structures, &analysis.structures)?;
    cq_lang_sequence::gpu::publication::check(program, analysis, &analysis.gpu_publications)?;
    cq_lang_sequence::gpu::check(program, analysis)?;
    if arena.is_none() && program.sequence.output == Some(OutputProtocol::ReservePrevious) {
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
        let path = std::path::PathBuf::from(directory).join(format!(
            "{:016x}.input.sequence",
            std::hash::Hasher::finish(&fingerprint)
        ));
        std::fs::write(&path, quote!(#program).to_string()).map_err(|err| {
            error(format!(
                "cannot dump Sequence program to {}: {err}",
                path.display()
            ))
        })?;
    }
    let rows = Rows::new(program, analysis)?;
    let codecs =
        crate::runs::Codecs::check(&program.structures, &analysis.structures.physical.rows)?;
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
    let mut program_preparations = Vec::new();
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
    let terminal_record = publication_record(analysis, final_name)?;
    // GPU-Arena: only a recorded raw-candidate terminal publishes into an arena.
    let arena_packing = arena
        .map(|_| {
            terminal_record
                .arena
                .as_ref()
                .map(crate::arena::Packing::new)
                .ok_or_else(|| {
                    error("GPU arena terminal requires a recorded raw-candidate arena encoding")
                })
        })
        .transpose()?;
    // CUDA exports are not scoped by the surrounding Rust expression. Give
    // each expansion its own module, including repeated identical programs.
    static NEXT_MODULE: AtomicUsize = AtomicUsize::new(0);
    let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
    quote!(#program).to_string().hash(&mut fingerprint);
    let expansion = NEXT_MODULE.fetch_add(1, Ordering::Relaxed);
    let module_name = format_ident!(
        "__cq_sequence_kernels_{:016x}_{}",
        fingerprint.finish(),
        expansion
    );
    // Host LLVM inlines a once-called closure into its caller; one program of
    // many holes then becomes one function whose register allocation dominates
    // the build. A non-inlined runner keeps each hole a separate function, and
    // its module selects a separate codegen unit.
    let partition = format_ident!("partition_{}", expansion % 16);
    if let Some(directory) = std::env::var_os("CQ_GPU_SEQUENCE_DUMP") {
        let path = std::path::PathBuf::from(directory).join(format!("{module_name}.sequence"));
        std::fs::write(&path, quote!(#program).to_string()).map_err(|err| {
            error(format!(
                "cannot dump Sequence program to {}: {err}",
                path.display()
            ))
        })?;
    }
    let realization = &analysis.gpu_bindings.realization;
    if realization.steps.len() != program.sequence.definitions.len() {
        return Err(error(
            "GPU-Realization: steps differ from the Sequence definitions",
        ));
    }
    for (definition, (recorded, step)) in
        program.sequence.definitions.iter().zip(&realization.steps)
    {
        let publication = symbol_name(&definition.name);
        if *recorded != publication {
            return Err(error("GPU-Realization: step names another definition"));
        }
        // Formal rule GPU-TerminalGroup: transparent store aliases of the
        // terminal group are realized by its recorded fold, not published.
        if let Step::GroupAlias = step {
            continue;
        }
        if let Step::Group(record) = step {
            let (_, input) = grouped_boundary(&definition.term, &compiler.definitions)
                .ok_or_else(|| error("GPU-TerminalGroup: step is not a grouped publication"))?;
            let mut chains = compiler.fold(input)?;
            let [chain] = chains.as_mut_slice() else {
                return Err(error(
                    "GPU group requires one canonical valuation publication",
                ));
            };
            let result = record.group.clone();
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
            let stored_index = |name: &str| {
                compiler
                    .stored
                    .iter()
                    .position(|stored| stored.name == name)
            };
            let canonical = match &record.input {
                GroupInput::Host { input, upload } => {
                    // The host-graded input is evaluated at its recorded boundary,
                    // using published device operands rather than replaying joins.
                    // Only the numeric grouped-fold representation runs on device.
                    if let Some(evaluation) = upload {
                        publish_host(&mut compiler, input, evaluation, &mut launches, &codecs)?;
                    }
                    chain.source = input.clone();
                    chain.result = input.clone();
                    chain.stored = None;
                    chain.operations.clear();
                    None
                }
                GroupInput::Canonical { publication } => {
                    let index = stored_index(publication)
                        .filter(|index| chain.stored == Some(*index) && chain.operations.is_empty())
                        .ok_or_else(|| {
                            error(
                                "GPU-TerminalGroup: canonical input is not the chain's publication",
                            )
                        })?;
                    let columns = compiler.stored[index]
                        .packed
                        .clone()
                        .ok_or_else(|| error("GPU-TerminalGroup: canonical input is not packed"))?;
                    Some((index, columns))
                }
                GroupInput::Stored { publication } => {
                    if chain.stored.is_none() || chain.stored != stored_index(publication) {
                        return Err(error(
                            "GPU-TerminalGroup: stored input is not the chain's publication",
                        ));
                    }
                    None
                }
                GroupInput::Source { source } => {
                    if chain.stored.is_some() || chain.source != *source {
                        return Err(error(
                            "GPU-TerminalGroup: source input is not the chain's source",
                        ));
                    }
                    None
                }
            };
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
                        && !compiler.rows.query_captures.contains(&symbol_name(name))
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
                let dead = DeviceRow::writes(load.clone())?
                    .difference(&row.0.keys().copied().collect())
                    .copied()
                    .collect();
                let load = row.rewrite_with_dead(load, &dead)?;
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
                let mut arguments = compiler
                    .rows
                    .capture_arguments(&[(capture_definitions, quote!((#length) != 0))]);
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
                .as_ref()
                .map(|(index, columns)| (*index, columns.as_slice()))
                .unwrap_or((0, &[]));
            let kernel = format_ident!("__cq_group_arguments_{module_name}");
            let (code, publication) = match &record.completion {
                GroupCompletion::Segmented {
                    arguments,
                    geometry,
                } => {
                    let Grid::KernelOccupancy = arguments else {
                        return Err(error(
                            "GPU-TerminalGroup: argument extraction uses the occupancy grid",
                        ));
                    };
                    crate::reduce::group(
                        group,
                        analysis
                            .structures
                            .groups
                            .get(&result)
                            .map(|provider| &provider.ty),
                        columns,
                        &compiler.rows.output_columns,
                        &analysis.structures.physical.rows[&result].types,
                        prior,
                        &kernel,
                        direct,
                        &codecs,
                        &compiler.rows.query_captures,
                        geometry,
                        &compiler.rows,
                    )?
                }
                GroupCompletion::GridPartials {
                    partials,
                    geometry,
                    combine,
                } => {
                    let Grid::KernelOccupancy = partials else {
                        return Err(error(
                            "GPU-TerminalGroup: grid partials use the occupancy grid",
                        ));
                    };
                    if analysis.structures.groups.contains_key(&result) {
                        return Err(error(
                            "GPU-Group: global fold cannot carry a keyed accumulator provider",
                        ));
                    }
                    crate::reduce::global(
                        group,
                        columns,
                        &compiler.rows.output_columns,
                        &analysis.structures.physical.rows[&result].types,
                        prior,
                        &format_ident!("__cq_group_partials_{module_name}"),
                        direct,
                        &codecs,
                        &compiler.rows.query_captures,
                        geometry,
                        combine,
                        &compiler.rows,
                    )?
                }
            };
            kernels.push(code);
            output = Some(publication);
            continue;
        }
        let dispatch = analysis.dispatches.iter().enumerate().find(|(_, dispatch)| {
            dispatch.inner_stores.iter().rev().find(|name| {
                compiler
                    .definitions
                    .get(*name)
                    .is_some_and(|term| matches!(term, Term::Store { .. }))
            }) == Some(&publication)
        });
        let index = compiler.stored.len();
        let Some((dispatch_index, dispatch)) = dispatch else {
            if let Step::HostUnion(union) = step {
                let result = &analysis.results[&publication];
                let record = publication_record(analysis, &publication)?;
                let width = record
                    .union_staging
                    .as_ref()
                    .ok_or_else(|| {
                        error("GPU-PublicationStorage: union lacks recorded flat staging")
                    })?
                    .width;
                let store_rows = format_ident!("__cq_store_rows_{index}");
                let store_length = format_ident!("__cq_store_length_{index}");
                let store_begin = format_ident!("__cq_store_begin_{index}");
                let store_offsets = format_ident!("__cq_store_offsets_{index}");
                let store_meta = format_ident!("__cq_store_meta_{index}");
                let mut lengths = Vec::new();
                let mut copies = Vec::new();
                for arm in &union.arms {
                    {
                        let (UnionArm::Copy { publication: name }
                        | UnionArm::Decode {
                            publication: name, ..
                        }) = arm;
                        let prior = compiler
                            .stored
                            .iter()
                            .position(|stored| stored.name == *name)
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
                        if let UnionArm::Decode { grid, .. } = arm {
                            let block = element_block(grid)?;
                            let columns = source.packed.as_ref().ok_or_else(|| {
                                error("GPU-Realization: decoded union arm is not packed")
                            })?;
                            let owner = format_ident!("__cq_canonical_{prior}");
                            let fields = format_ident!("__cq_fields_{prior}");
                            let arm_index = copies.len();
                            let kernel = format_ident!(
                                "__cq_union_decode_{index}_{arm_index}_{module_name}"
                            );
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
                            let abi = CheckedKernel::lower(&kernel, quote! {
                                #[kernel]
                                #[launch_bounds(256)]
                                #[launch_contract(domain=1,block=(256,1,1))]
                            }, quote!(keys:*const u64, rows:*mut i64, length:usize, lanes:usize, #(#params),*), quote! {
                                    unsafe {
                                        let mut position = ::cuda_device::thread::index_1d().get() as usize;
                                        while position < length {
                                            #(#writes)*
                                            position += ::cuda_device::thread::gridDim_x() as usize * #block;
                                        }
                                    }
                            }, &compiler.rows, Launch::InvocationOccupancy, 0)?;
                            kernels.push(abi.definition());
                            let prepare = abi.prepare(quote!(::cuda_core::LaunchConfig1D::new((#length.div_ceil(#block)) as u32,#block,0)));
                            let call = abi.call(quote!(__cq_pool.stream()), quote!(&prepared), quote!(1u32),
                                quote!(#owner.keys().ptr(), unsafe { #store_rows.ptr().add(__cq_union_offset * #width) },
                                    #length, #owner.lanes(), #(#args),*))?;
                            copies.push(quote! {
                                if #length != 0 {
                                    let prepared = #prepare?;
                                    #call?;
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
                let packed_columns = packed_columns(&record.storage);
                if let Some(codec) = record.storage.codec() {
                    let block = element_block(union.pack.as_ref().ok_or_else(|| {
                        error("GPU-Realization: packed union lacks its pack launch")
                    })?)?;
                    let provider = record.storage.canonical_provider();
                    let set_params = set_params(record);
                    let canonical = provider.is_some();
                    let mut fields = Vec::new();
                    let mut params = Vec::new();
                    let mut args = Vec::new();
                    let mut parts = Vec::new();
                    for (field, recipe) in codec.fields().iter().enumerate() {
                        let base = format_ident!("base_{field}");
                        let shift = format_ident!("shift_{field}");
                        let slot = compiler.slot(&recipe.column)?;
                        fields.push(packing_field(recipe, &base, &shift));
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
                    let keys = format_ident!("__cq_store_keys_{index}");
                    let abi = CheckedKernel::lower(&kernel, quote! {
                        #[kernel]
                        #[launch_bounds(256)]
                        #[launch_contract(domain=1,block=(256,1,1))]
                    }, quote!(rows:*const i64,keys:*mut u64,length:usize,lanes:usize,#(#params),*), quote! {
                            unsafe {
                                let mut position = ::cuda_device::thread::index_1d().get() as usize;
                                while position < length {
                                    let packed = keys.add(position * lanes);
                                    for lane in 0..lanes { *packed.add(lane) = 0; }
                                    #(#parts)*
                                    position += ::cuda_device::thread::gridDim_x() as usize * #block;
                                }
                            }
                    }, &compiler.rows, Launch::InvocationOccupancy, 0)?;
                    kernels.push(abi.definition());
                    let prepare = abi.prepare(quote!(::cuda_core::LaunchConfig1D::new(#store_length.div_ceil(#block) as u32,#block,0)));
                    let call = abi.call(quote!(__cq_pool.stream()), quote!(&prepared), quote!(1u32),
                        quote!(#store_rows.ptr(),#keys.ptr(),#store_length,__cq_key_lanes,#(#args),*))?;
                    let lanes = key_lanes(codec);
                    launches.push(quote! {
                        let mut __cq_key_bits = 0u32;
                        let mut __cq_fields = ::std::vec::Vec::new();
                        #(#fields)*
                        let __cq_key_lanes = #lanes;
                        let #keys = unsafe { ::cq_gpu_runtime::Buffer::<u64>::uninitialized(__cq_pool.clone(),#store_length * __cq_key_lanes)? };
                        if #store_length != 0 {
                            let prepared = #prepare?;
                            #call?;
                        }
                    });
                    let owner = format_ident!("__cq_canonical_{index}");
                    let saved_fields = format_ident!("__cq_fields_{index}");
                    let canonicalize = quote! {
                        let #owner = unsafe { <#provider>::build_set(
                            &__cq_pool, &#keys, #store_length, __cq_key_bits, #set_params)? };
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
                        result,
                        &publication,
                        index,
                        packed_columns.as_deref(),
                        union.provider.as_ref(),
                    )?);
                }
                compiler.stored.push(Stored {
                    name: publication,
                    result: result.clone(),
                    range: None,
                    packed: packed_columns,
                });
            } else if let Step::Upload {
                evaluation,
                provider: None,
            } = step
            {
                let result = analysis
                    .results
                    .get(&publication)
                    .ok_or_else(|| error("GPU-Realization: upload lacks its checked result"))?;
                publish_host(&mut compiler, result, evaluation, &mut launches, &codecs)?;
            } else if !matches!(step, Step::Host) {
                return Err(error(
                    "GPU-Realization: step contradicts the checked dispatches",
                ));
            }
            continue;
        };
        let Step::Dispatch {
            result: dispatch_result,
            provider: dispatch_provider,
        } = step
        else {
            return Err(error(
                "GPU-Realization: checked dispatch lacks its dispatch step",
            ));
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
        let record = publication_record(analysis, &publication)?;
        let provider = record.storage.canonical_provider();
        let set_params = set_params(record);
        let canonical = provider.is_some();
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
                            "GPU nested canon requires a segmented set build; full-row sets certify complete flat publications",
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
        // A chain's Canon operation is realized only by its publication's recorded provider.
        if !canonical
            && chains.iter().any(|chain| {
                chain
                    .operations
                    .iter()
                    .any(|op| matches!(op, Operation::Canon))
            })
        {
            return Err(error(
                "GPU-PublicationStorage: canon operation lacks a recorded canonical publication",
            ));
        }
        let execution = dispatch
            .gpu
            .as_ref()
            .ok_or_else(|| error("GPU dispatch lacks checked launch refinement"))?;
        let launch = realization.launches[dispatch_index];
        if execution.block_threads != 256 {
            return Err(error(
                "GPU-Launch: this collective template requires 256 lanes",
            ));
        }
        let cq_lang_sequence::gpu::Reservation::StripeBatch = execution.reservation;
        if execution.chains.len() != chains.len() {
            return Err(error(
                "GPU-Launch: chain records differ from the publication",
            ));
        }
        let virtual_range = match &execution.publication {
            cq_lang_sequence::gpu::Publication::Rows => None,
            cq_lang_sequence::gpu::Publication::DeferredRange { provider, keys } => {
                Some(compiler.rows.provider_name(provider, keys)?)
            }
        };
        let packed_columns = packed_columns(&record.storage);
        let local_packing = record
            .storage
            .codec()
            .filter(|_| !(arena.is_some() && final_output))
            .map(packing_fields)
            .unwrap_or_default();
        let kernel_name = format_ident!("__cq_sequence_dispatch_{index}_{module_name}");
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
            let length = format_ident!("__cq_store_length_{prior}");
            parameters.push(quote!(#length:usize));
            arguments.push(quote!(#length));
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
        let mut whole_lengths = BTreeSet::new();
        for operation in chains.iter().flat_map(|chain| &chain.operations) {
            if let Operation::Lookup { provider, .. } = operation
                && let Some(source) = compiler.rows.whole_lookups.get(provider)
                && !compiler
                    .stored
                    .iter()
                    .any(|stored| stored.result == *source || stored.name == *source)
                && whole_lengths.insert(provider)
            {
                let length = format_ident!("{provider}_length");
                let value = compiler.rows.source_len(source)?;
                parameters.push(quote!(#length:usize));
                arguments.push(quote!(#value));
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
        if expansion_counters {
            parameters.push(quote!(__cq_expansions: *mut u64));
            arguments.push(quote!(__cq_expansions.ptr()));
        }
        let mut generator = Kernel {
            compiler: &compiler,
            shared: crate::layout::SharedLayout::new(),
            virtual_range: virtual_range.clone(),
            packed_columns: packed_columns.clone(),
            arena_packing: if final_output {
                arena_packing.as_ref()
            } else {
                None
            },
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
            combine: None,
            expansion_counters,
            expansion_stages: 0,
            strips: Default::default(),
            parents: Default::default(),
            share: None,
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
                    if compiler.rows.radix_provider(provider).is_some() {
                        return Err(error(
                            "GPU-RadixProvider: a deferred range publication needs a contiguous range provider",
                        ));
                    }
                    let payload = compiler.payload(provider, &[], quote!(__source_position))?;
                    let payload_columns = compiler.rows.provider(provider)?.1;
                    let copy = analysis.structures.physical.headings
                        [&compiler.stored[prior].result]
                        .iter()
                        .filter(|column| !payload_columns.contains(column))
                        .map(|column| {
                            let slot = compiler.slot(column)?;
                            Ok(quote! {
                                __row[#slot] = *#rows.add(__parent * #width + #slot);
                            })
                        })
                        .collect::<syn::Result<Vec<_>>>()?;
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
            generator.strips = execution.chains[chain_index]
                .strips
                .iter()
                .cloned()
                .collect();
            generator.parents = analysis
                .resources
                .get(&publication)
                .and_then(|resource| resource.shape.chains.get(chain_index))
                .into_iter()
                .flatten()
                .filter_map(|allocation| match allocation {
                    cq_lang_sequence::ResourceAllocation::Payload { parents } => {
                        Some(parents.iter().map(|column| column.name.clone()).collect())
                    }
                    cq_lang_sequence::ResourceAllocation::Fold => None,
                })
                .collect();
            let strip = generator.next_strip()?;
            if strip.rows.is_some() {
                return Err(error(
                    "GPU-Launch: source strip uses the dispatch partition",
                ));
            }
            let tile = match execution.partition {
                cq_lang_sequence::gpu::Partition::Tiles { rows } => quote!(#rows),
                cq_lang_sequence::gpu::Partition::One => quote!((#domain as u64).max(1)),
                // GPU-RootSpread: the launch passes the tile sized from occupancy.
                cq_lang_sequence::gpu::Partition::Spread { .. } => {
                    let tile = format_ident!("__cq_tile_{chain_index}");
                    parameters.push(quote!(#tile:u32));
                    arguments.push(quote!(#tile));
                    quote!(#tile)
                }
            };
            let share_word = 4 + chains.len();
            let share = matches!(
                execution.share,
                cq_lang_sequence::gpu::Share::FirstExpansion { .. }
            );
            if share {
                parameters.push(quote!(__cq_share_tiles:u32));
                arguments.push(quote!(__cq_share_tiles));
                generator.share = Some(share_word);
            }
            let (outside_declare, partition, outside_finish, whole) = generator.partition_rows(
                &chain.operations,
                &strip,
                StripeInput {
                    items: strip.items,
                    inherited: DeviceRow::default(),
                    prelude: TokenStream::new(),
                    load: source,
                },
            )?;
            if !generator.strips.is_empty() {
                return Err(error("GPU-Launch: unconsumed strip records"));
            }
            if generator.share.take().is_some() {
                return Err(error(
                    "GPU-RootShare: the chain has no shared first expansion",
                ));
            }
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
            let root_loop = if share {
                // GPU-RootShare: blocks first claim root tiles; once those are
                // exhausted, each block visits the shared tiles once and replays
                // any whose announced child batches are not all claimed yet.
                quote! {
                    let __claim_slot=__shared.add(44).cast::<u32>();
                    let mut __share_helping=false;
                    let mut __share_visit=0u64;
                    loop {
                        let __tile = if !__share_helping {
                            if __tid==0 {
                                *__tile_slot=::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(#cursor))
                                    .fetch_add(1,::cuda_device::atomic::AtomicOrdering::Relaxed);
                            }
                            ::cuda_device::thread::sync_threads();
                            let __tile=*__tile_slot as u64;
                            ::cuda_device::thread::sync_threads();
                            if __tile*#tile as u64>=#domain as u64 { __share_helping=true; continue; }
                            __tile
                        } else {
                            // Only tiles their owner listed as hot (more than one
                            // batch in an expansion) are visited, in list order.
                            let __list=#share_word+2*__cq_share_tiles as usize;
                            if __tid==0 {
                                let __listed=::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__list))
                                    .fetch_add(0,::cuda_device::atomic::AtomicOrdering::Relaxed) as u64;
                                *__tile_slot=0;
                                if __share_visit<__listed {
                                    let __entry=::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__list+1+__share_visit as usize))
                                        .fetch_add(0,::cuda_device::atomic::AtomicOrdering::Relaxed);
                                    if __entry!=0 {
                                        let __word=#share_word+2*(__entry-1) as usize;
                                        let __claimed=::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__word))
                                            .fetch_add(0,::cuda_device::atomic::AtomicOrdering::Relaxed);
                                        let __known=::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__word+1))
                                            .fetch_add(0,::cuda_device::atomic::AtomicOrdering::Relaxed);
                                        *__tile_slot=if __claimed<__known {__entry} else {u32::MAX};
                                    } else {
                                        *__tile_slot=u32::MAX;
                                    }
                                }
                            }
                            ::cuda_device::thread::sync_threads();
                            let __entry=*__tile_slot;
                            ::cuda_device::thread::sync_threads();
                            if __entry==0 {break;}
                            __share_visit+=1;
                            if __entry==u32::MAX {continue;}
                            (__entry-1) as u64
                        };
                        let __share_tile=__tile<__cq_share_tiles as u64;
                        let mut __share_base=0u64;
                        let mut __share_next=0u64;
                        if __tid==0 { *__claim_slot=u32::MAX; }
                        ::cuda_device::thread::sync_threads();
                        let __partition_begin=__tile*#tile as u64;
                        let __partition_end=(__partition_begin+#tile as u64).min(#domain as u64);
                        #partition
                    }
                }
            } else {
                quote! {
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
                }
            };
            bodies.push(quote! {
                if #enabled {
                    let __partial_base = #chain_index * __blocks as usize;
                    #outside_declare
                    #root_loop
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
        // Formal rule GPU-ResidentMeta: the dispatch's words are the arena's
        // resident words, zero on entry. Every block fences its metadata
        // updates before counting itself finished; the last block publishes
        // the count and failure words and re-zeroes every word the launch used.
        let resident = execution.meta == cq_lang_sequence::gpu::Meta::Resident;
        if resident && !(arena.is_some() && final_output) {
            return Err(error(
                "GPU-ResidentMeta: resident metadata requires a final GPU arena dispatch",
            ));
        }
        let (resident_parameter, resident_epilogue) = if resident {
            let used_words = 4 + chains.len();
            let share_used = matches!(
                execution.share,
                cq_lang_sequence::gpu::Share::FirstExpansion { .. }
            )
            .then(|| quote!(+ 3 * __cq_share_tiles as usize + 1));
            let finished = RESIDENT_FINISHED;
            let result_words = RESIDENT_RESULT;
            (
                quote!(__meta_result:*mut u32,),
                quote! {
                    ::cuda_device::threadfence();
                    ::cuda_device::thread::sync_threads();
                    if __tid==0 {
                        let __finished=::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta_result.add(#finished))
                            .fetch_add(1,::cuda_device::atomic::AtomicOrdering::AcqRel);
                        *__tile_slot=(__finished+1==__blocks) as u32;
                    }
                    ::cuda_device::thread::sync_threads();
                    if *__tile_slot!=0 {
                        ::cuda_device::threadfence();
                        if __tid<#result_words {
                            *__meta_result.add(__tid)=::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__tid))
                                .fetch_add(0,::cuda_device::atomic::AtomicOrdering::Relaxed);
                        }
                        ::cuda_device::thread::sync_threads();
                        let mut __word=__tid;
                        while __word<#used_words #share_used {
                            ::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta.add(__word))
                                .store(0,::cuda_device::atomic::AtomicOrdering::Relaxed);
                            __word+=256;
                        }
                        if __tid==0 {
                            ::cuda_device::atomic::DeviceAtomicU32::from_ptr(__meta_result.add(#finished))
                                .store(0,::cuda_device::atomic::AtomicOrdering::Relaxed);
                        }
                    }
                },
            )
        } else {
            (quote!(), quote!())
        };
        let dispatch_abi = CheckedKernel::lower(&kernel_name, quote! {
            #[kernel]
            #[launch_bounds(256)]
            #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(#shared_u32,#shared_u32))]
        }, quote!(#(#parameters,)* __output_rows:*mut i64,__output_begin:*mut u32,
            __output_offsets:*mut u32,__output_keys:*mut u64,__capacity:u32,__meta:*mut u32,
            #resident_parameter __partials:*mut i64,__blocks:u32), quote! {
                unsafe {
                    let __tid=::cuda_device::thread::threadIdx_x() as usize;
                    let __shared=::cuda_device::DynamicSharedArray::<u8,16>::get_raw();
                    let __scratch=__shared.cast::<u32>();
                    let __reservation=__shared.add(32).cast::<u32>();
                    let __tile_slot=__shared.add(36).cast::<u32>();
                    let __abort_slot=__shared.add(40).cast::<u32>();
                    #(#bodies)*
                    #resident_epilogue
                }
        }, &compiler.rows, launch, shared_u32)?;
        kernels.push(dispatch_abi.definition());
        let combine_kernel = format_ident!("__cq_sequence_combine_{index}_{module_name}");
        let whole_reduction = matches!(
            execution.fold,
            cq_lang_sequence::gpu::FoldCompletion::BlockPartials { .. }
        );
        let combine_abi = if whole_reduction {
            let abi = CheckedKernel::lower(&combine_kernel, quote! {
                #[kernel]
                #[launch_bounds(256)]
                #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(#shared_u32,#shared_u32))]
            }, quote!(#(#parameters,)* __output_rows:*mut i64,__output_begin:*mut u32,
                __output_offsets:*mut u32,__output_keys:*mut u64,__capacity:u32,__meta:*mut u32,
                __partials:*mut i64,__blocks:u32), quote! {
                    unsafe {
                        let __tid=::cuda_device::thread::threadIdx_x() as usize;
                        let __shared=::cuda_device::DynamicSharedArray::<u8,16>::get_raw();
                        let __scratch=__shared.cast::<u32>();
                        let __reservation=__shared.add(32).cast::<u32>();
                        let __abort_slot=__shared.add(40).cast::<u32>();
                        #(#combine_bodies)*
                    }
            }, &compiler.rows, if launch == Launch::InvocationOccupancy {
                Launch::InvocationOccupancy
            } else { Launch::ProgramOne }, shared_u32)?;
            kernels.push(abi.definition());
            Some(abi)
        } else { None };
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
            // Ask the occupancy query for the full resident capacity first.
            cq_lang_sequence::gpu::Partition::Spread { .. } => quote!(usize::MAX),
        };
        // GPU-RootSpread: size each chain's root tile from the resident block
        // count, then launch one block per tile up to that capacity.
        let spread = match execution.partition {
            cq_lang_sequence::gpu::Partition::Spread { max_rows } => {
                let tiles = (0..chains.len()).map(|i| {
                    let domain = format_ident!("__cq_domain_{i}");
                    let tile = format_ident!("__cq_tile_{i}");
                    quote!(let #tile=::cq_gpu_runtime::spread_tile(#domain,__cq_grid,#max_rows);)
                });
                let counts = (0..chains.len()).map(|i| {
                    let domain = format_ident!("__cq_domain_{i}");
                    let tile = format_ident!("__cq_tile_{i}");
                    quote!((#domain as usize).div_ceil(#tile as usize))
                });
                match execution.share {
                    // GPU-RootShare: every resident block may help a hot tile,
                    // so the grid stays at the resident capacity.
                    cq_lang_sequence::gpu::Share::FirstExpansion { tiles: cap } => quote! {
                        #(#tiles)*
                        let __cq_share_tiles=((__cq_domain_0 as usize).div_ceil(__cq_tile_0 as usize)).min(#cap) as u32;
                    },
                    cq_lang_sequence::gpu::Share::None => quote! {
                        #(#tiles)*
                        let __cq_grid=(0usize #(.max(#counts))*).clamp(1,__cq_grid as usize) as u32;
                    },
                }
            }
            _ => quote!(),
        };
        // GPU-RootShare: per shared tile a claim cursor and announced batches,
        // then the hot-tile list count and entries.
        let share_words = matches!(
            execution.share,
            cq_lang_sequence::gpu::Share::FirstExpansion { .. }
        )
        .then(|| quote!(+ 3 * __cq_share_tiles as usize + 1));
        use cq_lang_sequence::gpu::InitialCapacity;
        let initial = match (&execution.capacity, capacity) {
            (InitialCapacity::Zero, _) => quote!(0usize),
            // Formal rule GPU-ReservePrevious: a recorded previous-count hint is
            // not a bound; validate full count and failure flags, then replay any
            // undersized attempt against unchanged inputs before publication.
            (InitialCapacity::PreviousCount, _) if arena.is_some() && final_output => {
                quote!(__cq_arena.previous_count())
            }
            (InitialCapacity::PreviousCount, _) => {
                return Err(error(
                    "reserve_previous requires a final GPU arena dispatch",
                ));
            }
            (InitialCapacity::Declared, Some(capacity)) => quote!(#capacity),
            (InitialCapacity::Declared, None) => {
                return Err(error(
                    "GPU-Launch: declared capacity is absent from the store",
                ));
            }
            (InitialCapacity::Rows(rows), _) => quote!(#rows),
        };
        let (row_width, key_width) = match &record.storage {
            PublicationStorage::FlatRows(flat) => (flat.width, 0usize),
            PublicationStorage::PackedKeys(_) => (0, 1),
        };
        let range_width = usize::from(virtual_range.is_some());
        let complete_offsets = match execution.offsets {
            cq_lang_sequence::gpu::OffsetScan::None => quote!(__cq_required),
            cq_lang_sequence::gpu::OffsetScan::DeviceExclusive => quote!(
                ::cq_gpu_sequence_runtime::offsets::scan(&__cq_pool, &#store_offsets, __cq_required, &#store_meta, __cq_auxiliary.offsets)?
            ),
        };
        // GPU-ReservePrevious: a keys-only arena attempt uses the leased slot's
        // full capacity, which is at least the hint; replay still follows a
        // validated shortfall.
        let key_allocation = if arena.is_some()
            && final_output
            && row_width == 0
            && range_width == 0
        {
            quote! {
                let #output_keys = __cq_arena.reserve(__cq_arena_packing.clone(), __cq_capacity)?;
                __cq_capacity = #output_keys.capacity();
            }
        } else if arena.is_some() && final_output {
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
        let lanes = record
            .storage
            .codec()
            .map(key_lanes)
            .unwrap_or_else(|| quote!(1usize));
        let packing_setup = if arena_final {
            quote!()
        } else {
            quote! {
                let mut __cq_key_bits=0u32;
                let mut __cq_fields=::std::vec::Vec::<::cq_gpu_sequence_runtime::canon::PackedField>::new();
                #(#local_packing)*
                let __cq_key_lanes=#lanes;
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
                quote!(
                    ::core::ptr::null_mut::<i64>(),
                    ::core::ptr::null_mut::<u32>(),
                    ::core::ptr::null_mut::<u32>(),
                ),
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
        let launch_setup = match launch {
            Launch::InvocationOccupancy => {
                let probe = dispatch_abi.prepare(quote!(::cuda_core::LaunchConfig1D::new(1,256,#shared_u32)));
                let prepare = dispatch_abi.prepare(quote!(::cuda_core::LaunchConfig1D::new(__cq_grid,256,#shared_u32)));
                quote! {
                    let __cq_probe = #probe?;
                    let __cq_grid=::cq_gpu_runtime::launch_grid_for_kernel(
                        __cq_probe.function(), #parts, 256, #shared_u32)?;
                    #spread
                    let __cq_prepared = #prepare?;
                }
            },
            Launch::ProgramOne | Launch::ProgramOccupancy => {
                let slot = program_preparations.len();
                let cache = format_ident!("__cq_launch_cache_{slot}");
                let cached = dispatch_abi.cached(&cache);
                program_preparations.push(dispatch_abi.program_helpers(&module_name));
                quote! {
                    let __cq_prepared = #cached?;
                    let __cq_grid=(#parts).max(1).min(__cq_prepared.resident_blocks() as usize) as u32;
                    #spread
                }
            }
        };
        let combine_launch = if let Some(abi) = &combine_abi {
            let preparation = if launch == Launch::InvocationOccupancy {
                abi.prepare(quote!(::cuda_core::LaunchConfig1D::new(1,256,#shared_u32)))
            } else {
                let slot = program_preparations.len();
                let cache = format_ident!("__cq_launch_cache_{slot}");
                program_preparations.push(abi.program_helpers(&module_name));
                abi.cached(&cache)
            };
            let call = abi.call(quote!(__cq_pool.stream()), quote!(&prepared), quote!(1u32),
                quote!(#(#arguments,)* #pointers #key_pointer, __cq_capacity as u32, #store_meta.ptr(),
                    __cq_partials.ptr(), __cq_grid))?;
            quote! {
                let prepared = #preparation?;
                #call?;
            }
        } else { quote!() };
        let count_readback = execution
            .synchronization
            .iter()
            .find_map(|point| match point {
                cq_lang_sequence::gpu::Synchronization::CountAndFailure => Some(if resident {
                    // GPU-ResidentMeta: the last block published the result words.
                    quote!(let __cq_counts=__cq_resident.to_host_prefix(#RESIDENT_RESULT)?;)
                } else if share_words.is_some() {
                    // GPU-RootShare cursors stay on the device.
                    quote!(let __cq_counts=#store_meta.to_host_prefix(#meta_words)?;)
                } else {
                    quote!(let __cq_counts=#store_meta.to_host()?;)
                }),
                cq_lang_sequence::gpu::Synchronization::RangeOffsets => None,
            })
            .ok_or_else(|| {
                error("GPU-Launch: output protocol lacks a count/failure synchronization")
            })?;
        // GPU-ResidentMeta: a resident dispatch neither allocates nor clears
        // per-attempt metadata, and its local fold has no block partials.
        let (
            partials_setup,
            partials_pointer,
            meta_setup,
            meta_allocation,
            meta_pointers,
            meta_value,
        ) = if resident {
            (
                quote!(),
                quote!(::core::ptr::null_mut::<i64>()),
                quote!(let __cq_resident=__cq_arena.resident_words(#RESIDENT_HEADER + #meta_words #share_words)?;),
                quote!(),
                quote!(__cq_resident.ptr().wrapping_add(#RESIDENT_HEADER),__cq_resident.ptr(),),
                quote!(()),
            )
        } else {
            (
                quote! {
                    let __cq_partials=::cq_gpu_runtime::Buffer::<i64>::zeroed(
                        __cq_pool.clone(), __cq_grid as usize * #partial_count)?;
                },
                quote!(__cq_partials.ptr()),
                quote!(),
                quote!(let #store_meta=::cq_gpu_runtime::Buffer::<u32>::zeroed(__cq_pool.clone(),#meta_words #share_words)?;),
                quote!(#store_meta.ptr(),),
                quote!(#store_meta),
            )
        };
        let expansion_stages = generator.expansion_stages;
        let allocate_expansions = expansion_counters.then(|| {
            quote! {
                let __cq_expansions = ::cq_gpu_runtime::Buffer::<u64>::zeroed(
                    __cq_pool.clone(), #expansion_stages)?;
            }
        });
        let report_expansions = expansion_counters.then(|| {
            quote! {
                ::cq_gpu_runtime::expansions::publish(
                    stringify!(#module_name), #publication,
                    &__cq_expansions.to_host()?, __cq_required as u64);
            }
        });
        let dispatch_call = dispatch_abi.call(quote!(__cq_pool.stream()), quote!(&__cq_prepared),
            quote!(__cq_grid), quote!(#(#arguments,)* #pointers #key_pointer,
                __cq_capacity as u32, #meta_pointers #partials_pointer, __cq_grid))?;
        launches.push(quote! {
            #packing_setup
            #(#host_domains)*
            #launch_setup
            #partials_setup
            #meta_setup
            let mut __cq_capacity=#initial;
            let (#stores #output_keys,#store_meta,#store_length)=loop {
                if __cq_capacity>u32::MAX as usize {return Err("Sequence output exceeds u32 positions".into());}
                #allocations
                #key_allocation
                #meta_allocation
                #allocate_expansions
                #dispatch_call?;
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
                #report_expansions
                let __cq_logical = #complete_offsets;
                break (#stores #output_keys,#meta_value,__cq_logical);
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
                    let __cq_keys=unsafe {<#provider>::build_set(
                        &__cq_pool, &#output_keys, #store_length, __cq_key_bits, #set_params)?};
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
                let #canonical_owner=unsafe {<#provider>::build_set(
                    &__cq_pool, &#output_keys, #store_length, __cq_key_bits, #set_params)?};
                let #store_length=#canonical_owner.logical_count()?;
                let #fields=__cq_fields;
                drop(#output_keys);
            });
        }
        if !final_output {
            launches.push(compiler.rows.publish_provider(
                analysis,
                dispatch_result,
                &publication,
                index,
                packed_columns.as_deref(),
                dispatch_provider.as_ref(),
            )?);
        }
        compiler.stored.push(Stored {
            name: publication.clone(),
            result: dispatch_result.clone(),
            range: virtual_range,
            packed: packed_columns,
        });
    }
    if output.is_none() {
        let provider = terminal_record.storage.canonical_provider();
        let set_params = set_params(terminal_record);
        let canonical = provider.is_some();
        // A checked host publication may be the complete result. Its transfer
        // to the device is a storage adapter, not an additional row dispatch.
        let source = realization.output.as_ref();
        if let Some(HostOutput { source, pack }) = source {
            let block = element_block(pack)?;
            let length = compiler.rows.source_len(source)?;
            let load = compiler
                .rows
                .source(source, quote!(__position), quote!(__row))?;
            let mut row = DeviceRow::default();
            let mut declarations = TokenStream::new();
            for column in &compiler.rows.output_columns {
                if compiler.rows.is_capture_output(column) {
                    continue;
                }
                let slot = compiler.slot(column)?;
                let code = format_ident!("__code_{slot}");
                declarations.extend(quote!(let mut #code = 0i64;));
                row.0.insert(slot, quote!(#code));
            }
            let dead = DeviceRow::writes(load.clone())?
                .difference(&row.0.keys().copied().collect())
                .copied()
                .collect();
            let load = row.rewrite_with_dead(load, &dead)?;
            let mut parameters = Vec::new();
            let mut arguments = compiler.rows.capture_arguments(&[]);
            for (name, ty, _) in &compiler.rows.parameters {
                parameters.push(quote!(#name:#ty));
            }
            let codec = terminal_record
                .storage
                .codec()
                .ok_or_else(|| error("GPU-PublicationStorage: host output lacks recorded keys"))?;
            let packing = packing_fields(codec);
            for i in 0..codec.fields().len() {
                let base = format_ident!("__cq_pack_base_{i}");
                let shift = format_ident!("__cq_pack_shift_{i}");
                parameters.extend([quote!(#base:i64), quote!(#shift:u32)]);
                arguments.extend([quote!(#base), quote!(#shift)]);
            }
            let generator = Kernel {
                compiler: &compiler,
                shared: crate::layout::SharedLayout::new(),
                virtual_range: None,
                packed_columns: packed_columns(&terminal_record.storage),
                arena_packing: None,
                publication_columns: BTreeSet::new(),
                sink: None,
                combine: None,
                expansion_counters: false,
                expansion_stages: 0,
                strips: Default::default(),
                parents: Default::default(),
                share: None,
            };
            let write = row.rewrite(generator.output_write()?)?;
            let kernel = format_ident!("__cq_host_output_{module_name}");
            let abi = CheckedKernel::lower(&kernel, quote! {
                #[kernel]
                #[launch_bounds(256)]
                #[launch_contract(domain=1,block=(256,1,1))]
            }, quote!(#(#parameters,)* __output_keys:*mut u64,
                __length:usize,__cq_key_lanes:usize), quote! {
                    unsafe {
                        let __position = ::cuda_device::thread::index_1d().get() as usize;
                        if __position < __length {
                            #declarations
                            #load
                            let __destination = __position;
                            #write
                        }
                    }
            }, &compiler.rows, Launch::InvocationOccupancy, 0)?;
            kernels.push(abi.definition());
            let prepare = abi.prepare(quote!(::cuda_core::LaunchConfig1D::new(grid,#block,0)));
            let call = abi.call(quote!(__cq_pool.stream()), quote!(&prepared), quote!(grid),
                quote!(#(#arguments,)* __cq_host_output_keys.ptr(),__cq_host_output_length,__cq_key_lanes))?;
            let lanes = key_lanes(codec);
            launches.push(quote! {
                let __cq_host_output_length = #length;
                let mut __cq_key_bits = 0u32;
                let mut __cq_fields = ::std::vec::Vec::new();
                #(#packing)*
                let __cq_key_lanes = #lanes;
                let __cq_host_output_keys = unsafe {
                    ::cq_gpu_runtime::Buffer::<u64>::uninitialized(__cq_pool.clone(),
                        __cq_host_output_length.checked_mul(__cq_key_lanes).ok_or("Sequence output allocation overflow")?)?
                };
                if __cq_host_output_length != 0 {
                    let grid = u32::try_from(__cq_host_output_length.div_ceil(#block))
                        .map_err(|_| "Sequence output exceeds CUDA grid")?;
                    let prepared = #prepare?;
                    #call?;
                }
            });
            output = Some(if canonical {
                quote! {
                    let keys = unsafe { <#provider>::build_set(
                        &__cq_pool, &__cq_host_output_keys, __cq_host_output_length,
                        __cq_key_bits, #set_params)? };
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
    let program_helpers = &program_preparations;
    let output = launches.pop().expect("output launch");
    let setup = &compiler.rows.setup;
    let build = &compiler.rows.build;
    let arena_setup = arena
        .zip(arena_packing.as_ref())
        .map(|(expression, packing)| packing.setup(expression));
    let empty = arena
        .zip(arena_packing.as_ref())
        .map(|(expression, packing)| packing.empty(expression))
        .unwrap_or_else(|| {
            quote!(::cq_gpu_runtime::DeviceColumns::empty(
                ::gpu_datalog_runtime::thread_pool()?
            ))
        });
    let query_setup = crate::capture::print_query(&realization.captures.query, &empty);
    let mut used = BTreeSet::new();
    identifiers(
        quote!(#setup #arena_setup #build #(#launches)* #output),
        &mut used,
    );
    // A support object may be referenced only by a later support object
    // (for example the auxiliary geometry by the range builder): include
    // dependencies from the last object back to the first.
    let mut included = vec![false; compiler.rows.support.len()];
    for (index, (name, before, after)) in compiler.rows.support.iter().enumerate().rev() {
        if used.contains(&name.to_string()) {
            included[index] = true;
            identifiers(quote!(#before #after), &mut used);
        }
    }
    let support = compiler
        .rows
        .support
        .iter()
        .zip(&included)
        .filter_map(|(support, included)| included.then_some(support));
    let (support_before, support_after): (Vec<_>, Vec<_>) =
        support.map(|(_, before, after)| (before, after)).unzip();
    // Formal rule GPU-Arena-Timing: optional measurements alone add a terminal fence;
    // checked dispatch metadata and retained stream-ordered owners suffice otherwise.
    let timing_enabled = if arena.is_some() {
        quote!(::cq_gpu_runtime::arena_timing_enabled())
    } else {
        quote!(true)
    };
    let preparation_count = program_preparations.len();
    let cache_names = (0..preparation_count).map(|slot| format_ident!("__cq_launch_cache_{slot}"));
    syn::parse2(quote!({
        #[::cuda_device::cuda_module]
        mod #module_name {
            use super::*;
            use ::cuda_device::{kernel,launch_bounds,launch_contract};
            #(#kernels)*
        }
        #(#program_helpers)*
        ::cq_gpu_sequence_runtime::__codegen::#partition::run(#[inline] || -> ::cq_gpu_sequence_runtime::Result<_> {
            #query_setup
            #(#support_before)*
            #setup
            #arena_setup
            #(#support_after)*
            ::std::thread_local! {
                static __CQ_SEQUENCE_MODULE: ::std::cell::RefCell<Option<(
                    ::std::sync::Arc<::cuda_core::CudaContext>,
                    #module_name::LoadedModule,
                    [::cq_gpu_runtime::ProgramLaunchCache; #preparation_count],
                )>> = const { ::std::cell::RefCell::new(None) };
            }
            ::cq_gpu_sequence_runtime::register_query_cleanup(|| {
                __CQ_SEQUENCE_MODULE.with(|cache| cache.borrow_mut().take());
            });
            __CQ_SEQUENCE_MODULE.with(|cache| -> ::cq_gpu_runtime::Result<_> {
                let mut cache=cache.borrow_mut();
                if cache.as_ref().is_none_or(|(context,_,_)|
                    !::std::sync::Arc::ptr_eq(context,__cq_pool.context()))
                {
                    let __cq_module=unsafe {#module_name::load(__cq_pool.context())?};
                    let prepared=::std::array::from_fn(|_| ::cq_gpu_runtime::ProgramLaunchCache::default());
                    *cache=Some((__cq_pool.context().clone(),__cq_module,prepared));
                }
                let (_, __cq_module, [#(#cache_names,)*])=cache.as_mut().expect("initialized program launch");
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
            })
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
    #[test]
    fn expansion_counters_are_opt_in_block_collectives_before_closing() {
        let program: cq_lang_sequence::Program = syn::parse_quote! {
            physical {
                relation Driver(b:i32) = (&data.driver);
                relation R(a:i32,b:i32) = (&data.r);
                d = pipe rename Driver@0 {b -> b};
                r = let rename R@1 {a -> a,b -> b};
                joined = pipe probe first [b] d,r;
                result = let canon project joined keep {a,b};
                return result as answer(a,b);
            }
            structures { providers { radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                d host scan = @radix_relation input { keys 1; columns [i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,),(),u64,u64,()> [];
                r host scan = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [];
                r = @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word u64; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32),(),u64,u64,()> [b];
                result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                result canon = rows;
            }
            sequence {
                input = store rows d;
                index = store rows r;
                result_rows = store canon map result flatten lookup r [b] flatten tile 256 input;
                output count;
            }
            graded gpu { input at host; index at host; result_rows at host.device; }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let plain = super::translate_observed(&program, &analysis, None, false).unwrap();
        let observed = super::translate_observed(&program, &analysis, None, true).unwrap();
        let plain = quote::quote!(#plain).to_string();
        let observed = quote::quote!(#observed).to_string();
        assert!(!plain.contains("__cq_expansions"));
        assert!(plain.contains("RadixLayoutView"));
        assert!(plain.contains("view_on"));
        assert!(plain.contains("locate_key"));
        assert!(plain.contains("encode_prefix"));
        assert!(plain.contains("decode_key"));
        assert!(!plain.contains("prefix_range"));
        assert!(!plain.contains("interval_items"));
        assert!(observed.contains("if __tid == 0 { :: cuda_device :: atomic :: DeviceAtomicU64"));
        assert!(
            observed.find("DeviceAtomicU64").unwrap()
                < observed.find("let mut __child_base").unwrap()
        );
        assert!(
            observed.find("__cq_required > __cq_capacity").unwrap()
                < observed.find("expansions :: publish").unwrap()
        );
    }

    use super::*;
    use cq_lang_sequence::gpu::publication::{Granularity, PackedKeys};
    use quote::ToTokens;

    #[test]
    fn flat_publication_reads_only_its_recorded_columns() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i64,b:i64) = (&data);
                r = let rename R@0 {a -> a,b -> b};
                driver = pipe rename R@1 {a -> a,b -> c};
                joined = let canon probe first [a] driver,r;
                return joined as answer(a,b,c);
            }
            structures {
                providers {
                    device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns };
                    radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred };
                    range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range };
                }
                r host scan = @device_columns input { keys 0; columns [i64 => i64,i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                driver host scan = @device_columns input { keys 0; columns [i64 => i64,i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                r = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];
                joined rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                joined canon = rows;
            }
            sequence {
                index = store rows r;
                input = store rows driver;
                output_rows = store canon map joined flatten lookup r [a] flatten tile 256 input;
                output count;
            }
            graded gpu { index at host; input at host; output_rows at device; }
        };
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let compiler = Compiler {
            program: &program,
            analysis: &analysis,
            rows: Rows::new(&program, &analysis).unwrap(),
            stored: Vec::new(),
            definitions: BTreeMap::new(),
        };
        let slot = compiler.slot("b").unwrap();
        let mut kernel = Kernel {
            compiler: &compiler,
            shared: crate::layout::SharedLayout::new(),
            virtual_range: None,
            packed_columns: None,
            arena_packing: None,
            publication_columns: BTreeSet::from([slot]),
            sink: None,
            combine: None,
            expansion_counters: false,
            expansion_stages: 0,
            strips: Default::default(),
            parents: Default::default(),
            share: None,
        };
        let write = kernel.output_write().unwrap();
        assert_eq!(
            DeviceRow::uses(write.clone()).unwrap(),
            BTreeSet::from([slot])
        );
        DeviceRow(BTreeMap::from([(slot, quote!(value))]))
            .rewrite(write)
            .unwrap();
        kernel.virtual_range = Some(compiler.rows.provider_name("r", &["a".into()]).unwrap());
        let write = kernel.output_write().unwrap();
        assert!(DeviceRow::uses(write.clone()).unwrap().is_empty());
        DeviceRow::default().rewrite(write).unwrap();
    }

    /// A raw-candidate terminal, or with `canon` a canonical one.
    fn arena_program(canon: bool) -> Program {
        let canon = canon.then(|| quote!(canon));
        syn::parse_quote! {
            physical {
                relation R(a:i64,b:i64) = (&data);
                r = let rename R@0 {a -> a,b -> b};
                driver = pipe rename R@1 {a -> a,b -> c};
                joined = let canon probe first [a] driver,r;
                return joined as answer(c,a,b);
            }
            structures {
                providers {
                    device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns };
                    radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred };
                    range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range };
                }
                r host scan = @device_columns input { keys 0; columns [i64 => i64,i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                driver host scan = @device_columns input { keys 0; columns [i64 => i64,i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                r = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];
                joined rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                joined canon = rows;
            }
            sequence {
                index = store rows r;
                input = store rows driver;
                output_rows = store #canon map joined flatten lookup r [a] flatten tile 256 input;
                output count;
            }
            graded gpu { index at host; input at host; output_rows at device; }
        }
    }

    #[test]
    fn program_launch_preparation_is_recorded_checked_and_printed() {
        let program = arena_program(false);
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        assert_eq!(analysis.gpu_bindings.realization.launches, [Launch::ProgramOccupancy]);
        let arena: syn::Expr = syn::parse_quote!(arena);
        let printed = translate_observed(&program, &analysis, Some(&arena), false)
            .unwrap().to_token_stream().to_string();
        assert!(printed.contains("ProgramLaunchCache"));
        assert!(printed.contains("__cq_prepare_program_"));
        assert!(printed.find("let prepared =").unwrap() < printed.find("let __cq_timing_enabled").unwrap());
        assert!(!printed.contains("let __cq_probe"));
        assert!(printed.contains("let __cq_counts"));

        let mut alternate = analysis.clone();
        alternate.gpu_bindings.realization.launches[0] = Launch::InvocationOccupancy;
        crate::realize::check(&program, &alternate, &alternate.gpu_bindings.realization).unwrap();
        let printed = translate_observed(&program, &alternate, Some(&arena), false)
            .unwrap().to_token_stream().to_string();
        assert!(printed.contains("let __cq_probe"));
        assert!(!printed.contains("__cq_prepare_program_"));

        let mut missing = analysis.clone();
        missing.gpu_bindings.realization.launches.clear();
        assert!(crate::realize::check(&program, &missing, &missing.gpu_bindings.realization).is_err());
        assert!(translate_observed(&program, &missing, Some(&arena), false).is_err());
        let mut wrong_partition = analysis.clone();
        wrong_partition.gpu_bindings.realization.launches[0] = Launch::ProgramOne;
        assert!(crate::realize::check(&program, &wrong_partition, &wrong_partition.gpu_bindings.realization).is_err());
    }

    #[test]
    fn arena_encoding_is_recorded_for_raw_terminals_only() {
        use cq_lang_sequence::gpu::publication::{self, ArenaColumn, ArenaRows};
        let program = arena_program(false);
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let records = &analysis.gpu_publications;
        let arena = records["output_rows"].arena.clone().unwrap();
        let i64: syn::Type = syn::parse_quote!(i64);
        assert_eq!(
            arena,
            ArenaRows {
                columns: ["c", "a", "b"]
                    .map(|column| ArenaColumn {
                        column: column.into(),
                        carrier: i64.clone(),
                    })
                    .into()
            }
        );
        let rejected = |mutate: &dyn Fn(&mut BTreeMap<String, publication::PublicationRecord>)| {
            let mut records = records.clone();
            mutate(&mut records);
            publication::check(&program, &analysis, &records).is_err()
        };
        assert!(rejected(&|records| records
            .get_mut("output_rows")
            .unwrap()
            .arena = None));
        assert!(rejected(&|records| {
            let fields = &mut records
                .get_mut("output_rows")
                .unwrap()
                .arena
                .as_mut()
                .unwrap()
                .columns;
            fields.swap(0, 1);
        }));
        assert!(rejected(&|records| {
            let fields = &mut records
                .get_mut("output_rows")
                .unwrap()
                .arena
                .as_mut()
                .unwrap()
                .columns;
            fields[0].carrier = syn::parse_quote!(u32);
        }));
        assert!(rejected(&|records| {
            records.get_mut("index").unwrap().arena = Some(arena.clone());
        }));
        let arena_expr: syn::Expr = syn::parse_quote!(arena);
        translate_observed(&program, &analysis, Some(&arena_expr), false).unwrap();
        // A canonical terminal publishes no raw candidates into an arena.
        let program = arena_program(true);
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        assert_eq!(analysis.gpu_publications["output_rows"].arena, None);
        assert!(translate_observed(&program, &analysis, Some(&arena_expr), false).is_err());
    }

    /// Identifiers of generated code; independent of token spacing.
    fn idents(tokens: TokenStream) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        identifiers(tokens, &mut names);
        names
    }

    /// Whether a call path is a runtime fold: the keyed `fold` or the keyless
    /// `GlobalFold::new` grid-partials path.
    fn is_fold_call(path: &syn::Path) -> bool {
        let mut segments = path.segments.iter().rev();
        match (segments.next(), segments.next()) {
            (Some(last), _) if last.ident == "fold" => true,
            (Some(last), Some(owner)) => last.ident == "new" && owner.ident == "GlobalFold",
            _ => false,
        }
    }

    /// Accumulator providers of every emitted runtime fold; `None` is the
    /// global (keyless) `GlobalFold` path.
    fn fold_providers(generated: &syn::Expr) -> Vec<Option<syn::Type>> {
        struct Folds(Vec<Option<syn::Type>>);
        impl<'a> syn::visit::Visit<'a> for Folds {
            fn visit_expr_call(&mut self, call: &'a syn::ExprCall) {
                if let syn::Expr::Path(path) = &*call.func
                    && is_fold_call(&path.path)
                {
                    self.0
                        .push(path.qself.as_ref().map(|qself| (*qself.ty).clone()));
                }
                syn::visit::visit_expr_call(self, call);
            }
        }
        let mut folds = Folds(Vec::new());
        syn::visit::Visit::visit_expr(&mut folds, generated);
        folds.0
    }

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
            (
                quote!(select r if (#key0 > 0)),
                quote!(compact filtered flatten tile 256 input),
            ),
            (
                quote!(guard r if (*#key0 > 0 && *#key1 > 0)),
                quote!(map filtered flatten tile 256 input),
            ),
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
                    result = let canon project second keep {out0,out1};
                    return result as answer(out0,out1);
                }
                structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                    r host scan = @device_columns input { keys 0; columns [i32 => i32, i32 => i32] } as ::cq_gpu_runtime::DeviceColumns<'static,(i32,i32)> [];
                    result rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                    result canon = rows;
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
            })
            .unwrap();
            let program = crate::runs::record_representation(&program).unwrap();
            let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
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
            let mut bindings = Bindings {
                keys: &keys,
                counts: [0; 2],
            };
            bindings.visit_expr(&generated);
            assert!(bindings.counts.iter().all(|count| *count > 0));
        }
    }

    fn abi_rows() -> Rows {
        let program = arena_program(false);
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        Rows::new(&program, &analysis).unwrap()
    }

    #[test]
    fn parameter_budget_is_checked_by_rustc_layout() {
        let names: Vec<_> = (0..513).map(|index| format_ident!("p{index}")).collect();
        let abi = CheckedKernel::lower(&format_ident!("oversized"), quote!(),
            quote!(#(#names: *const u64),*), quote!(#(let _ = #names;)*),
            &abi_rows(), Launch::InvocationOccupancy, 0).unwrap();
        let emitted = abi.definition().to_string();
        assert!(emitted.contains("size_of"));
        assert!(emitted.contains("align_of"));
        assert!(emitted.contains("BE-Resource"));
    }

    #[test]
    fn recorded_pruning_preserves_signature_and_launch_order() {
        let abi = CheckedKernel::lower(&format_ident!("kernel"), quote!(),
            quote!(unused: u32, a: u32, b: *const i64), quote!(let _ = (b, a);),
            &abi_rows(), Launch::InvocationOccupancy, 0).unwrap();
        let function: syn::ItemFn = syn::parse2(abi.definition()).unwrap();
        assert_eq!(function.sig.inputs.to_token_stream().to_string(),
            quote!(a: u32, b: *const i64).to_string());
        let call = abi.call(quote!(stream), quote!(prepared), quote!(grid),
            quote!(unused(), first(), second())).unwrap();
        assert_eq!(call.to_string(),
            quote!(__cq_module.kernel(stream, prepared, first(), second())).to_string());
        assert!(abi.call(quote!(stream), quote!(prepared), quote!(grid), quote!(first())).is_err());
    }

    fn callable_abi(launch: Launch) -> (Rows, CheckedKernel) {
        let mut rows = abi_rows();
        rows.type_witnesses = vec![
            (format_ident!("Unused"), quote!(Fn() -> bool), format_ident!("unused")),
            (format_ident!("Evaluate"), quote!(Fn(&i64) -> bool), format_ident!("evaluate")),
        ];
        rows.parameter_witnesses = [
            ("unused".into(), format_ident!("Unused")),
            ("evaluate".into(), format_ident!("Evaluate")),
        ].into();
        let abi = CheckedKernel::lower(&format_ident!("kernel"), quote!(),
            quote!(unused: Unused, evaluate: Evaluate, x: i64),
            quote!(let _ = evaluate(&x);), &rows, launch, 64).unwrap();
        (rows, abi)
    }

    #[test]
    fn callable_preparation_uses_recorded_witnesses() {
        let (_, abi) = callable_abi(Launch::InvocationOccupancy);
        assert_eq!(abi.generics().to_string(),
            quote!(<Evaluate: Copy + Fn(&i64) -> bool>).to_string());
        assert_eq!(abi.prepare(quote!(config)).to_string(),
            quote!(__cq_module.prepare_kernel_for(&evaluate, config)).to_string());
    }

    #[test]
    fn nested_radix_word_generics_share_the_recorded_preparation_abi() {
        let mut rows = abi_rows();
        rows.type_witnesses = vec![
            (format_ident!("Word0"), quote!(::gpu_datalog_runtime::Word), format_ident!("word0")),
            (format_ident!("Word1"), quote!(::gpu_datalog_runtime::Word), format_ident!("word1")),
        ];
        rows.parameter_witnesses = [
            ("view".into(), format_ident!("Word0")),
            ("unused".into(), format_ident!("Word1")),
        ].into();
        let abi = CheckedKernel::lower(&format_ident!("kernel"), quote!(),
            quote!(view: ::gpu_datalog_runtime::radix_run::RadixLayoutView<Word0, u64>,
                unused: ::gpu_datalog_runtime::radix_run::RadixLayoutView<Word1, u64>),
            quote!(let _ = view.keys.run_count;), &rows, Launch::ProgramOccupancy, 64).unwrap();
        assert_eq!(abi.generics().to_string(),
            quote!(<Word0: Copy + ::gpu_datalog_runtime::Word>).to_string());
        let helpers = abi.program_helpers(&format_ident!("module")).to_string();
        assert!(helpers.contains("prepare_kernel_for (word0"));
        assert!(!helpers.contains("Word1"));
        assert!(!helpers.contains("ptx_name"));
        assert_eq!(abi.cached(&format_ident!("cache")).to_string(),
            quote!(__cq_prepare_program_kernel(__cq_module, &mut *cache, &word0)).to_string());
        let call = abi.call(quote!(stream), quote!(&prepared), quote!(__cq_grid),
            quote!(view, unused)).unwrap();
        assert_eq!(call.to_string(),
            quote!(unsafe { __cq_launch_program_kernel(stream, &prepared, __cq_grid, view) }).to_string());
    }

    #[test]
    fn program_launch_resolves_only_on_miss_and_uses_stack_scalar_arguments() {
        let (_, abi) = callable_abi(Launch::ProgramOccupancy);
        let helpers = abi.program_helpers(&format_ident!("module")).to_string();
        assert!(helpers.find("get_or_prepare (||").unwrap()
            < helpers.find("prepare_kernel_for").unwrap());
        assert!(!helpers.contains("ptx_name"));
        assert!(helpers.contains("evaluate : Evaluate , x : i64"));
        assert!(helpers.contains("KernelArgs :: < 2usize >"));
        assert!(!helpers.contains("Unused"));
        let call = abi.call(quote!(stream), quote!(&prepared), quote!(__cq_grid),
            quote!(unused, evaluate, x)).unwrap();
        assert_eq!(call.to_string(),
            quote!(unsafe { __cq_launch_program_kernel(stream, &prepared, __cq_grid, evaluate, x) }).to_string());
    }

    #[test]
    fn kernel_abi_checker_rejects_missing_live_parameters_and_witnesses() {
        let (rows, abi) = callable_abi(Launch::ProgramOccupancy);
        let mut missing = abi.0.clone();
        missing.parameters[1].live = false;
        assert!(missing.check(&rows).is_err());
        let mut missing = abi.0.clone();
        missing.witnesses.clear();
        assert!(missing.check(&rows).is_err());
        let mut wrong = abi.0.clone();
        wrong.preparation = format_ident!("prepare_other");
        assert!(wrong.check(&rows).is_err());
        let mut extra = abi.0.clone();
        extra.witnesses.push((format_ident!("Extra"), quote!(Copy), format_ident!("extra")));
        assert!(extra.check(&rows).is_err());
        let mut wrong = abi.0.clone();
        wrong.witnesses[0].2 = format_ident!("unused");
        assert!(wrong.check(&rows).is_err());
        let mut missing = abi.0.clone();
        missing.parameters[1].witness = None;
        assert!(missing.check(&rows).is_err());
    }

    #[test]
    fn publication_storage_and_codec_order_are_recorded_before_printing() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i64,b:u32) = (&data);
                r = let canon rename R@0 {a -> a,b -> b};
                return r as answer(b,a);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @device_columns input { keys 0; columns [i64 => i64, u32 => u32] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,u32)> [];
                r rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                r canon = rows;
            }
            sequence {
                input = store rows r;
                packed_feed = canon flatten tile 256 input;
                packed = store packed_feed;
                raw_feed = flatten tile 256 packed;
                raw = store raw_feed;
                terminal_feed = flatten tile 256 raw;
                result = store terminal_feed;
                output count;
            }
            graded gpu {
                input at host;
                packed_feed at device.block;
                packed at host;
                raw_feed at device.block;
                raw at host;
                terminal_feed at device.block;
                result at host;
            }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let records = &analysis.gpu_publications;
        let radix_set: syn::Type = syn::parse_quote!(::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>);
        assert!(!records.contains_key("packed_feed"));
        assert_eq!(
            packed_columns(&records["packed"].storage).unwrap(),
            ["a", "b"]
        );
        assert_eq!(
            records["packed"].storage.canonical_provider(),
            Some(&radix_set)
        );
        assert!(matches!(
            records["raw"].storage,
            PublicationStorage::FlatRows(_)
        ));
        assert_eq!(
            packed_columns(&records["result"].storage).unwrap(),
            ["b", "a"]
        );
        assert_eq!(records["result"].storage.canonical_provider(), None);
        for field in records["result"].storage.codec().unwrap().fields() {
            assert_eq!(
                field.bounds,
                FieldBounds::Column(analysis.gpu_bindings.bounds[&field.column].clone())
            );
        }
        let names = idents(translate(&program, &analysis).unwrap().to_token_stream());
        // `packed` canonicalizes; the terminal publishes its candidate keys.
        assert!(names.contains("build_set") && names.contains("packed_columns"));
        assert!(!names.contains("unpack_columns"));

        let rejects = |records| {
            let mut corrupted = analysis.clone();
            corrupted.gpu_publications = records;
            assert!(
                cq_lang_sequence::gpu::publication::check(
                    &program,
                    &corrupted,
                    &corrupted.gpu_publications
                )
                .is_err()
            );
            assert!(translate(&program, &corrupted).is_err());
        };
        let mutate = |name: &str, change: &dyn Fn(&mut PackedKeys)| {
            let mut records = records.clone();
            let PublicationStorage::PackedKeys(packed) =
                &mut records.get_mut(name).unwrap().storage
            else {
                unreachable!()
            };
            change(packed);
            records
        };
        fn fields(packed: &mut PackedKeys) -> &mut Vec<KeyField> {
            let KeyCodec::FrameOfReference { fields } = &mut packed.codec;
            fields
        }
        rejects(mutate("result", &|packed| fields(packed).swap(0, 1)));
        rejects(mutate("result", &|packed| {
            fields(packed)[0].bounds = FieldBounds::Inclusive { min: 0, max: 1 };
        }));
        rejects(mutate("result", &|packed| {
            fields(packed)[0].bounds = FieldBounds::Column(ColumnBounds::Scalar {
                ty: syn::parse_quote!(u8),
            });
        }));
        // Granularity: a dropped Canon, a Canon without one on every root
        // path, and a provider other than the canonical rows record's provider.
        rejects(mutate("packed", &|packed| {
            packed.granularity = Granularity::Rows
        }));
        rejects(mutate("result", &|packed| {
            packed.granularity = Granularity::Canonical {
                provider: syn::parse_quote!(::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>),
            };
        }));
        rejects(mutate("packed", &|packed| {
            packed.granularity = Granularity::Canonical {
                provider: syn::parse_quote!(::cq_gpu_sequence_runtime::RangeProvider),
            };
        }));
        for mutation in 0..4 {
            let mut corrupt = records.clone();
            let set = corrupt
                .get_mut("packed")
                .unwrap()
                .set_build
                .as_mut()
                .unwrap();
            match mutation {
                0 => set.block_threads = 128,
                1 => set.ratio = 1,
                2 => set.radix_bits = 1,
                _ => set.merge_tile_rows = 0,
            }
            rejects(corrupt);
        }
        // Valid non-default policy parameters are obligations, not a replay
        // of the selector's defaults, and the printer consumes their values.
        let mut alternate = analysis.clone();
        let record = alternate.gpu_publications.get_mut("packed").unwrap();
        let set = record.set_build.as_mut().unwrap();
        set.ratio = 8;
        set.merge_tile_rows = 512;
        let printed = set_params(record).unwrap().to_string();
        assert!(printed.contains("ratio : 8"));
        assert!(printed.contains("merge_tile_rows : 512"));
        assert!(printed.contains("block_threads : 256"));
        assert!(!printed.contains("__cq_auxiliary"));
        cq_lang_sequence::gpu::publication::check(
            &program,
            &alternate,
            &alternate.gpu_publications,
        )
        .unwrap();
        let mut no_build = records.clone();
        no_build.get_mut("packed").unwrap().set_build = None;
        rejects(no_build);
        let mut dropped = records.clone();
        dropped.get_mut("packed").unwrap().storage = records["raw"].storage.clone();
        rejects(dropped);
        let mut wrong_storage = records.clone();
        wrong_storage.get_mut("result").unwrap().storage = records["raw"].storage.clone();
        rejects(wrong_storage);
        let mut wrong_carrier = records.clone();
        let PublicationStorage::FlatRows(flat) = &mut wrong_carrier.get_mut("raw").unwrap().storage
        else {
            unreachable!()
        };
        flat.columns.reverse();
        rejects(wrong_carrier);
        let mut wrong_stride = records.clone();
        let PublicationStorage::FlatRows(flat) = &mut wrong_stride.get_mut("raw").unwrap().storage
        else {
            unreachable!()
        };
        flat.width += 1;
        rejects(wrong_stride);
        let mut packed_raw = records.clone();
        packed_raw.get_mut("raw").unwrap().storage = records["result"].storage.clone();
        rejects(packed_raw);
        let mut missing = records.clone();
        missing.remove("result");
        rejects(missing);
        let mut deferred = analysis.clone();
        deferred
            .dispatches
            .last_mut()
            .unwrap()
            .gpu
            .as_mut()
            .unwrap()
            .publication = cq_lang_sequence::gpu::Publication::DeferredRange {
            provider: "r".into(),
            keys: vec![],
        };
        assert!(
            cq_lang_sequence::gpu::publication::check(
                &program,
                &deferred,
                &deferred.gpu_publications
            )
            .is_err()
        );
    }

    #[test]
    fn packing_renders_exactly_the_recorded_bounds_source() {
        let resident = FieldBounds::Column(ColumnBounds::Resident {
            sources: vec![("r".into(), 1), ("s".into(), 0)],
        });
        let names = idents(packing_bounds(&resident));
        assert!(names.contains("__cq_sequence_host_r") && names.contains("__cq_sequence_host_s"));
        assert!(
            !names
                .iter()
                .any(|name| name.starts_with("__cq_sequence_uploaded_"))
        );
        let scalar = FieldBounds::Column(ColumnBounds::Scalar {
            ty: syn::parse_quote!(u16),
        });
        let names = idents(packing_bounds(&scalar));
        assert!(names.contains("u16") && names.contains("Scalar"));
        assert!(
            !names
                .iter()
                .any(|name| name.starts_with("__cq_sequence_host_"))
        );
        // Field order fixes the base/shift parameter each recorded source feeds.
        let codec = KeyCodec::FrameOfReference {
            fields: vec![
                KeyField {
                    column: "a".into(),
                    bounds: scalar,
                },
                KeyField {
                    column: "b".into(),
                    bounds: resident,
                },
            ],
        };
        let packing = packing_fields(&codec);
        assert!(idents(packing[0].clone()).contains("__cq_pack_base_0"));
        assert!(idents(packing[1].clone()).contains("__cq_sequence_host_s"));
    }

    #[test]
    fn scalar_fold_codec_bounds_are_recorded_not_guessed_from_column_types() {
        for (fold, min, max) in [
            (quote!(count), 0, i64::MAX),
            (quote!(or), 0, 1),
            (quote!(sum a), i64::MIN, i64::MAX),
        ] {
            let program: Program = syn::parse2(quote! {
                physical {
                    relation R(a:i64) = (&data);
                    r = let canon rename R@0 {a -> a};
                    return r as answer(a);
                }
                structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                    r host scan = @device_columns input { keys 0; columns [i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                    r rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                    r canon = rows;
                }
                sequence {
                    input = store rows r;
                    result = store capacity 1 fold #fold input;
                    output bounded;
                }
                graded gpu { input at host; result at device.block; }
            })
            .unwrap();
            let program = crate::runs::record_representation(&program).unwrap();
            let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
            let codec = analysis.gpu_publications["result"].storage.codec().unwrap();
            assert_eq!(
                codec.fields()[0].bounds,
                FieldBounds::Inclusive { min, max }
            );
            let names = idents(translate(&program, &analysis).unwrap().to_token_stream());
            assert!(!names.contains("build_set"));
            let accepts = |bounds: FieldBounds| {
                let mut corrupt = analysis.clone();
                let PublicationStorage::PackedKeys(PackedKeys {
                    codec: KeyCodec::FrameOfReference { fields },
                    ..
                }) = &mut corrupt.gpu_publications.get_mut("result").unwrap().storage
                else {
                    unreachable!()
                };
                fields[0].bounds = bounds;
                cq_lang_sequence::gpu::publication::check(
                    &program,
                    &corrupt,
                    &corrupt.gpu_publications,
                )
                .is_ok()
            };
            assert!(!accepts(FieldBounds::Column(ColumnBounds::Scalar {
                ty: syn::parse_quote!(i64)
            })));
            // Fold bounds must contain every fold value; any containing range is sound.
            assert!(!accepts(FieldBounds::Inclusive { min: min + 1, max }));
            assert!(!accepts(FieldBounds::Inclusive { min, max: max - 1 }));
            assert!(accepts(FieldBounds::Inclusive {
                min: i64::MIN,
                max: i64::MAX
            }));
        }
    }

    fn uploaded_host_program(canonical: bool) -> Program {
        let canon = canonical.then(|| quote!(canon));
        let program = syn::parse_quote! {
            physical {
                relation Resident(a:i64) = (&data.resident);
                relation Host(a:i64) = (&data.host);
                resident = let rename Resident@0 {a -> a};
                host_input = pipe rename Host@1 {a -> a};
                host = let canon semi eager [a] host_input,resident;
                return host as answer(a);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sorted_run = ::cq_storage::SortedRun<_, _> { reads [exact, range, membership], supports [retain, expand, keys], build typed, join_filter, run_filter, shared, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact run_finalize stat_rows host required build single, keys: keys distinct_keys exact run_finalize stat_keys host required build single, max_group: max_group max_bucket_occurrences exact run_finalize stat_max_group host required build single, runs: runs runs exact run_finalize stat_runs host required build single ] }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                resident host scan = @device_columns input { keys 0; columns [i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                host_input host scan = @host_scan as ::core::iter::Map<_,_> [];
                resident = @sorted_run as ::cq_storage::SortedRun<_,_> [a];
                host rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                host canon = rows;
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
    fn structures_staging_cannot_be_silently_dropped_by_target_switch() {
        let mut program = uploaded_host_program(false);
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        assert!(translate(&program, &analysis).is_ok());
        program
            .structures
            .structures
            .entries
            .push(syn::parse_quote!(host_input stage = batch 2;));
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        assert!(translate(&program, &analysis).is_err());
    }

    #[test]
    fn terminal_group_store_alias_emits_one_collective_without_host_reupload() {
        for keyed in [false, true] {
            let keys = keyed.then(|| quote!(key));
            let output_key = keyed.then(|| quote!(key,));
            let accumulator = keyed.then(|| {
                quote!(
                    grouped group = @group_map as ::cq_gpu_sequence_runtime::group::SegmentedGroup;
                )
            });
            let group_map = keyed.then(|| {
                quote!(group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed };)
            });
            let program: Program = syn::parse2(quote! {
                physical {
                    relation R(id:i64,key:i32,value:i64) = (&data.r);
                    r = pipe rename R@0 {id -> id,key -> key,value -> value};
                    valuation = let canon project r keep {id,key,value};
                    grouped = pipe group valuation by [#keys] {
                        total:i64=sum(value); rows:usize=count();
                    };
                    return grouped as answer(#output_key total,rows);
                }
                structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; #group_map }
                    r host scan = @device_columns input { keys 0; columns [i64 => i64, i32 => i32, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i32,i64)> [];
                    valuation rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                    valuation canon = rows;
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
            let expression = translate(&program, &analysis).unwrap();
            let generated = expression.to_token_stream().to_string();
            let expected = keyed.then(|| analysis.structures.groups["grouped"].ty.clone());
            assert_eq!(fold_providers(&expression), vec![expected], "{generated}");
            let names = idents(expression.to_token_stream());
            assert!(!names.contains("upload_numeric_columns"));
            assert!(!names.contains("from_rows"));
        }
    }

    #[test]
    fn keyed_group_reads_resident_columns_without_valuation_publication() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(id:i64,key:i32,value:i64) = (&data.r);
                r = pipe rename R@0 {id -> id,key -> key,value -> value};
                valuation = let canon project r keep {id,key,value};
                grouped = pipe group valuation by [key] { total:i64=sum(value); rows:usize=count(); };
                return grouped as answer(key,total,rows);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed }; }
                r host scan = @device_columns input { keys 0; columns [i64 => i64, i32 => i32, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i32,i64)> [];
                valuation rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                valuation canon = rows;
                grouped group = @group_map as ::cq_gpu_sequence_runtime::group::SegmentedGroup;
            }
            sequence {
                input = rows r;
                valuations = map valuation input;
                groups = store group grouped valuations;
                output count;
            }
            graded gpu { input at host; valuations at host; groups at host; }
            evidence from authored("experiments/gpu/sequence/core/src/emit.rs fixture") { trusted_key R [id]; }
        };
        let program = crate::runs::record_representation(&program).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let expression = translate(&program, &analysis).unwrap();
        let names = idents(expression.to_token_stream());
        assert_eq!(
            fold_providers(&expression),
            vec![Some(crate::grouping::provider().ty)],
        );
        assert!(matches!(
            analysis.gpu_publications["groups"].storage,
            PublicationStorage::FlatRows(_)
        ));
        assert!(!analysis.gpu_publications.contains_key("valuations"));
        assert!(!names.contains("build_set"));
    }

    #[test]
    fn grouped_host_rust_input_is_materialized_before_device_reduction() {
        for (provider, declaration) in [
            (
                quote!(@host_scan as ::core::iter::Map<_,_>),
                quote!(host_scan = ::core::iter::Map<_, _> { reads [], build typed };),
            ),
            (
                quote!(@radix_relation input { keys 2; columns [i64 => i64, i32 => ::gpu_datalog_runtime::Symbol<i32, TestCodec> codec TestCodec] } radix { word u128; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<
                    (i64, ::gpu_datalog_runtime::Symbol<i32, TestCodec>), (), u128, u64, ()>),
                quote!(radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> { reads [exact, range, membership, resident, distinct], build typed, abi radix_relation };),
            ),
        ] {
            let program: Program = syn::parse2(quote! {
                physical {
                    relation R(id:i64,raw:i32) = (&data.r);
                    r = pipe rename R@0 {id -> id,raw -> raw};
                    bound = pipe bind r with value:i64 = (*raw as i64);
                    valuation = let canon project bound keep {id,value};
                    grouped = pipe group valuation by [] { total:i64=sum(value); rows:usize=count(); };
                    return grouped as answer(total,rows);
                }
                structures { providers { #declaration radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                    r host scan = #provider [];
                    valuation rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                    valuation canon = rows;
                }
                sequence {
                    input = rows r;
                    valuations = map valuation input;
                    groups = store group grouped valuations;
                    output count;
                }
                graded gpu { input at host; valuations at host; groups at host; }
                evidence from authored("experiments/gpu/sequence/core/src/emit.rs fixture") { trusted_key R [id]; }
            }).unwrap();
            let program = crate::runs::record_representation(&program).unwrap();
            let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
            assert!(analysis.gpu_bindings.uploads.contains_key("valuation"));
            let expression = translate(&program, &analysis).unwrap();
            let generated = expression.to_token_stream().to_string();
            // Calls in source order: host upload adapters, then folds.
            struct Calls(Vec<&'static str>, bool);
            impl<'a> syn::visit::Visit<'a> for Calls {
                fn visit_expr_cast(&mut self, cast: &'a syn::ExprCast) {
                    let expected: syn::Expr = syn::parse_quote!(*raw as i64);
                    self.1 |= syn::Expr::Cast(cast.clone()) == expected;
                    syn::visit::visit_expr_cast(self, cast);
                }
                fn visit_expr_call(&mut self, call: &'a syn::ExprCall) {
                    if let syn::Expr::Path(path) = &*call.func
                        && let Some(last) = path.path.segments.last()
                    {
                        if last.ident == "from_rows" || last.ident == "upload_numeric_columns" {
                            self.0.push("upload");
                        } else if is_fold_call(&path.path) {
                            self.0.push("fold");
                        }
                    }
                    syn::visit::visit_expr_call(self, call);
                }
            }
            let mut calls = Calls(Vec::new(), false);
            syn::visit::Visit::visit_expr(&mut calls, &expression);
            let upload = calls
                .0
                .iter()
                .position(|call| *call == "upload")
                .expect(&generated);
            let reduction = calls
                .0
                .iter()
                .position(|call| *call == "fold")
                .expect(&generated);
            assert!(upload < reduction, "{generated}");
            assert!(calls.1, "the authored opaque Rust cast must be preserved");
            assert_eq!(fold_providers(&expression), vec![None], "{generated}");
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
            let record = &analysis.gpu_publications["result"];
            assert_eq!(
                record.storage.codec().unwrap().fields(),
                &[KeyField {
                    column: "a".into(),
                    bounds: FieldBounds::Column(analysis.gpu_bindings.bounds["a"].clone()),
                }]
            );
            assert_eq!(record.storage.canonical_provider().is_some(), canonical);
            let names = idents(translate(&program, &analysis).unwrap().to_token_stream());
            assert!(names.contains("__cq_sequence_uploaded_length_host"));
            assert!(names.contains("__cq_host_output_length"));
            assert!(
                !names
                    .iter()
                    .any(|name| name.starts_with("__cq_sequence_dispatch_")
                        || name.contains("__row"))
            );
            assert_eq!(names.contains("build_set"), canonical);
            assert_eq!(names.contains("unpack_columns"), canonical);
            // Printing follows the recorded granularity. The terminal aliases a
            // host-materialized publication, so a Rows record is sound there,
            // while a Canonical record without a Canon on its alias path is not.
            let mut flipped = analysis.clone();
            let PublicationStorage::PackedKeys(packed) =
                &mut flipped.gpu_publications.get_mut("result").unwrap().storage
            else {
                unreachable!()
            };
            packed.granularity = if canonical {
                Granularity::Rows
            } else {
                Granularity::Canonical {
                    provider: syn::parse_quote!(::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>),
                }
            };
            if canonical {
                flipped
                    .gpu_publications
                    .get_mut("result")
                    .unwrap()
                    .set_build = None;
                let names = idents(translate(&program, &flipped).unwrap().to_token_stream());
                assert!(!names.contains("build_set") && names.contains("packed_columns"));
            } else {
                assert!(translate(&program, &flipped).is_err());
            }
        }
    }

    #[test]
    fn host_read_of_a_device_provider_uses_a_checked_host_adapter() {
        let program: Program = syn::parse_quote! {
            physical {
                relation R(a:i64) = (&data.r);
                relation P(a:i64) = (&data.p);
                r = pipe rename R@0 {a -> a};
                p = let rename P@1 {a -> a};
                present = let canon semi eager [a] r,p;
                return present as answer(a);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @device_columns input { keys 0; columns [i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                p host scan = @host_scan as ::core::iter::Map<_,_> [];
                p = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider [a];
                present rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                present canon = rows;
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
        let adapters = &analysis.gpu_bindings.host_materializations;
        assert_eq!(
            adapters.get(&2),
            Some(&cq_lang_structures::provider::base::declaration("sorted_run").instance()),
        );
        assert!(translate(&program, &analysis).is_ok());
        let mut invalid = analysis.gpu_bindings.clone();
        invalid.host_materializations.clear();
        assert!(crate::bindings::check(&program, &analysis, &invalid).is_err());
    }

    #[test]
    fn deferred_host_map_reads_the_published_rows_not_the_original_producer() {
        for canonical in [false, true] {
            let canon = canonical.then(|| quote!(canon));
            let (filtered_rows, vec) = if canonical {
                (
                    quote!(
                        filtered rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                        filtered canon = rows;
                    ),
                    None,
                )
            } else {
                (
                    quote!(filtered rows = @vec as ::std::vec::Vec<_>;),
                    Some(
                        quote!(vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows };),
                    ),
                )
            };
            let program: Program = syn::parse2(quote! {
                physical {
                    relation R(a:i64) = (&data.r);
                    r = pipe rename R@0 {a -> a};
                    filtered = let #canon select r if (a > 0);
                    guarded = let canon guard filtered if (a % 2 == 0);
                    return guarded as answer(a);
                }
                structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; #vec }
                    r host scan = @device_columns input { keys 0; columns [i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                    #filtered_rows
                    guarded rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                    guarded canon = rows;
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
            // A canonical rows record is realized by its canonical publication:
            // the host reads the downloaded distinct rows by value, without a
            // host build of the device provider or a second row copy.
            let copied = generated.contains(
                "collect :: < :: std :: vec :: Vec < _ > > (:: core :: iter :: Iterator :: map (__cq_downloaded_0",
            );
            assert_eq!(copied, !canonical, "{generated}");
            assert!(!generated.contains("collect :: < :: gpu_datalog_runtime :: RadixLayout"));
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
                r = let canon rename R@0 {a -> a};
                return r as answer(a);
            }
            structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                r host scan = @device_columns as ::cq_gpu_runtime::DeviceColumns<'static,(i64,)> [];
                r rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                r canon = rows;
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
                    r = let canon rename R@0 {a -> a};
                    return r as answer(a);
                }
                structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred }; }
                    r host scan = @device_columns input { keys 0; columns [#ty => #ty] } as ::cq_gpu_runtime::DeviceColumns<'static,(#ty,),::cq_gpu_runtime::For> [];
                    r rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                    r canon = rows;
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
                        operations.push(Operation::Tile);
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
                        combine: None,
                        expansion_counters: false,
                        expansion_stages: 0,
                        strips: Default::default(),
                        parents: Default::default(),
                        share: None,
                    };
                    let strip = Strip {
                        rows: None,
                        items: 4,
                        nested: Vec::new(),
                        fold: Some(if tiled {
                            FoldScope::Tiles(1)
                        } else {
                            FoldScope::Whole
                        }),
                    };
                    let (declare, body, finish, whole) = kernel
                        .partition_rows(
                            &operations,
                            &strip,
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
                        assert!(generated.contains("let mut __code_0 : i64"));
                    }
                }
            }
        }
    }
}
