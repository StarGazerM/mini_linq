//! Compile and execute complete native Rust programs for boundary tests.
pub(crate) fn run_rust(tokens: proc_macro2::TokenStream) {
    check_rust(tokens, None);
}

pub(crate) fn check_rust(tokens: proc_macro2::TokenStream, expected_error: Option<&str>) {
    use std::{
        fs,
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mini-linq-generated-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.rs");
    fs::write(&file, prettyplease::unparse(&syn::parse2(tokens).unwrap())).unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let built = Command::new(rustc)
        .arg("--edition=2024")
        .arg("-Awarnings")
        .arg(&file)
        .arg("-o")
        .arg(dir.join("run"))
        .output()
        .unwrap();
    if let Some(message) = expected_error {
        assert!(
            !built.status.success() && String::from_utf8_lossy(&built.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        fs::remove_dir_all(dir).unwrap();
        return;
    }
    assert!(
        built.status.success(),
        "{}\nsource: {}",
        String::from_utf8_lossy(&built.stderr),
        file.display()
    );
    let run = Command::new(dir.join("run")).output().unwrap();
    assert!(
        run.status.success(),
        "{}\nsource: {}",
        String::from_utf8_lossy(&run.stderr),
        file.display()
    );
    fs::remove_dir_all(dir).unwrap();
}
