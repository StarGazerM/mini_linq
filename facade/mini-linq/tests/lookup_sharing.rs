//! Authored sharing decisions preserve scalar probe answers and lookup counts.
#![recursion_limit = "512"]

#[test]
fn shared_lookup_expands_ranges_in_scalar_order() {
    let left = [(0_i32,), (2,), (1,), (3,), (2,), (4,)];
    let right = [(2_i32, 20_i32), (1, 11), (2, 21), (2, 22), (4, 40)];
    cq_storage::counters::reset();
    let result: Vec<_> = mini_linq::passes::sequence! {
        pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation L(k:i32) = (left); relation R(k:i32,v:i32) = (right);
            l = pipe rename L@0 {k -> k};
            r = let rename R@1 {k -> k,v -> v};
            s = pipe semi eager [k] l,r;
            kept = pipe select s if (k >= 1);
            projected = pipe project kept keep {k};
            p = pipe probe first [k] projected,r;
            out = pipe canon project p keep {k,v};
            return out as answer(k,v);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r = ::cq_storage::SortedRun<_,_> [k];
            s share = lookup r [k] first s force p;
            s repr = tuples witness { r [k] at s as ::cq_storage::RunCursor<'_, _> };
            p stage = batch 2 expand;
            out canon = ::std::collections::HashSet<_>;
        }
    }
    .map(|(k, v)| (*k, *v))
    .collect();
    assert_eq!(result, [(2, 20), (2, 21), (2, 22), (1, 11), (4, 40)]);
    assert_eq!(cq_storage::counters::snapshot().lookups, left.len() as u64);
}

#[test]
fn positional_witness_is_consumed_by_scalar_probe() {
    let left = [(1_i32,), (3,), (2,)];
    let right = [(2_i32, 20_i32), (1, 11), (2, 21)];
    cq_storage::counters::reset();
    let result: Vec<_> = mini_linq::passes::sequence! {
        pipeline { mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation L(k:i32) = (left); relation R(k:i32,v:i32) = (right);
            l = pipe rename L@0 {k -> k};
            r = let rename R@1 {k -> k,v -> v};
            s = pipe semi eager [k] l,r;
            kept = pipe select s if (k >= 1);
            p = pipe probe first [k] kept,r;
            out = pipe canon project p keep {k,v};
            return out as answer(k,v);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            l repr = positions { L lineage {k -> k} } owned [];
            r = ::cq_storage::SortedRun<_,_> [k];
            s share = lookup r [k] first s force p;
            s repr = positions { L lineage {k -> k} } owned [] witness { r [k] at s as ::cq_storage::RunCursor<'_, _> };
            kept repr = positions { L lineage {k -> k} } owned [];
            out canon = ::std::collections::HashSet<_>;
        }
    }.map(|(k,v)| (*k,*v)).collect();
    assert_eq!(result, [(1, 11), (2, 20), (2, 21)]);
    assert_eq!(cq_storage::counters::snapshot().lookups, left.len() as u64);
}
