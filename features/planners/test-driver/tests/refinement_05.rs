//! One ordinal-modulo-eight partition of the compiled refinement corpus.
#[path = "support/refinement.rs"]
mod support;
use support::{ExecutableCase, integer};

include!(concat!(env!("OUT_DIR"), "/refinement_05.rs"));

#[test]
fn every_planner_refines_25_rust_seeds_in_compiled_rust() {
    support::check_shard(EXECUTABLES, 5);
}
