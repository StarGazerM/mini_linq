// Sequential-vs-Tasks matrix for the triangle kernel.
//
// The executable corpus (full_matrix.rs) keeps the full supported product.
// Conformance compares the two Sequence translations of the same program, so
// it keeps the full product only over the choices that change that program
// (planner x join x intersection) and covers host representation and the four
// independent rewrite/representation axes pairwise: every pair of values of
// any two axes occurs in some row. Printer rows are omitted because
// conformance replaces the printer (push rows duplicated pull rows here).
//
// With four hosts, each host occurs twice with complementary rewrite choices
// (vectors 0000/1111, 0011/1100, 0101/1010, 0110/1001 over canon, dedup,
// payload, membership): 8 rows. With the ordered host only, the five rows
// 0000, 0111, 1011, 1101, 1110 cover every pair of the four binary axes.
// Planners with ordered merge hosts: 2*8 + 2*5 = 26 rows; all hosts: 32.
macro_rules! sequence_row {
    ($name:ident, $plan:tt, $join:tt, $intersection:tt, [$($host:tt)*],
     $canon:tt, $dedup:tt, $payload:tt, $membership:tt) => {
        mod $name {
            pipeline_configuration! { run_triangle, golden_scenarios;
                planner: $plan;
                merge: $join;
                canon: $canon;
                intersection: $intersection;
                host: $($host)*;
                dedup: $dedup;
                payload: $payload;
                membership: $membership;
                printer: mini_linq::passes::translate_pull; }
        }
    };
}
macro_rules! sequence_hosts {
    (all; $plan:tt, $join:tt, $intersection:tt) => {
        sequence_row!(slice_0000, $plan, $join, $intersection, [[take] slice []],
            [], [], [], []);
        sequence_row!(slice_1111, $plan, $join, $intersection, [[take] slice []],
            [cq_rewrites::canon_prefixes,], [cq_workload_passes::ordered_dedup,],
            [cq_workload_passes::sorted_payload,], [cq_rewrites::lazy_semijoins,]);
        sequence_row!(positional_0011, $plan, $join, $intersection, [[take] slice [positional]],
            [], [], [cq_workload_passes::sorted_payload,], [cq_rewrites::lazy_semijoins,]);
        sequence_row!(positional_1100, $plan, $join, $intersection, [[take] slice [positional]],
            [cq_rewrites::canon_prefixes,], [cq_workload_passes::ordered_dedup,], [], []);
        sequence_row!(hashmap_0101, $plan, $join, $intersection, [[take_hash] map []],
            [], [cq_workload_passes::ordered_dedup,], [], [cq_rewrites::lazy_semijoins,]);
        sequence_row!(hashmap_1010, $plan, $join, $intersection, [[take_hash] map []],
            [cq_rewrites::canon_prefixes,], [], [cq_workload_passes::sorted_payload,], []);
        sequence_row!(btreemap_0110, $plan, $join, $intersection, [[take_btree] map []],
            [], [cq_workload_passes::ordered_dedup,], [cq_workload_passes::sorted_payload,], []);
        sequence_row!(btreemap_1001, $plan, $join, $intersection, [[take_btree] map []],
            [cq_rewrites::canon_prefixes,], [], [], [cq_rewrites::lazy_semijoins,]);
    };
    (ordered; $plan:tt, $join:tt, $intersection:tt) => {
        sequence_row!(btreemap_0000, $plan, $join, $intersection, [[take_btree] map []],
            [], [], [], []);
        sequence_row!(btreemap_0111, $plan, $join, $intersection, [[take_btree] map []],
            [], [cq_workload_passes::ordered_dedup,], [cq_workload_passes::sorted_payload,],
            [cq_rewrites::lazy_semijoins,]);
        sequence_row!(btreemap_1011, $plan, $join, $intersection, [[take_btree] map []],
            [cq_rewrites::canon_prefixes,], [], [cq_workload_passes::sorted_payload,],
            [cq_rewrites::lazy_semijoins,]);
        sequence_row!(btreemap_1101, $plan, $join, $intersection, [[take_btree] map []],
            [cq_rewrites::canon_prefixes,], [cq_workload_passes::ordered_dedup,], [],
            [cq_rewrites::lazy_semijoins,]);
        sequence_row!(btreemap_1110, $plan, $join, $intersection, [[take_btree] map []],
            [cq_rewrites::canon_prefixes,], [cq_workload_passes::ordered_dedup,],
            [cq_workload_passes::sorted_payload,], []);
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
