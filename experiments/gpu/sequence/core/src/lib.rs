//! Checked Sequence-to-CUDA translation over resident column providers.

use cq_lang_sequence::{LevelNode, Machine};

mod arena;
// DESIGN gpu-provider-records
// P3d/T-Gpu: select once into GPU analysis; check recorded facts and print them.
// Runtime layout algorithms belong to the selected provider, never the emitter.
mod bindings;
mod capture;
pub mod constants;
mod domains;
mod emit;
mod grouping;
pub mod layout;
pub mod providers;
mod realize;
mod reduce;
mod rows;
pub mod runs;

pub use arena::translate_arena;
pub use emit::translate;

/// Select target-owned accumulator providers before checking GPU Structures.
/// The only grouping selection site: pipelines reach it through `stage`, the
/// TPC-H build before Sequence lowering, and authored fixtures explicitly.
pub fn bind_structures(
    source: &cq_lang_structures::Program,
) -> syn::Result<cq_lang_structures::Program> {
    grouping::bind(source)
}

/// Validate the Structures boundary without selecting CPU batching.
/// GPU batching and storage scheduling are recorded later in Sequence.
pub fn check_structures(
    source: &cq_lang_structures::Program,
    analysis: &cq_lang_structures::Analysis,
) -> syn::Result<()> {
    // PH-Smallest is admitted here; GPU-Smallest realization records it or
    // rejects it before printing (never as a First probe).
    grouping::check(source, analysis)?;
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

/// Collective block width of every generated CUDA template.
pub(crate) const BLOCK_THREADS: u32 = 256;

/// Host publication over separated, dynamically scheduled device blocks.
/// Each block is collaborative; lanes are sequential leaves (no warp level).
pub fn machine() -> Machine {
    Machine {
        gpu: Some(cq_lang_sequence::gpu::Machine {
            block_threads: BLOCK_THREADS,
            reserve_rows: 256,
            retain_prefix_rows: 65536,
            bindings: bindings::select,
            realize: realize::select,
        }),
        name: syn::parse_quote!(gpu),
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
        },
    }
}

/// Resident range reads, order-preserving codes and flat packed publication.
///
/// The tokens name runtime implementations without importing the runtime into
/// this compile-time target. Distinct providers type a canon result's rows
/// record; their build (sort/unique) runs at a separated publication, not
/// inside a collaborative block or lane.
pub fn catalog() -> cq_lang_sequence::Catalog {
    use cq_lang_sequence::{Catalog, CodecToken};

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
    let mut catalog = Catalog {
        declarations: Vec::new(),
        providers: Vec::new(),
        codecs: Vec::new(),
    };
    // Range providers consume the recorded scalar/code carrier, not logical
    // type spellings. Native Scalar bounds and explicit codec declarations
    // establish the publication representation.
    catalog.register(providers::declaration("range_provider"), levels.clone());
    // Input schema and codecs are recorded by the provider's producer;
    // runtime trait obligations check the opaque storage spellings.
    catalog.register(providers::declaration("device_layout"), levels.clone());
    // Formal rule GPU-RadixAdvice: grouped radix indices are selected only by
    // authored instances carrying their advice; registered after the existing
    // tokens, so unadvised catalog choices are unchanged.
    catalog.register(providers::declaration("radix_layout"), levels.clone());
    catalog.register(
        cq_lang_structures::provider::base::declaration("sorted_run"),
        vec![vec!["host".into()]],
    );
    // Formal rule PL-Distinct: the device sort/unique build of a canonical
    // rows record. Its scan is the publication itself.
    catalog.register(providers::declaration("packed_canon"), levels.clone());
    catalog.register(providers::declaration("wide_canon"), levels.clone());
    // `None` is the identity representation of columns without a declared codec.
    catalog.codecs = vec![CodecToken {
        codec_id: None,
        codec: None,
        ordered: true,
        levels: std::iter::once(vec!["host".into()])
            .chain(levels.iter().cloned())
            .collect(),
    }];
    catalog
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_catalog_only_authorizes_identity() {
        let catalog = super::catalog();
        assert_eq!(catalog.codecs.len(), 1);
        assert_eq!(catalog.codecs[0].codec_id, None);
        assert_eq!(catalog.codecs[0].codec, None);
        assert!(catalog.codecs[0].ordered);
    }
}
