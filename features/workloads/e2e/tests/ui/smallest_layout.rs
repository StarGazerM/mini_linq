// Selection normally supplies r = Trie [b][c]. This hand-authored Structures
// program omits that retained candidate index, while keeping the host-side
// candidate/membership decisions. Sequence lowering must reject the missing
// two-level layout, not silently fall back to an ordinary full-key index.
fn main() {
    let L = [(1_i32, 2_i32)];
    let R = [(2_i32, 3_i32)];
    let S = [(1_i32, 3_i32)];
    let _ = mini_linq::passes::sequence! {
        pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation L(a:i32,b:i32);
            relation R(b:i32,c:i32);
            relation S(a:i32,c:i32);
            l = pipe rename L {a -> a,b -> b};
            r0 = pipe rename R {b -> b,c -> c};
            r = let project r0 keep {b,c};
            s = pipe rename S {a -> a,c -> c};
            candidate = pipe probe smallest [b] l,r;
            checked = let canon semi eager [a,c] candidate,s;
            return checked as answer(a,b,c);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; keyset = ::cq_storage::dispatch::KeySet<_> { reads [membership], build typed }; hash_index = ::cq_storage::HashIndex<'_, _, _> { reads [exact, membership], supports [keys], build typed, join_filter, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact hash_finalize stat_rows host optional build single, keys: keys distinct_keys exact hash_finalize stat_keys host optional build single, max_group: max_group max_bucket_occurrences exact hash_finalize stat_max_group host optional build single ] }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            l host scan = @host_scan as ::core::iter::Map<_, _> [];
            r0 host scan = @host_scan as ::core::iter::Map<_, _> [];
            r rows = @vec as ::std::vec::Vec<_>;
            s host scan = @host_scan as ::core::iter::Map<_, _> [];
            s host result(checked) = @keyset as ::cq_storage::dispatch::KeySet<_> [a,c];
            s host candidates = @hash_index as ::cq_storage::HashIndex<'_, _, _> [a];
            s host membership = @keyset as ::cq_storage::dispatch::KeySet<_> [a,c];
            checked rows = @sort as ::cq_storage::dedup::Sort; checked canon = rows;
        }
    };
}
