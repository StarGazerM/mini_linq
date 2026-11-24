//! GPU provider tokens (STR-Provider): resident range reads, full-key radix relations
//! and columns, grouped radix indices and the canonical distinct builds. A
//! token's facts are declared here once; recorded entries carry its id and
//! spelling.
use cq_lang_structures::Declaration;

/// Every GPU declaration, in catalog order.
pub fn declarations() -> Vec<Declaration> {
    TOKENS.iter().map(|token| declaration(token)).collect()
}

/// Declared tokens in catalog order.
const TOKENS: [&str; 5] = [
    "range_provider",
    "radix_relation",
    "radix_layout",
    "device_columns",
    "radix_set",
];

/// The GPU declaration of `token`.
pub fn declaration(token: &str) -> Declaration {
    // One declaration is parsed per lookup; the table is never re-parsed whole.
    match token {
        "range_provider" => syn::parse_quote!(
            range_provider = ::cq_gpu_sequence_runtime::RangeProvider {
                reads [exact, range, membership], build inferred, abi range, contiguous_range
            };
        ),
        "radix_relation" => syn::parse_quote!(
            radix_relation = ::gpu_datalog_runtime::RadixLayout<_, (), _, u64, ()> {
                reads [exact, range, membership, resident, distinct], build typed, abi radix_relation
            };
        ),
        "radix_layout" => syn::parse_quote!(
            radix_layout = ::gpu_datalog_runtime::RadixLayout<_, _, _, _, _> {
                reads [exact, range, membership, candidates], build typed, abi radix
            };
        ),
        "device_columns" => syn::parse_quote!(
            device_columns = ::cq_gpu_runtime::DeviceColumns<_> {
                reads [], build typed, abi columns, bounds resident_columns
            };
        ),
        "radix_set" => syn::parse_quote!(
            radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> {
                reads [distinct, resident], build inferred
            };
        ),
        _ => panic!("STR-Provider: `{token}` is a GPU provider token"),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_token_declares_itself_once() {
        let declared = declarations();
        assert_eq!(declared.len(), TOKENS.len());
        for (declaration, token) in declared.iter().zip(TOKENS) {
            assert_eq!(declaration.token, token);
        }
    }
}
