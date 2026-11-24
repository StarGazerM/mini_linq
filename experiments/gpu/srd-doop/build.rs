use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/advice_cost.dl");
    if env::var_os("CARGO_FEATURE_ADVICE_COST").is_none() {
        return;
    }
    let advice = fs::read_to_string("src/advice_cost.dl")
        .expect("generate src/advice_cost.dl with DOOP_ORACLE_ADVICE before building advice-cost");
    assert!(!advice.trim().is_empty(), "cost-oracle advice must not be empty");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    fs::write(output.join("advice_cost.rs"), format!("engine! {{\n{advice}\n}}\n"))
        .expect("write verbatim cost-oracle engine invocation");
}
