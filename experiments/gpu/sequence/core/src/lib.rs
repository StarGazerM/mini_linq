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

/// Validate the Structures boundary without selecting CPU batching.
/// GPU batching and storage scheduling are recorded later in Sequence.
pub fn check_structures(
    source: &cq_lang_structures::Program,
    analysis: &cq_lang_structures::Analysis,
) -> syn::Result<()> {
    check_physical(&source.physical)?;
    if !analysis.witnesses.is_empty()
        || !analysis.stages.is_empty()
        || !analysis.join_filters.is_empty()
    {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "T-Gpu: Structures witnesses, staging and join filters are not realized by this target; use supported Sequence scheduling or a native terminal",
        ));
    }
    Ok(())
}

fn check_physical(source: &cq_lang_structures::physical::Program) -> syn::Result<()> {
    use cq_lang_structures::physical::{Choice, Operator};
    for definition in &source.program.definitions {
        if matches!(&definition.operator,
            Operator::Probe(_, key) if matches!(key.choice, Some(Choice::Smallest(_)))) {
            return Err(syn::Error::new_spanned(&definition.name,
                "T-Gpu: probe smallest is not realized by this target; per-tuple minimum candidate selection requires grouped-run ranges"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod capability_tests {
    #[test]
    fn rejects_smallest_instead_of_emitting_a_sequential_probe() {
        use cq_lang_structures::physical::{self, Choice, Operator};
        let source: physical::Program = syn::parse_quote!(physical {
            relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
            l = pipe rename L@0 {a -> a,b -> b};
            r = let rename R@1 {b -> b,c -> c};
            s = let rename S@2 {a -> a,c -> c};
            candidate = pipe probe smallest [b] l,r;
            checked = pipe canon semi eager [a,c] candidate,s;
            return checked as answer(a,b,c);
        });
        physical::contract::check(&source).unwrap();
        assert!(super::check_physical(&source).unwrap_err().to_string()
            .contains("T-Gpu: probe smallest is not realized"));
        let mut ordinary = source;
        let Operator::Probe(_, key) = &mut ordinary.program.definitions[3].operator else { unreachable!() };
        key.choice = Some(Choice::First(Default::default()));
        physical::contract::check(&ordinary).unwrap();
        super::check_physical(&ordinary).unwrap();
    }
}


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
                supports: Vec::new(),
                ty: syn::parse_quote!(#range),
                capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
                // Range providers consume the recorded scalar/code carrier,
                // not logical type spellings. Native Scalar bounds and explicit
                // codec declarations establish the publication representation.
                elements: None,
                levels: levels.clone(),
            },
            ProviderToken {
                supports: Vec::new(),
                ty: syn::parse_quote!(#runs<_, _>),
                capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership, Capability::Resident],
                // The declared layout checks primitives and Symbol<T, Codec>
                // columns; logical symbol types do not appear in device rows.
                elements: None,
                levels: levels.clone(),
            },
            ProviderToken {
                supports: Vec::new(),
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
