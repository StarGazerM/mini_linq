// Full supported triangle product, split by planner to bound rustc memory.
// Push has only probe/first; merge and smallest are public UI rejection cases.
// Coarse/hypergraph/LE merge reads native leading keys, so it needs a BTreeMap.
// Fine planners retain ordered structures and allow every host representation.
// Seven planners have 224 rows; four fine planners have 320: 2,848 total.
macro_rules! matrix_branch {
    ($case:ident; [$($chosen:tt)*]; [$($next:tt)*]; $rest:tt) => {
        matrix_product!($case; [$($chosen)* $($next)*]; $rest);
    };
}
macro_rules! matrix_product {
    ($case:ident; [$($chosen:tt)*]; ()) => {
        pipeline_configuration! { $case, golden_scenarios; $($chosen)* }
    };
    ($case:ident; $chosen:tt; ({$($name:ident => $next:tt),+ $(,)?} $rest:tt)) => {
        $(mod $name { matrix_branch!($case; $chosen; $next; $rest); })+
    };
}

// The four independent rewrite/representation axes contribute 2^4 = 16 rows.
macro_rules! kernel_product {
    ($plan:tt, $join:tt, $intersections:tt, $hosts:tt, $printer:tt) => {
        matrix_product!(run_triangle; [planner: $plan; merge: $join;];
            ({ keep => [canon: [];], canon => [canon: [cq_rewrites::canon_prefixes,];] }
            ($intersections ($hosts
            ({ hash_dedup => [dedup: [];], ordered_dedup => [dedup: [cq_workload_passes::ordered_dedup,];] }
            ({ default_payload => [payload: [];], sorted => [payload: [cq_workload_passes::sorted_payload,];] }
            ({ eager => [membership: [];], lazy => [membership: [cq_rewrites::lazy_semijoins,];] }
            ($printer ()))))))));
    };
}
macro_rules! kernel_hosts {
    (all; $plan:tt, $join:tt, $intersections:tt, $printer:tt) => {
        kernel_product!($plan, $join, $intersections, {
            slice => [host: [take] slice [];],
            positional => [host: [take] slice [positional];],
            hashmap => [host: [take_hash] map [];],
            btreemap => [host: [take_btree] map [];],
        }, $printer);
    };
    (ordered; $plan:tt, $join:tt, $intersections:tt, $printer:tt) => {
        kernel_product!($plan, $join, $intersections, {
            btreemap => [host: [take_btree] map [];],
        }, $printer);
    };
}
macro_rules! full_matrix {
    ($plan:tt; merge_hosts: $merge_hosts:ident) => {
        mod probe_pull {
            kernel_hosts!(all; $plan, [], {
                first => [intersection: [];],
                smallest => [intersection: [cq_rewrites::smallest_intersections,];],
            }, { pull => [printer: mini_linq::passes::translate_pull;] });
        }
        mod merge_pull {
            kernel_hosts!($merge_hosts; $plan, [cq_rewrites::merge_joins,], {
                first => [intersection: [];],
                smallest => [intersection: [cq_rewrites::smallest_intersections,];],
            }, { pull => [printer: mini_linq::passes::translate_pull;] });
        }
        mod probe_push {
            kernel_hosts!(all; $plan, [], {
                first => [intersection: [];],
            }, { push => [printer: own_push::push;] });
        }
    };
}
