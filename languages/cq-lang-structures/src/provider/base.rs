//! Base `cq_storage` provider tokens (STR-Provider): the tokens structure
//! selection chooses, the sequential Pull catalog's tokens, and the tokens
//! authored `retype` advice may name without declaring them. A token's facts
//! are declared here once; selected entries record its id and spelling.
use crate::{Declaration, Provider, Providers};

/// Every base declaration, in Pull catalog order.
pub fn declarations() -> Vec<Declaration> {
    let providers: Providers = syn::parse_quote!(providers {
        hash_index = ::cq_storage::HashIndex<'_, _, _> {
            reads [exact, membership], supports [keys], build typed, join_filter
        };
        trie = ::cq_storage::Trie<'_, _, _> { reads [exact, membership], build typed, nested 2 };
        sorted_run = ::cq_storage::SortedRun<_, _> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed,
            join_filter, shared
        };
        radix_run = ::cq_storage::RadixRun<_, _> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed, shared,
            items 2 [rows ::std::vec::Vec<_>, columns ::cq_storage::TupleColumns<_>]
        };
        hash_set = ::cq_storage::HashSet<_> {
            reads [membership], build typed, accounting keys, shared
        };
        std_hash_set = ::std::collections::HashSet<_> {
            reads [membership], build typed, accounting keys, shared
        };
        btree_set = ::std::collections::BTreeSet<_> {
            reads [membership, ordered], build typed, shared
        };
        hash_map_grouped = ::std::collections::HashMap<_, ::std::vec::Vec<_>> {
            reads [exact, membership], supports [keys], build typed, shared
        };
        btree_map_grouped = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> {
            reads [exact, membership, ordered], supports [keys], build typed, shared
        };
        grouped_answers = ::cq_storage::GroupedAnswers<_, _> {
            reads [exact, membership], build typed, shared
        };
        single_group = ::cq_storage::SingleGroup<_, _> {
            reads [exact, membership], build typed, shared
        };
        keyset = ::cq_storage::dispatch::KeySet<_> {
            reads [membership], build typed, choice 1 [eager true, lazy false]
        };
        adaptive_keyset = ::cq_storage::AdaptiveKeySet<_, _, _> {
            reads [membership], build typed, shared
        };
        positional_keyset = ::cq_storage::AdaptiveKeySet<_, ::cq_storage::I64Codec, 67108864> {
            reads [membership], build typed, shared
        };
        adaptive_membership = ::cq_storage::AdaptiveMembership<_, 16777216> {
            reads [membership], build typed, shared
        };
        join_filter_membership = ::cq_storage::DenseMembership<_, 67108864> {
            reads [membership], build typed, shared
        };
        dense_membership = ::cq_storage::DenseMembership<_, 16777216> {
            reads [membership], build typed, shared
        };
        sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared };
        packed = ::cq_storage::dedup::Packed { reads [distinct], build inferred, shared };
        exists = ::cq_storage::Exists { reads [distinct], build inferred, shared };
        sharded_index = ::cq_storage::sharded::Index<_, _> {
            reads [exact, membership], supports [keys], build typed, join_filter, shared
        };
        sharded = ::cq_storage::sharded::Sharded<_> {
            reads [], build typed, routed 0, shared
        };
        vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows };
        host_scan = ::core::iter::Map<_, _> { reads [], build typed };
        group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> {
            reads [], build typed
        };
        selection = ::cq_storage::Selection<_> { reads [positions], build typed, shared };
    });
    providers.declarations
}

/// The base declaration of `token`.
pub fn declaration(token: &str) -> Declaration {
    declarations()
        .into_iter()
        .find(|declaration| declaration.token == token)
        .unwrap_or_else(|| panic!("STR-Provider: `{token}` is a base provider token"))
}

/// An instance of base token `token` spelled `ty`.
pub fn instance(token: &str, ty: syn::Type) -> Provider {
    Provider {
        token: declaration(token).token,
        inner: None,
        input: None,
        radix: None,
        ty,
    }
}
