//! Run with: cargo run -p mini-linq --example rust-expressions-planned --features compiled-workloads
use mini_linq::planned_query as query;

mod gallery;
mod plans;

fn main() {
    println!("Rust expressions through the planned materialized compiler:");
    gallery::run();
    plans::run();
}
