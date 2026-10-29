use std::path::PathBuf;
use std::process::Command;

fn main() {
    let src = "src/kernels.metal";
    println!("cargo:rerun-if-changed={src}");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let air = out.join("kernels.air");
    let lib = out.join("kernels.metallib");
    let st = Command::new("xcrun")
        .args(["-sdk", "macosx", "metal", "-O3", "-c", src, "-o"])
        .arg(&air)
        .status()
        .expect("xcrun metal not found");
    assert!(st.success(), "metal compile failed");
    let st = Command::new("xcrun")
        .args(["-sdk", "macosx", "metallib"])
        .arg(&air)
        .arg("-o")
        .arg(&lib)
        .status()
        .expect("xcrun metallib not found");
    assert!(st.success(), "metallib failed");
    println!("cargo:rustc-env=KERNELS_METALLIB={}", lib.display());
}
