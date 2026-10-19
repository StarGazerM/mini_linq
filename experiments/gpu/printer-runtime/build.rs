use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/nvtx.c");
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CC");
    println!("cargo:rerun-if-env-changed=AR");
    if env::var_os("CARGO_FEATURE_NVTX").is_none() {
        return;
    }
    let cuda =
        PathBuf::from(env::var_os("CUDA_HOME").expect("nvtx requires CUDA_HOME (NVTX3 headers)"));
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("nvtx.o");
    assert!(
        Command::new(env::var_os("CC").unwrap_or_else(|| "cc".into()))
            .arg("-O2")
            .arg("-fPIC")
            .arg("-I")
            .arg(cuda.join("include"))
            .args(["-c", "src/nvtx.c", "-o"])
            .arg(&object)
            .status()
            .expect("compile NVTX3 bridge")
            .success()
    );
    assert!(
        Command::new(env::var_os("AR").unwrap_or_else(|| "ar".into()))
            .arg("crs")
            .arg(out.join("libcq_nvtx.a"))
            .arg(object)
            .status()
            .expect("archive NVTX3 bridge")
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=cq_nvtx");
    println!("cargo:rustc-link-lib=dl");
}
