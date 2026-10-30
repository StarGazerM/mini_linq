//! CQ negation cores use the established retail pairwise matrix and DuckDB goldens.
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../matrix.rs"]
mod matrix;

// Full-key intersections expose leading-key merge reads: HashMap lacks
// OrderedRead, and own_push has no merge. Anti-only cores below retain the
// complete pairwise list, so combined pair coverage remains unchanged.
macro_rules! intersection_configurations {
    ($case:ident) => { configurations!($case;
        written_coarse_probe_keep_first_slice_default_eager_pull_unadvised,
        hypergraph_probe_keep_first_hashmap_sorted_lazy_push_unadvised,
        prefix_connected_coarse_merge_canon_smallest_btreemap_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_smallest_slice_default_eager_push_positional,
        prefix_greedy_coarse_probe_canon_first_btreemap_sorted_lazy_pull_unadvised,
        elimination_coarse_probe_keep_first_btreemap_default_lazy_push_unadvised,
        elimination_fine_probe_canon_smallest_hashmap_sorted_eager_pull_unadvised,
        hypergraph_merge_canon_first_slice_default_eager_pull_positional,
        written_fine_probe_keep_first_hashmap_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_slice_sorted_lazy_push_positional,
        prefix_greedy_fine_probe_canon_smallest_slice_sorted_lazy_push_positional,
        prefix_greedy_coarse_probe_keep_smallest_slice_default_eager_push_positional,
        elimination_coarse_probe_canon_smallest_slice_sorted_eager_pull_positional,
        written_coarse_probe_canon_smallest_slice_sorted_lazy_push_positional,
        written_coarse_merge_keep_first_btreemap_default_eager_pull_unadvised,
        hypergraph_probe_keep_smallest_btreemap_default_eager_pull_unadvised,
        written_coarse_probe_keep_first_hashmap_default_eager_pull_unadvised,
        written_fine_probe_keep_first_btreemap_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_hashmap_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_first_btreemap_default_eager_pull_unadvised,
        prefix_greedy_coarse_probe_keep_first_hashmap_default_eager_pull_unadvised,
        prefix_greedy_coarse_merge_keep_first_btreemap_default_eager_pull_unadvised,
        prefix_greedy_fine_probe_keep_first_btreemap_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_default_eager_pull_unadvised,
        elimination_coarse_merge_keep_first_btreemap_default_eager_pull_unadvised,
        elimination_fine_probe_keep_first_btreemap_default_eager_pull_unadvised,
    ); };
}

// These anti-only bodies share H03's historical witnesses, not its checked
// positive join plan. Keep their admissible rows independent of catalog H03.
macro_rules! anti_configurations {
    ($case:ident) => { configurations!($case;
        written_coarse_probe_keep_first_slice_default_eager_pull_unadvised,
        written_fine_merge_canon_smallest_slice_sorted_lazy_push_positional,
        hypergraph_probe_keep_first_hashmap_sorted_lazy_push_unadvised,
        prefix_connected_coarse_merge_canon_smallest_btreemap_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_smallest_slice_default_eager_push_positional,
        prefix_greedy_coarse_probe_canon_first_btreemap_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_merge_keep_first_hashmap_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_btreemap_default_lazy_push_unadvised,
        elimination_fine_probe_canon_smallest_hashmap_sorted_eager_pull_unadvised,
        hypergraph_merge_canon_first_slice_default_eager_pull_positional,
        written_fine_probe_keep_first_hashmap_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_slice_sorted_lazy_push_positional,
        prefix_connected_fine_merge_canon_first_hashmap_sorted_lazy_pull_unadvised,
        prefix_greedy_fine_probe_canon_smallest_slice_sorted_lazy_push_positional,
        elimination_fine_merge_keep_first_slice_default_lazy_push_positional,
        prefix_greedy_coarse_probe_keep_smallest_slice_default_eager_push_positional,
        elimination_coarse_probe_canon_smallest_slice_sorted_eager_pull_positional,
        written_coarse_probe_canon_smallest_slice_sorted_lazy_push_positional,
        written_coarse_merge_keep_first_btreemap_default_eager_pull_unadvised,
        hypergraph_probe_keep_smallest_btreemap_default_eager_pull_unadvised,
        written_coarse_probe_keep_first_hashmap_default_eager_pull_unadvised,
        written_fine_probe_keep_first_btreemap_default_eager_pull_unadvised,
        prefix_connected_coarse_probe_keep_first_hashmap_default_eager_pull_unadvised,
        prefix_connected_fine_probe_keep_first_btreemap_default_eager_pull_unadvised,
        prefix_greedy_coarse_probe_keep_first_hashmap_default_eager_pull_unadvised,
        prefix_greedy_coarse_merge_keep_first_btreemap_default_eager_pull_unadvised,
        prefix_greedy_fine_probe_keep_first_btreemap_default_eager_pull_unadvised,
        elimination_coarse_probe_keep_first_hashmap_default_eager_pull_unadvised,
        elimination_coarse_merge_keep_first_btreemap_default_eager_pull_unadvised,
        elimination_fine_probe_keep_first_btreemap_default_eager_pull_unadvised,
    ); };
}

mod retail_h04 {
    macro_rules! run_retail_h03 {
        (configuration: $configuration:ident;
         host: [$load:ident] $view:ident [$($positional:ident)?];
         pipeline: {$($pipeline:tt)*}) => {
            #[test]
            #[allow(non_snake_case)]
            fn $configuration() {
                let case = cq_workloads::extended::case("retail_h04").unwrap();
                for scenario in cq_workloads::Scenario::ALL {
                    let mut inputs = case.inputs(scenario);
                    let Orders = host_view!($view, inputs.$load::<(i32,)>("Orders"));
                    let LateLineitem = host_view!($view, inputs.$load::<(i32,)>("LateLineitem"));
                    let answers = mini_linq::compile! {
                        pipeline {
                            $($pipeline)*
                            select = mini_linq::passes::select($($positional(Orders, LateLineitem))?);
                        }
                        struct Query;
                        relation Orders(order_key: i32);
                        relation LateLineitem(order_key: i32);
                        answer(order_key) :- Orders(order_key), LateLineitem(order_key);
                    }.map(|row| vec![*row.0]).collect();
                    case.check(scenario, answers);
                }
            }
        };
    }
    intersection_configurations!(run_retail_h03);
}

mod retail_h04_not_exists {
    macro_rules! run_retail_h03 {
        (configuration: $configuration:ident;
         host: [$load:ident] $view:ident [$($positional:ident)?];
         pipeline: {$($pipeline:tt)*}) => {
            #[test]
            #[allow(non_snake_case)]
            fn $configuration() {
                let case = cq_workloads::extended::case("retail_h04_not_exists").unwrap();
                for scenario in cq_workloads::Scenario::ALL {
                    let mut inputs = case.inputs(scenario);
                    let Orders = host_view!($view, inputs.$load::<(i32,)>("Orders"));
                    let LateLineitem = host_view!($view, inputs.$load::<(i32,)>("LateLineitem"));
                    let answers = mini_linq::compile! {
                        pipeline {
                            $($pipeline)*
                            select = mini_linq::passes::select($($positional(Orders, LateLineitem))?);
                        }
                        struct Query;
                        relation Orders(order_key: i32);
                        relation LateLineitem(order_key: i32);
                        answer(order_key) :- Orders(order_key), !LateLineitem(order_key);
                    }.map(|row| vec![*row.0]).collect();
                    case.check(scenario, answers);
                }
            }
        };
    }
    anti_configurations!(run_retail_h03);
}

mod retail_h16 {
    macro_rules! run_retail_h03 {
        (configuration: $configuration:ident;
         host: [$load:ident] $view:ident [$($positional:ident)?];
         pipeline: {$($pipeline:tt)*}) => {
            #[test]
            #[allow(non_snake_case)]
            fn $configuration() {
                let case = cq_workloads::extended::case("retail_h16").unwrap();
                for scenario in cq_workloads::Scenario::ALL {
                    let mut inputs = case.inputs(scenario);
                    let Part = host_view!($view, inputs.$load::<(i32,)>("Part"));
                    let Partsupp = host_view!($view, inputs.$load::<(i32, i32,)>("Partsupp"));
                    let BadSupplier = host_view!($view, inputs.$load::<(i32,)>("BadSupplier"));
                    let answers = mini_linq::compile! {
                        pipeline {
                            $($pipeline)*
                            select = mini_linq::passes::select($($positional(Part, Partsupp, BadSupplier))?);
                        }
                        struct Query;
                        relation Part(part_key: i32);
                        relation Partsupp(part_key: i32, suppkey: i32);
                        relation BadSupplier(suppkey: i32);
                        answer(part_key, suppkey) :- Part(part_key), Partsupp(part_key, suppkey), !BadSupplier(suppkey);
                    }.map(|row| vec![*row.0, *row.1]).collect();
                    case.check(scenario, answers);
                }
            }
        };
    }
    intersection_configurations!(run_retail_h03);
}

mod retail_h22 {
    macro_rules! run_retail_h03 {
        (configuration: $configuration:ident;
         host: [$load:ident] $view:ident [$($positional:ident)?];
         pipeline: {$($pipeline:tt)*}) => {
            #[test]
            #[allow(non_snake_case)]
            fn $configuration() {
                let case = cq_workloads::extended::case("retail_h22").unwrap();
                for scenario in cq_workloads::Scenario::ALL {
                    let mut inputs = case.inputs(scenario);
                    let Customer = host_view!($view, inputs.$load::<(i32,)>("Customer"));
                    let Orders = host_view!($view, inputs.$load::<(i32, i32,)>("Orders"));
                    let answers = mini_linq::compile! {
                        pipeline {
                            $($pipeline)*
                            select = mini_linq::passes::select($($positional(Customer, Orders))?);
                        }
                        struct Query;
                        relation Customer(custkey: i32);
                        relation Orders(order_key: i32, custkey: i32);
                        answer(custkey) :- Customer(custkey), !Orders(_, custkey);
                    }.map(|row| vec![*row.0]).collect();
                    case.check(scenario, answers);
                }
            }
        };
    }
    anti_configurations!(run_retail_h03);
}

#[test]
fn every_axis_value_and_pair_is_exercised() {
    let rows: Vec<_> = [
        retail_h04::CONFIGURATIONS,
        retail_h04_not_exists::CONFIGURATIONS,
        retail_h16::CONFIGURATIONS,
        retail_h22::CONFIGURATIONS,
    ]
    .into_iter()
    .flat_map(|rows| rows.iter().copied())
    .collect();
    matrix::verify_cover(&rows);
}
