//! Checked Sequence-to-CUDA translation over resident column providers.

use cq_lang_sequence::{LevelNode, Machine};

mod arena;
mod capture;
pub mod constants;
pub mod runs;
mod emit;
pub mod layout;
mod reduce;
mod rows;

pub use emit::translate;
pub use arena::translate_arena;


/// Native constructor identities in the GPU catalog. Generic arguments carry
/// the concrete schema; neither aliases nor unrelated suffixes imply an ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeAbi {
    Runs,
    Symbol,
    Columns,
    Range,
}

impl NativeAbi {
    pub fn identity(self) -> syn::Path {
        match self {
            Self::Runs => syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout),
            Self::Symbol => syn::parse_quote!(::gpu_datalog_runtime::Symbol),
            Self::Columns => syn::parse_quote!(::cq_gpu_runtime::DeviceColumns),
            Self::Range => syn::parse_quote!(::cq_gpu_sequence_runtime::RangeProvider),
        }
    }
}

/// Formal rule GPU-NativeAbi: native ABI and residency follow complete catalog constructor identity.
pub fn native_abi(ty: &syn::Type) -> Option<NativeAbi> {
    let syn::Type::Path(ty) = ty else { return None };
    if ty.qself.is_some() { return None; }
    [NativeAbi::Runs, NativeAbi::Symbol, NativeAbi::Columns, NativeAbi::Range]
        .into_iter().find(|abi| {
            let identity = abi.identity();
            ty.path.segments.len() == identity.segments.len()
                && ty.path.segments.iter().zip(&identity.segments)
                    .all(|(actual, expected)| actual.ident == expected.ident)
                && ty.path.segments.iter().rev().skip(1)
                    .all(|segment| matches!(segment.arguments, syn::PathArguments::None))
        })
}
/// Host publication over separated, dynamically scheduled device blocks.
/// Each block is collaborative; lanes are sequential leaves (no warp level).
pub fn machine() -> Machine {
    Machine { gpu: Some(cq_lang_sequence::gpu::Machine {
        block_threads: 256,
        contiguous_range: |ty| native_abi(ty) == Some(NativeAbi::Range),
    }), name: syn::parse_quote!(gpu),
    items: 4,
    key_partition_threshold: 65536,
    key_partition_cache_threshold: 6 * 1024 * 1024,
    layout: Some(layout::shared_bytes),
    root: LevelNode {
        name: syn::parse_quote!(host),
        host_rust: true,
        collaborative: false,
        dynamic: false,
        width: None,
        shared_bytes: None,
        children: vec![LevelNode {
            name: syn::parse_quote!(device),
            host_rust: false,
            collaborative: false,
            dynamic: true,
            width: None,
            shared_bytes: None,
            children: vec![LevelNode {
                name: syn::parse_quote!(block),
                host_rust: false,
                collaborative: true,
                dynamic: false,
                width: Some(256),
                shared_bytes: Some(101_376),
                children: vec![LevelNode {
                    name: syn::parse_quote!(lane),
                    host_rust: false,
                    collaborative: false,
                    dynamic: false,
                    width: None,
                    shared_bytes: None,
                    children: vec![],
                }],
            }],
        }],
    }, }
}

/// Resident range reads, order-preserving codes and flat packed publication.
///
/// The tokens name runtime implementations without importing the runtime into
/// this compile-time target. Canonicalization runs at a separated publication,
/// not inside a collaborative block or lane.
pub fn catalog() -> cq_lang_sequence::Catalog {
    use cq_lang_sequence::{CanonToken, Capability, Catalog, CodecToken, ProviderToken};

    let range = NativeAbi::Range.identity();
    let runs = NativeAbi::Runs.identity();
    let levels = vec![
        vec!["host".into(), "device".into()],
        vec!["host".into(), "device".into(), "block".into()],
        vec![
            "host".into(),
            "device".into(),
            "block".into(),
            "lane".into(),
        ],
    ];
    Catalog {
        providers: vec![
            ProviderToken {
                ty: syn::parse_quote!(#range),
                capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
                // Range providers consume the recorded scalar/code carrier,
                // not logical type spellings. Native Scalar bounds and explicit
                // codec declarations establish the publication representation.
                elements: None,
                levels: levels.clone(),
            },
            ProviderToken {
                ty: syn::parse_quote!(#runs<_, _>),
                capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
                // The declared layout checks primitives and Symbol<T, Codec>
                // columns; logical symbol types do not appear in device rows.
                elements: None,
                levels: levels.clone(),
            },
            ProviderToken {
                ty: syn::parse_quote!(::cq_storage::SortedRun<_, _>),
                capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
                elements: None,
                levels: vec![vec!["host".into()]],
            },
        ],
        codecs: [
            syn::parse_quote!(::cq_storage::sequence::Identity),
            syn::parse_quote!(::cq_gpu_sequence_runtime::DenseDictionary<'_>),
        ]
        .into_iter()
        .map(|ty| CodecToken {
            ty,
            ordered: true,
            levels: std::iter::once(vec!["host".into()])
                .chain(levels.iter().cloned())
                .collect(),
        })
        .collect(),
        canons: vec![
            CanonToken {
                ty: syn::parse_quote!(::cq_gpu_sequence_runtime::canon::PackedCanon),
                levels: levels.clone(),
                max_packed_bits: Some(128),
            },
            CanonToken {
                ty: syn::parse_quote!(::cq_gpu_sequence_runtime::canon::WideCanon),
                levels,
                max_packed_bits: None,
            },
        ],
    }
}
