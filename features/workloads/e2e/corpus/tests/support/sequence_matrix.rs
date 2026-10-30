// Sequential-vs-Tasks matrix for the triangle kernel.
//
// The executable corpus (full_matrix.rs) keeps the full supported product.
// Conformance compares the two Sequence translations of the same program, so
// it keeps the full product only over the choices that change that program
// (planner x join x intersection) and covers host representation and the three
// independent rewrite/representation axes pairwise: every pair of values of
// any two axes occurs in some row. Printer rows are omitted because
// conformance replaces the printer (push rows duplicated pull rows here).
//
// With four hosts, each host occurs twice with complementary rewrite choices
// (vectors 000/111, 011/100, 001/110, 010/101 over canon, payload,
// membership): 8 rows. With the ordered host only, the retained five rows
// 000, 011, 111, 101, 110 cover every pair of the three binary axes.
// Planners with ordered merge hosts: 2*8 + 2*5 = 26 rows; all hosts: 32.
macro_rules! sequence_row {
    ($name:ident, $plan:tt, $join:tt, $intersection:tt, [$($host:tt)*],
     $canon:tt, $payload:tt, $membership:tt) => {
        mod $name {
            pipeline_configuration! { run_triangle, golden_scenarios;
                planner: $plan;
                merge: $join;
                canon: $canon;
                intersection: $intersection;
                host: $($host)*;
                payload: $payload;
                membership: $membership;
                printer: mini_linq::passes::translate_pull; }
        }
    };
}
macro_rules! sequence_hosts {
    (all; $plan:tt, $join:tt, $intersection:tt) => {
        sequence_row!(slice_000, $plan, $join, $intersection, [[take] slice []],
            [], [], []);
        sequence_row!(slice_111, $plan, $join, $intersection, [[take] slice []],
            [cq_rewrites::canon_prefixes,],
            [cq_workload_passes::sorted_payload,], [cq_rewrites::lazy_semijoins,]);
        sequence_row!(positional_011, $plan, $join, $intersection, [[take] slice [positional]],
            [], [cq_workload_passes::sorted_payload,], [cq_rewrites::lazy_semijoins,]);
        sequence_row!(positional_100, $plan, $join, $intersection, [[take] slice [positional]],
            [cq_rewrites::canon_prefixes,], [], []);
        sequence_row!(hashmap_001, $plan, $join, $intersection, [[take_hash] map []],
            [], [], [cq_rewrites::lazy_semijoins,]);
        sequence_row!(hashmap_110, $plan, $join, $intersection, [[take_hash] map []],
            [cq_rewrites::canon_prefixes,], [cq_workload_passes::sorted_payload,], []);
        sequence_row!(btreemap_010, $plan, $join, $intersection, [[take_btree] map []],
            [], [cq_workload_passes::sorted_payload,], []);
        sequence_row!(btreemap_101, $plan, $join, $intersection, [[take_btree] map []],
            [cq_rewrites::canon_prefixes,], [], [cq_rewrites::lazy_semijoins,]);
    };
    (ordered; $plan:tt, $join:tt, $intersection:tt) => {
        sequence_row!(btreemap_000, $plan, $join, $intersection, [[take_btree] map []],
            [], [], []);
        sequence_row!(btreemap_011, $plan, $join, $intersection, [[take_btree] map []],
            [], [cq_workload_passes::sorted_payload,], [cq_rewrites::lazy_semijoins,]);
        sequence_row!(btreemap_111, $plan, $join, $intersection, [[take_btree] map []],
            [cq_rewrites::canon_prefixes,], [cq_workload_passes::sorted_payload,],
            [cq_rewrites::lazy_semijoins,]);
        sequence_row!(btreemap_101, $plan, $join, $intersection, [[take_btree] map []],
            [cq_rewrites::canon_prefixes,], [], [cq_rewrites::lazy_semijoins,]);
        sequence_row!(btreemap_110, $plan, $join, $intersection, [[take_btree] map []],
            [cq_rewrites::canon_prefixes,], [cq_workload_passes::sorted_payload,], []);
    };
}
macro_rules! sequence_matrix {
    ($plan:tt; merge_hosts: $merge_hosts:ident) => {
        mod probe_first {
            sequence_hosts!(all; $plan, [], []);
        }
        mod probe_smallest {
            sequence_hosts!(all; $plan, [], [cq_rewrites::smallest_intersections,]);
        }
        mod merge_first {
            sequence_hosts!($merge_hosts; $plan, [cq_rewrites::merge_joins,], []);
        }
        mod merge_smallest {
            sequence_hosts!($merge_hosts; $plan, [cq_rewrites::merge_joins,],
                [cq_rewrites::smallest_intersections,]);
        }
    };
}
