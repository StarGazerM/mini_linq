//! Token-only adapters shared by the visible cases and configuration tables.
#[macro_use]
mod le_configurations;

macro_rules! host_view {
    (slice, $rows:expr) => {
        &$rows[..]
    };
    (map, $rows:expr) => {
        &$rows
    };
}

// Resolve a named configuration to one pipeline block and one host loader.
// No query tokens are inspected or regenerated here.
macro_rules! pipeline_configuration {
    ($case:ident, $name:ident;
     planner: [le_runs];
     merge: $merge:tt; canon: $canon:tt;
     intersection: $intersection:tt;
     host: [$load:ident] $view:ident [$($positional:ident)?];
     dedup: $dedup:tt; payload: [$($payload:tt)*];
     membership: $membership:tt; printer: $printer:path;) => {
        pipeline_configuration! { $case, $name;
            planner: [le];
            merge: $merge; canon: $canon;
            intersection: $intersection;
            host: [$load] $view [$($positional)?];
            dedup: $dedup; payload: [cq_rewrites::sorted_runs, $($payload)*];
            membership: $membership; printer: $printer;
        }
    };
    ($case:ident, $name:ident;
     planner: [$($plan:tt)*];
     merge: [$($merge:tt)*]; canon: [$($canon:tt)*];
     intersection: [$($intersection:tt)*];
     host: [$load:ident] $view:ident [$($positional:ident)?];
     dedup: [$($dedup:tt)*]; payload: [$($payload:tt)*];
     membership: [$($membership:tt)*]; printer: $printer:path;) => {
        $case! {
            configuration: $name;
            host: [$load] $view [$($positional)?];
            pipeline: {
                plan = cq_planners::order($($plan)*);
                physical = [$($canon)* $($intersection)* $($membership)* $($merge)*];
                structures = [$($dedup)* $($payload)*];
                print = $printer;
            }
        }
    };
}
