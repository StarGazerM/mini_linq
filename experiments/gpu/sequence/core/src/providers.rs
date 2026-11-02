//! GPU provider tokens (STR-Provider): resident range reads, device layouts
//! and columns, grouped radix indices and the canonical distinct builds. A
//! token's facts are declared here once; recorded entries carry its id and
//! spelling.
use cq_lang_structures::{Declaration, Providers};

/// Every GPU declaration, in catalog order.
pub fn declarations() -> Vec<Declaration> {
    let providers: Providers = syn::parse_quote!(providers {
        range_provider = ::cq_gpu_sequence_runtime::RangeProvider {
            reads [exact, range, membership], build inferred, abi range, contiguous_range
        };
        device_layout = ::gpu_datalog_runtime::DeviceLayout<_, _> {
            reads [exact, range, membership, resident], build macro, abi runs
        };
        radix_layout = ::gpu_datalog_runtime::RadixLayout<_, _, _, _, _> {
            reads [exact, range, membership, candidates], build macro, abi radix
        };
        device_columns = ::cq_gpu_runtime::DeviceColumns<_> {
            reads [], build macro, abi columns, bounds resident_columns
        };
        packed_canon = ::cq_gpu_sequence_runtime::canon::PackedCanon {
            reads [distinct], build inferred, packed_bits 128
        };
        wide_canon = ::cq_gpu_sequence_runtime::canon::WideCanon {
            reads [distinct], build inferred
        };
    });
    providers.declarations
}

/// The GPU declaration of `token`.
pub fn declaration(token: &str) -> Declaration {
    declarations()
        .into_iter()
        .find(|declaration| declaration.token == token)
        .unwrap_or_else(|| panic!("STR-Provider: `{token}` is a GPU provider token"))
}
