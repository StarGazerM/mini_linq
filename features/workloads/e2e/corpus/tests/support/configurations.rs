// Named choices; each identifier spells out the values in axis order.
// The integer witness is emitted beside the corresponding executable test.
// Registrations follow Pipeline::rejection on the current checked physical plan:
// native leading merges need ordered hosts; push cannot consume retained merges.
// configuration_consistency checks eligibility separately from pairwise coverage.
macro_rules! configuration {
    (written_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take] slice [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_coarse_probe_keep_first_slice_hash_dedup_default_eager_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take] slice [positional];
        dedup: [];
        payload: [];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (written_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [0, 0, 0, 0, 1, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, coarse];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_coarse_probe_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [0, 0, 1, 1, 0, 1, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, coarse];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [0, 0, 1, 1, 2, 0, 1, 0, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, coarse];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (written_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [0, 1, 0, 0, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_coarse_merge_canon_first_hashmap_ordered_dedup_sorted_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [0, 1, 1, 0, 1, 1, 1, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_coarse_merge_canon_smallest_btreemap_ordered_dedup_sorted_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [0, 1, 1, 1, 2, 1, 1, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_fine_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [1, 0, 0, 0, 1, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, fine];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [1, 0, 0, 0, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, fine];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_fine_probe_keep_smallest_hashmap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [1, 0, 0, 1, 1, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, fine];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [1, 0, 1, 0, 0, 0, 1, 1, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, fine];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take] slice [];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [1, 1, 0, 1, 1, 1, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [1, 1, 0, 1, 2, 0, 0, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (written_fine_merge_canon_first_slice_ordered_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [1, 1, 1, 0, 0, 1, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [1, 1, 1, 1, 0, 1, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [written, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (hypergraph_probe_keep_first_hashmap_ordered_dedup_sorted_lazy_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 0, 0, 0, 1, 1, 1, 1, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (hypergraph_probe_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 0, 0, 1, 1, 1, 1, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (hypergraph_probe_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 0, 0, 1, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (hypergraph_probe_canon_first_slice_hash_dedup_default_eager_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 0, 1, 0, 0, 0, 0, 0, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take] slice [positional];
        dedup: [];
        payload: [];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 0, 1, 1, 0, 0, 1, 0, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (hypergraph_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 1, 0, 0, 1, 1, 0, 1, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (hypergraph_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 1, 0, 0, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 1, 0, 0, 2, 1, 1, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (hypergraph_merge_canon_first_slice_hash_dedup_default_eager_pull_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 1, 1, 0, 0, 0, 0, 0, 0, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take] slice [positional];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (hypergraph_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [2, 1, 1, 1, 0, 1, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [hypergraph];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_connected_coarse_probe_keep_first_slice_ordered_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [3, 0, 0, 0, 0, 1, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_connected_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [3, 0, 0, 0, 1, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [3, 0, 1, 0, 0, 1, 1, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, coarse];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take] slice [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_connected_coarse_merge_canon_smallest_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [3, 1, 1, 1, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_connected_fine_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 0, 0, 0, 1, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_connected_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 0, 0, 0, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_connected_fine_probe_keep_first_btreemap_hash_dedup_default_lazy_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 0, 0, 0, 2, 0, 0, 1, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_connected_fine_probe_keep_smallest_slice_hash_dedup_default_eager_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 0, 0, 1, 0, 0, 0, 0, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [];
        payload: [];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 0, 1, 0, 0, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take] slice [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 0, 1, 1, 2, 0, 0, 0, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 1, 0, 1, 1, 1, 1, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_connected_fine_merge_canon_first_hashmap_ordered_dedup_sorted_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 1, 1, 0, 1, 1, 1, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_eager_pull_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 1, 1, 1, 0, 1, 1, 0, 0, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [4, 1, 1, 1, 0, 1, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix connected, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_greedy_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [5, 0, 0, 0, 1, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_greedy_coarse_probe_keep_smallest_slice_hash_dedup_sorted_eager_pull_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [5, 0, 0, 1, 0, 0, 1, 0, 0, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, coarse];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_default_eager_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [5, 0, 0, 1, 0, 1, 0, 0, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, coarse];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [5, 0, 0, 1, 0, 1, 1, 1, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, coarse];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_greedy_coarse_probe_canon_first_hashmap_ordered_dedup_default_lazy_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [5, 0, 1, 0, 1, 1, 0, 1, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, coarse];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_greedy_coarse_probe_canon_first_btreemap_hash_dedup_sorted_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [5, 0, 1, 0, 2, 0, 1, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, coarse];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [5, 0, 1, 1, 2, 1, 0, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, coarse];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_greedy_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [5, 1, 0, 0, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_greedy_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [6, 0, 0, 0, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, fine];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_greedy_fine_probe_canon_first_hashmap_ordered_dedup_default_eager_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [6, 0, 1, 0, 1, 1, 0, 0, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, fine];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [6, 0, 1, 1, 0, 0, 1, 0, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, fine];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [6, 0, 1, 1, 0, 0, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, fine];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [6, 1, 0, 0, 1, 1, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [6, 1, 0, 0, 1, 1, 0, 1, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (prefix_greedy_fine_merge_keep_smallest_slice_hash_dedup_sorted_lazy_pull_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [6, 1, 0, 1, 0, 0, 1, 1, 0, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [6, 1, 0, 1, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [6, 1, 1, 1, 0, 1, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [prefix greedy, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take] slice [positional];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 0, 0, 0, 1, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 0, 0, 0, 1, 0, 0, 0, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (elimination_coarse_probe_keep_first_btreemap_ordered_dedup_default_lazy_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 0, 0, 0, 2, 1, 0, 1, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (elimination_coarse_probe_keep_smallest_hashmap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 0, 0, 1, 1, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_coarse_probe_canon_first_slice_hash_dedup_default_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 0, 1, 0, 0, 0, 0, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take] slice [positional];
        dedup: [];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (elimination_coarse_probe_canon_smallest_slice_hash_dedup_sorted_eager_pull_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 0, 1, 1, 0, 0, 1, 0, 0, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 1, 0, 0, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_coarse_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 1, 0, 0, 2, 1, 1, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_coarse_merge_keep_smallest_btreemap_hash_dedup_sorted_lazy_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 1, 0, 1, 2, 0, 1, 1, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (elimination_coarse_merge_canon_smallest_slice_hash_dedup_sorted_eager_pull_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 1, 1, 1, 0, 0, 1, 0, 0, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [7, 1, 1, 1, 0, 1, 1, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, coarse];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 0, 0, 0, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [];
        canon: [];
        intersection: [];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_fine_probe_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 0, 0, 1, 2, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 0, 0, 1, 2, 1, 0, 0, 1, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_btree] map [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [];
        print_input: structures;
        printer: own_push::push; }
    };
    (elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 0, 1, 0, 0, 1, 1, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take] slice [];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 0, 1, 0, 0, 1, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (elimination_fine_probe_canon_smallest_hashmap_hash_dedup_sorted_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 0, 1, 1, 1, 0, 1, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_hash] map [];
        dedup: [];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_fine_merge_keep_first_slice_ordered_dedup_default_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 1, 0, 0, 0, 1, 0, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
    (elimination_fine_merge_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 1, 0, 0, 1, 0, 0, 0, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 1, 0, 1, 1, 0, 0, 1, 0, 0];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take_hash] map [];
        dedup: [];
        payload: [];
        membership: [cq_rewrites::lazy_semijoins,];
        printer: mini_linq::passes::translate_pull; }
    };
    (elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional, $case:ident) => {
        pub const CONFIG: [usize; 10] = [8, 1, 1, 1, 0, 1, 1, 1, 1, 1];
        pipeline_configuration! { $case, golden_scenarios; planner: [elimination, fine];
        merge: [cq_rewrites::merge_joins,];
        canon: [cq_rewrites::canon_prefixes,];
        intersection: [cq_rewrites::smallest_intersections,];
        host: [take] slice [positional];
        dedup: [cq_workload_passes::ordered_dedup,];
        payload: [cq_workload_passes::sorted_payload,];
        membership: [cq_rewrites::lazy_semijoins,];
        print_input: structures;
        printer: own_push::push; }
    };
}

macro_rules! configurations {
    ($case:ident; $($configuration:ident),+ $(,)?) => {
        extend_configurations!($case; $($configuration),+);
    };
}

macro_rules! case_configurations {
    (intersection, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        hypergraph_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
    ); };
    (oriented_chain, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_probe_keep_first_hashmap_ordered_dedup_sorted_lazy_push_unadvised,
        prefix_connected_coarse_merge_canon_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_smallest_slice_hash_dedup_default_eager_push_positional,
        prefix_greedy_coarse_probe_canon_first_btreemap_hash_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_btreemap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_canon_smallest_hashmap_hash_dedup_sorted_eager_pull_unadvised,
        hypergraph_merge_canon_first_slice_hash_dedup_default_eager_pull_positional,
        written_fine_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_connected_fine_merge_canon_first_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_lazy_push_positional,
        elimination_coarse_merge_canon_smallest_slice_hash_dedup_sorted_eager_pull_positional,
        elimination_fine_merge_keep_first_slice_ordered_dedup_default_lazy_push_positional,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_default_eager_push_positional,
        written_coarse_probe_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        written_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        hypergraph_probe_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        written_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        written_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        elimination_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
    ); };
    (claw, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_probe_keep_first_hashmap_ordered_dedup_sorted_lazy_push_unadvised,
        prefix_connected_coarse_merge_canon_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_smallest_slice_hash_dedup_default_eager_push_positional,
        prefix_greedy_coarse_probe_canon_first_btreemap_hash_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_btreemap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_canon_smallest_hashmap_hash_dedup_sorted_eager_pull_unadvised,
        hypergraph_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        written_fine_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_connected_fine_merge_canon_first_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_lazy_push_positional,
        elimination_fine_merge_keep_first_slice_ordered_dedup_default_lazy_push_positional,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_default_eager_push_positional,
        elimination_coarse_probe_canon_smallest_slice_hash_dedup_sorted_eager_pull_positional,
        written_coarse_probe_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        written_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        hypergraph_probe_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        written_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        written_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
    ); };
    (fact_star, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised,
        elimination_coarse_merge_keep_smallest_btreemap_hash_dedup_sorted_lazy_push_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (triangle, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_first_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_probe_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_connected_coarse_merge_canon_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_first_btreemap_hash_dedup_default_lazy_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_hash_dedup_sorted_eager_pull_positional,
        prefix_greedy_fine_probe_canon_first_hashmap_ordered_dedup_default_eager_push_unadvised,
        elimination_coarse_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        elimination_fine_merge_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        written_fine_probe_keep_smallest_hashmap_hash_dedup_default_eager_pull_unadvised,
        hypergraph_probe_canon_first_slice_hash_dedup_default_eager_push_positional,
        prefix_connected_coarse_probe_keep_first_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_eager_pull_positional,
        prefix_greedy_fine_merge_keep_smallest_slice_hash_dedup_sorted_lazy_pull_positional,
        prefix_greedy_coarse_probe_canon_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        elimination_coarse_probe_canon_first_slice_hash_dedup_default_lazy_push_positional,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_lazy_push_positional,
        written_coarse_merge_canon_smallest_btreemap_ordered_dedup_sorted_lazy_pull_unadvised,
        written_coarse_probe_keep_first_slice_hash_dedup_default_eager_push_positional,
        hypergraph_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_smallest_hashmap_hash_dedup_default_eager_pull_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        written_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        written_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
    ); };
    (cycle_4, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
    ); };
    (spade, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_merge_keep_smallest_btreemap_hash_dedup_sorted_lazy_push_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
    ); };
    (barbell, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
    ); };
    (loomis_whitney_4, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
    ); };
    (retail_h03, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_probe_keep_first_hashmap_ordered_dedup_sorted_lazy_push_unadvised,
        prefix_connected_coarse_merge_canon_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_smallest_slice_hash_dedup_default_eager_push_positional,
        prefix_greedy_coarse_probe_canon_first_btreemap_hash_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_btreemap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_canon_smallest_hashmap_hash_dedup_sorted_eager_pull_unadvised,
        hypergraph_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        written_fine_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_connected_fine_merge_canon_first_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_lazy_push_positional,
        elimination_fine_merge_keep_first_slice_ordered_dedup_default_lazy_push_positional,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_default_eager_push_positional,
        elimination_coarse_probe_canon_smallest_slice_hash_dedup_sorted_eager_pull_positional,
        written_coarse_probe_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        written_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        hypergraph_probe_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        written_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        written_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_fine_probe_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
    ); };
    (retail_h05, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (retail_h07, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_probe_keep_first_hashmap_ordered_dedup_sorted_lazy_push_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised,
        elimination_coarse_merge_keep_smallest_btreemap_hash_dedup_sorted_lazy_push_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (retail_h08, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised,
        elimination_coarse_merge_keep_smallest_btreemap_hash_dedup_sorted_lazy_push_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (retail_h09, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        elimination_coarse_merge_keep_smallest_btreemap_hash_dedup_sorted_lazy_push_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (retail_h21_positive, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_probe_keep_first_hashmap_ordered_dedup_sorted_lazy_push_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised,
        elimination_coarse_merge_keep_smallest_btreemap_hash_dedup_sorted_lazy_push_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (retail_d19, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (retail_d27, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (retail_d72_inner, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_merge_keep_smallest_btreemap_hash_dedup_sorted_lazy_push_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (retail_d85, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_probe_keep_first_hashmap_ordered_dedup_sorted_lazy_push_unadvised,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        written_coarse_merge_canon_first_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        hypergraph_merge_keep_first_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
    (cartesian_product, $case:ident) => { configurations!($case;
        written_coarse_probe_keep_smallest_slice_hash_dedup_default_eager_pull_unadvised,
        written_coarse_merge_canon_first_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        written_coarse_probe_canon_smallest_btreemap_hash_dedup_sorted_eager_push_unadvised,
        written_fine_probe_canon_first_slice_hash_dedup_sorted_lazy_push_unadvised,
        written_fine_merge_keep_smallest_hashmap_ordered_dedup_default_eager_pull_unadvised,
        written_fine_merge_keep_smallest_btreemap_hash_dedup_default_lazy_pull_unadvised,
        written_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        hypergraph_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        hypergraph_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        hypergraph_merge_keep_first_btreemap_ordered_dedup_sorted_eager_pull_unadvised,
        hypergraph_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_connected_coarse_probe_canon_first_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_connected_fine_probe_canon_first_slice_hash_dedup_default_eager_pull_unadvised,
        prefix_connected_fine_merge_keep_smallest_hashmap_ordered_dedup_sorted_lazy_pull_unadvised,
        prefix_connected_fine_probe_canon_smallest_btreemap_hash_dedup_default_eager_push_unadvised,
        prefix_connected_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        prefix_greedy_coarse_probe_keep_smallest_slice_ordered_dedup_sorted_lazy_push_unadvised,
        prefix_greedy_coarse_probe_canon_smallest_btreemap_ordered_dedup_default_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_hash_dedup_sorted_eager_push_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_ordered_dedup_default_lazy_push_unadvised,
        prefix_greedy_fine_merge_keep_smallest_btreemap_hash_dedup_default_eager_pull_unadvised,
        prefix_greedy_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
        elimination_coarse_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_hash_dedup_default_eager_push_unadvised,
        elimination_coarse_merge_keep_smallest_btreemap_hash_dedup_sorted_lazy_push_unadvised,
        elimination_coarse_probe_keep_first_slice_hash_dedup_default_eager_pull_positional,
        elimination_fine_probe_canon_first_slice_ordered_dedup_sorted_eager_pull_unadvised,
        elimination_fine_merge_keep_smallest_hashmap_hash_dedup_default_lazy_pull_unadvised,
        elimination_fine_probe_keep_smallest_btreemap_ordered_dedup_default_eager_push_unadvised,
        elimination_fine_merge_canon_smallest_slice_ordered_dedup_sorted_lazy_push_positional,
    ); };
}
