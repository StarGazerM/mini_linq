//! Opt-in fuzz tier. Ordinary `cargo test` never compiles generated consumers.
//! Run with `--test random_programs random_programs_quick -- --ignored --nocapture`.
#[path = "../src/random_programs.rs"]
mod random_programs;

#[test]
#[ignore = "fuzz tier: explicitly opt in to generated consumer compilation"]
fn random_programs_quick() {
    let start = std::time::Instant::now();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    random_programs::run_quick(&root, 0);
    eprintln!("D1 quick: 16 programs, 2 routes, {:?}", start.elapsed());
}

#[test]
#[ignore = "slow tier: rotating program syntax and all CPU engines"]
fn random_programs_full() {
    let seed = std::env::var("CQ_FUZZ_SEED")
        .map(|s| s.parse().unwrap())
        .unwrap_or_else(|_| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        });
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    // Bounded batch sizes avoid a monolithic generated consumer.
    for batch in 0..16 {
        random_programs::run_cpu(&root, seed + batch * 16, 16);
    }
}
