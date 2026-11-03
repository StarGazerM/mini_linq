//! GPU provider tokens (STR-Provider): resident range reads, full-key radix relations
//! and columns, grouped radix indices, the canonical distinct builds and
//! retained shared-S1 scan blocks. A token's facts are declared here once;
//! recorded entries carry its id and spelling.
use cq_lang_structures::{Declaration, Providers};

/// Every GPU declaration, in catalog order.
pub fn declarations() -> Vec<Declaration> {
    let providers: Providers = syn::parse_quote!(providers {
        range_provider = ::cq_gpu_sequence_runtime::RangeProvider {
            reads [exact, range, membership], build inferred, abi range, contiguous_range
        };
        radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> {
            reads [exact, range, membership, resident, distinct], build typed, abi radix_relation
        };
        radix_layout = ::gpu_datalog_runtime::RadixLayout<_, _, _, _, _> {
            reads [exact, range, membership, candidates], build typed, abi radix
        };
        device_columns = ::cq_gpu_runtime::DeviceColumns<_> {
            reads [], build typed, abi columns, bounds resident_columns
        };
        device_scan = ::cq_gpu_runtime::scan_table::DeviceScanTable<_> {
            reads [], build typed, abi scan_blocks
        };
        // ComparatorRows: the build keys every row by its complete packed
        // row code (GPU-RadixSet), injective over the row's recorded columns,
        // and retains one representative per equal key.
        radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> {
            reads [distinct, resident, comparator_rows], build inferred
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

