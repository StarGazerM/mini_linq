//! Checked Sequence-to-CUDA translation over resident column providers.

use cq_lang_sequence::{LevelNode, Machine};

mod emit;
mod reduce;
mod rows;

pub use emit::translate;

/// Host publication over separated, dynamically scheduled device blocks.
/// Each block is collaborative; lanes are sequential leaves (no warp level).
pub fn machine() -> Machine {
    Machine {
        name: syn::parse_quote!(gpu),
        items: 4,
        root: LevelNode {
            name: syn::parse_quote!(host),
            collaborative: false,
            dynamic: false,
            width: None,
            children: vec![LevelNode {
                name: syn::parse_quote!(device),
                collaborative: false,
                dynamic: true,
                width: None,
                children: vec![LevelNode {
                    name: syn::parse_quote!(block),
                    collaborative: true,
                    dynamic: false,
                    width: Some(256),
                    children: vec![LevelNode {
                        name: syn::parse_quote!(lane),
                        collaborative: false,
                        dynamic: false,
                        width: None,
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
        vec!["host".into()],
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
        providers: vec![ProviderToken {
            ty: syn::parse_quote!(::cq_gpu_sequence_runtime::RangeProvider),
            capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
            levels: levels.clone(),
        }],
        codecs: [
            syn::parse_quote!(::cq_storage::sequence::Identity),
            syn::parse_quote!(::cq_gpu_sequence_runtime::DenseDictionary<'_>),
        ]
        .into_iter()
        .map(|ty| CodecToken {
            ty,
            ordered: true,
            levels: levels.clone(),
        })
        .collect(),
        canons: vec![CanonToken {
            ty: syn::parse_quote!(::cq_gpu_sequence_runtime::canon::PackedCanon),
            levels: levels[..2].to_vec(),
        }],
    }
}
