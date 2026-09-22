//! Run with: cargo run -p mini-linq --example rust-expressions --features compiled-workloads
use ::mini_linq;
use mini_linq::mini_linq as query;

mod gallery;
mod lazy;

fn main() {
    println!("Rust expressions through the default lazy compiler:");
    gallery::run();
    lazy::run();
}
