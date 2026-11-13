//! Base `cq_storage` provider tokens (STR-Provider): the tokens structure
//! selection chooses, the sequential Pull catalog's tokens, and the tokens
//! authored `retype` advice may name without declaring them. A token's facts
//! are declared here once; selected entries record its id and spelling.
use crate::{Declaration, Provider};

/// Every base declaration, in Pull catalog order.
pub fn declarations() -> Vec<Declaration> {
    TOKENS.iter().map(|token| declaration(token)).collect()
}

/// Declared tokens in catalog order.
const TOKENS: [&str; 28] = [
    "hash_index",
    "trie",
    "sorted_run",
    "radix_run",
    "hash_set",
    "std_hash_set",
    "btree_set",
    "hash_map_grouped",
    "btree_map_grouped",
    "grouped_answers",
    "single_group",
    "keyset",
    "adaptive_keyset",
    "positional_keyset",
    "adaptive_membership",
    "join_filter_membership",
    "attached_join_filter_membership",
    "borrowed_run_filter",
    "dense_membership",
    "sort",
    "packed",
    "exists",
    "sharded_index",
    "sharded",
    "vec",
    "host_scan",
    "group_map",
    "selection",
];

/// The base declaration of `token`.
pub fn declaration(token: &str) -> Declaration {
    // One declaration is parsed per lookup; the table is never re-parsed whole.
    match token {
        "hash_index" => syn::parse_quote!(
            hash_index = ::cq_storage::HashIndex<'_, _, _> {
                reads [exact, membership], supports [keys], build typed, join_filter,
                statistics_build [rows, max_group], statistics [
                    rows: rows occurrences exact hash_finalize stat_rows host optional build single,
                    keys: keys distinct_keys exact hash_finalize stat_keys host optional build single,
                    max_group: max_group max_bucket_occurrences exact hash_finalize stat_max_group host optional build single
                ]
            };
        ),
        "trie" => syn::parse_quote!(
            trie = ::cq_storage::Trie<'_, _, _> { reads [exact, membership], build typed, nested 2 };
        ),
        "sorted_run" => syn::parse_quote!(
            sorted_run = ::cq_storage::SortedRun<_, _> {
                reads [exact, range, membership], supports [retain, expand, keys], build typed,
                join_filter, run_filter, shared,
                statistics_build [rows, max_group], statistics [
                    rows: rows occurrences exact run_finalize stat_rows host required build single,
                    keys: keys distinct_keys exact run_finalize stat_keys host required build single,
                    max_group: max_group max_bucket_occurrences exact run_finalize stat_max_group host required build single,
                    runs: runs runs exact run_finalize stat_runs host required build single
                ]
            };
        ),
        "radix_run" => syn::parse_quote!(
            radix_run = ::cq_storage::RadixRun<_, _> {
                reads [exact, range, membership], supports [retain, expand, keys], build typed,
                join_filter, run_filter, shared,
                items 2 [rows ::std::vec::Vec<_>, columns ::cq_storage::TupleColumns<_>],
                statistics_build [rows, max_group], statistics [
                    rows: rows occurrences exact run_finalize stat_rows host required build overlapping,
                    keys: keys distinct_keys exact run_finalize stat_keys host optional build single,
                    max_group: max_group max_bucket_occurrences exact run_finalize stat_max_group host optional build single,
                    runs: runs runs exact run_finalize stat_runs host required build overlapping
                ]
            };
        ),
        "hash_set" => syn::parse_quote!(
            hash_set = ::cq_storage::HashSet<_> {
                reads [membership], build typed, accounting keys, shared
            };
        ),
        "std_hash_set" => syn::parse_quote!(
            std_hash_set = ::std::collections::HashSet<_> {
                reads [membership], build typed, accounting keys, shared
            };
        ),
        "btree_set" => syn::parse_quote!(
            btree_set = ::std::collections::BTreeSet<_> {
                reads [membership, ordered], build typed, shared
            };
        ),
        "hash_map_grouped" => syn::parse_quote!(
            hash_map_grouped = ::std::collections::HashMap<_, ::std::vec::Vec<_>> {
                reads [exact, membership], supports [keys], build typed, shared
            };
        ),
        "btree_map_grouped" => syn::parse_quote!(
            btree_map_grouped = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> {
                reads [exact, membership, ordered], supports [keys], build typed, shared
            };
        ),
        "grouped_answers" => syn::parse_quote!(
            grouped_answers = ::cq_storage::GroupedAnswers<_, _> {
                reads [exact, membership], build typed, shared
            };
        ),
        "single_group" => syn::parse_quote!(
            single_group = ::cq_storage::SingleGroup<_, _> {
                reads [exact, membership], build typed, shared
            };
        ),
        "keyset" => syn::parse_quote!(
            keyset = ::cq_storage::dispatch::KeySet<_> {
                reads [membership], build typed, choice 1 [eager true, lazy false]
            };
        ),
        "adaptive_keyset" => syn::parse_quote!(
            adaptive_keyset = ::cq_storage::AdaptiveKeySet<_, _, _> {
                reads [membership], build typed, shared
            };
        ),
        "positional_keyset" => syn::parse_quote!(
            positional_keyset = ::cq_storage::AdaptiveKeySet<_, ::cq_storage::I64Codec, 67108864> {
                reads [membership], build typed, shared
            };
        ),
        "adaptive_membership" => syn::parse_quote!(
            adaptive_membership = ::cq_storage::AdaptiveMembership<_, 16777216> {
                reads [membership], build typed, shared
            };
        ),
        "join_filter_membership" => syn::parse_quote!(
            join_filter_membership = ::cq_storage::DenseMembership<_, 67108864> {
                reads [membership], build typed, shared
            };
        ),
        "attached_join_filter_membership" => syn::parse_quote!(
            attached_join_filter_membership = ::cq_storage::AttachedDenseMembership<_, 67108864> {
                reads [membership], build typed, attached_filter, shared
            };
        ),
        "borrowed_run_filter" => syn::parse_quote!(
            borrowed_run_filter = ::cq_storage::RunFilterView<'_, _, _> {
                reads [membership], borrowed_filter, shared
            };
        ),
        "dense_membership" => syn::parse_quote!(
            dense_membership = ::cq_storage::DenseMembership<_, 16777216> {
                reads [membership], build typed, shared
            };
        ),
        "sort" => syn::parse_quote!(
            sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared };
        ),
        "packed" => syn::parse_quote!(
            packed = ::cq_storage::dedup::Packed { reads [distinct], build inferred, shared };
        ),
        "exists" => syn::parse_quote!(
            exists = ::cq_storage::Exists { reads [distinct], build inferred, shared };
        ),
        "sharded_index" => syn::parse_quote!(
            sharded_index = ::cq_storage::sharded::Index<_, _> {
                reads [exact, membership], supports [keys], build typed, join_filter, shared,
                statistics_build [rows, max_group], statistics [
                    rows: rows occurrences exact hash_finalize stat_rows host required build single,
                    keys: keys distinct_keys exact hash_finalize stat_keys host required build single,
                    max_group: max_group max_bucket_occurrences exact hash_finalize stat_max_group host required build single
                ]
            };
        ),
        "sharded" => syn::parse_quote!(
            sharded = ::cq_storage::sharded::Sharded<_> {
                reads [], build typed, routed 0, shared
            };
        ),
        "vec" => syn::parse_quote!(
            vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows };
        ),
        "host_scan" => syn::parse_quote!(
            host_scan = ::core::iter::Map<_, _> { reads [], build typed };
        ),
        "group_map" => syn::parse_quote!(
            group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> {
                reads [], build typed
            };
        ),
        "selection" => syn::parse_quote!(
            selection = ::cq_storage::Selection<_> { reads [positions], build typed, shared };
        ),
        _ => panic!("STR-Provider: `{token}` is a base provider token"),
    }
}

/// An instance of base token `token` spelled `ty`.
pub fn instance(token: &str, ty: syn::Type) -> Provider {
    Provider {
        token: declaration(token).token,
        inner: None,
        input: None,
        radix: None,
        construction: crate::stats::Construction::Ordinary,
        ty,
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
        let mut names = TOKENS.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TOKENS.len());
    }
}
