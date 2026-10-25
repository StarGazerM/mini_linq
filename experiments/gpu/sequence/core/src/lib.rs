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

/// Host publication over separated, dynamically scheduled device blocks.
/// Each block is collaborative; lanes are sequential leaves (no warp level).
pub fn machine() -> Machine {
    Machine {
        name: syn::parse_quote!(gpu),
        items: 4,
        key_partition_threshold: 65536,
        key_partition_cache_threshold: 6 * 1024 * 1024,
        layout: Some(layout::shared_bytes),
        root: LevelNode {
            name: syn::parse_quote!(host),
            collaborative: false,
            dynamic: false,
            width: None,
            shared_bytes: None,
            children: vec![LevelNode {
                name: syn::parse_quote!(device),
                collaborative: false,
                dynamic: true,
                width: None,
                shared_bytes: None,
                children: vec![LevelNode {
                    name: syn::parse_quote!(block),
                    collaborative: true,
                    dynamic: false,
                    width: Some(256),
                    shared_bytes: Some(101_376),
                    children: vec![LevelNode {
                        name: syn::parse_quote!(lane),
                        collaborative: false,
                        dynamic: false,
                        width: None,
                        shared_bytes: None,
                        children: vec![],
                    }],
                }],
            }],
        },
    }
}

/// Resident range reads, order-preserving codes and flat packed publication.
///
/// The tokens name runtime implementations without importing the runtime into
/// this compile-time target. Canonicalization runs at a separated publication,
/// not inside a collaborative block or lane.
pub fn catalog() -> cq_lang_sequence::Catalog {
    use cq_lang_sequence::{CanonToken, Capability, Catalog, CodecToken, ProviderToken};

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
                ty: syn::parse_quote!(::cq_gpu_sequence_runtime::RangeProvider),
                capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
                elements: Some(vec![
                    syn::parse_quote!(i8),
                    syn::parse_quote!(i16),
                    syn::parse_quote!(i32),
                    syn::parse_quote!(i64),
                    syn::parse_quote!(u8),
                    syn::parse_quote!(u16),
                    syn::parse_quote!(u32),
                    syn::parse_quote!(bool),
                ]),
                levels: levels.clone(),
            },
            ProviderToken {
                ty: syn::parse_quote!(::gpu_datalog_runtime::DeviceLayout<_, _>),
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
