//! Run: cargo run -p mini-linq --example blocked-ya --locked
use cq_storage::BlockStore;
use std::collections::{BTreeSet, HashSet};
use std::num::NonZeroUsize;

// A facade composes providers per binding and access implementations per operation.
// S and T remain native containers; only R offers block descriptors and bounds.
macro_rules! configured {
    ($scan:path, $key_provider:path; $($ra:tt)*) => {
        ::mini_linq::physical_plan! {
            storage ::cq_storage;
            access ::mini_linq::collect_access;
            iterator ::mini_linq::stream_access;
            materialize ::mini_linq::hash_rows;
            inputs {
                R => ::mini_linq::block_rows;
                S => ::mini_linq::hash_rows;
                T => ::mini_linq::sorted_rows;
            }
            results {}
            scan ::mini_linq::scan_rows;
            scans { r1 => $scan; }
            key_provider ::mini_linq::exact_keys;
            key_providers { r1 => $key_provider; }
            keys ::std::collections::HashSet;
            key_sets { r1 => ::std::collections::BTreeSet; }
            join ::mini_linq::hash_witnesses;
            joins {}
            $($ra)*
        }
    };
}
macro_rules! exact_rows {
    ($($ra:tt)*) => { configured!(::mini_linq::scan_rows, ::mini_linq::exact_keys; $($ra)*); };
}
macro_rules! zone_rows {
    ($($ra:tt)*) => { configured!(::mini_linq::prune_rows, ::mini_linq::zone_keys; $($ra)*); };
}
pub(crate) use {exact_rows, zone_rows};
mod exact_wiring {
    pub(crate) use super::exact_rows as planned_relational_plan;
    pub use ::mini_linq::*;
}
mod zone_wiring {
    pub(crate) use super::zone_rows as planned_relational_plan;
    pub use ::mini_linq::*;
}

macro_rules! chain {
    () => {
        mini_linq::planned_relational_plan! {
            pub struct Query;
            relation R(k:i64, x:i64);
            relation S(z:i64, k:i64);
            relation T(y:i64, z:i64);
            answer(x,y) :- R(k,x), S(z,k), T(y,z);
            relational {
                r = rename R {k -> k,x -> x};
                s = rename S {z -> z,k -> k};
                t = rename T {y -> y,z -> z};
                s1 = semijoin s with t;
                r1 = semijoin r with s1;
                s2 = semijoin s1 with r1;
                t1 = semijoin t with s2;
                rs = natural_join r1 with s2;
                live = project rs keep {x,z};
                joined = natural_join live with t1;
                result = project joined keep {x,y};
                output result as answer(x,y);
            }
        }
    };
}
mod exact {
    use crate::exact_wiring as mini_linq;
    chain!();
}
mod zone {
    use crate::zone_wiring as mini_linq;
    chain!();
}

fn main() {
    let block_rows = NonZeroUsize::new(1).unwrap();
    // Only this constructor establishes block layout and zone metadata.
    let r = BlockStore::from_rows([[1, 10], [2, 20], [50, 30], [60, 40]], block_rows);
    let s = HashSet::from([[100, 1], [200, 2], [900, 90]]);
    let t = BTreeSet::from([[7, 100]]);
    let (answer, selected) = zone::Query::run_with_stats(&r, &s, &t, block_rows);
    let (reference, all) = exact::Query::run_with_stats(&r, &s, &t, block_rows);
    assert_eq!(answer, vec![(10, 7)]);
    assert_eq!(answer, reference);
    println!("R: BlockStore; S: HashSet; T: BTreeSet; materialized results: HashSet");
    println!("r1: zone guard then exact membership using BTreeSet keys");
    println!("answer: {answer:?}");
    println!("result   rows   index rows   visited rows   skipped blocks");
    for (name, rows, work) in &selected.steps {
        println!(
            "{name:>6} {rows:>6} {:>12} {:>14} {:>16}",
            work.index_rows, work.rows_visited, work.blocks_skipped
        );
    }
    let zone_visits = selected
        .steps
        .iter()
        .map(|s| s.2.rows_visited)
        .sum::<usize>()
        + selected.output.rows_visited;
    let exact_visits =
        all.steps.iter().map(|s| s.2.rows_visited).sum::<usize>() + all.output.rows_visited;
    println!("query row visits: zone={zone_visits} exact={exact_visits}");
    assert!(zone_visits < exact_visits);
    assert_eq!(selected.output.blocks_opened, 0);
    println!("Counters measure in-memory payload access; ingestion and disk I/O are not included.");
}
