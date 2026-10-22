//! Compile and execute generated programs against a caller-selected storage source.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Locate the Cargo artifact with the exact type identity linked into this test.
/// Callers with custom providers must pass both their provider crate and its
/// storage crate to `Runtime::with_dependencies`.
pub fn linked_dependency<T: 'static>(name: &str, type_name: &str) -> (String, PathBuf) {
    let executable = std::env::current_exe().expect("test executable path");
    let artifact = crate::linked_runtime_backend(
        executable.parent().expect("Cargo dependency directory"),
        name,
        type_name,
        &format!("{:?}", std::any::TypeId::of::<T>()),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    (name.to_owned(), artifact)
}

pub struct Runtime {
    directory: PathBuf,
    dependencies: Vec<(String, PathBuf)>,
}
impl Runtime {
    pub fn new(source: &Path, counters: bool) -> Self {
        let mut runtime = Self::with_dependencies(Vec::new());
        let mut compiler = crate::runtime_rustc();
        compiler.args([
            "--edition=2024",
            "--crate-name",
            "cq_storage",
            "--crate-type",
            "rlib",
        ]);
        if counters {
            compiler.args(["--cfg", "feature=\"counters\""]);
        }
        success(
            &compiler
                .arg(source)
                .arg("--out-dir")
                .arg(&runtime.directory)
                .output()
                .unwrap(),
        );
        runtime.dependencies.push((
            "cq_storage".into(),
            runtime.directory.join("libcq_storage.rlib"),
        ));
        runtime
    }

    /// Use already-linked Cargo artifacts when fixture-specific providers
    /// implement traits from the caller's storage crate. All supplied artifacts
    /// must belong to the same dependency graph.
    pub fn with_dependencies(dependencies: Vec<(String, PathBuf)>) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("cq-generated-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        Self {
            directory,
            dependencies,
        }
    }

    pub fn compile(&self, source: &str) -> Output {
        let path = self.directory.join("main.rs");
        std::fs::write(&path, source).unwrap();
        let mut compiler = crate::runtime_rustc();
        for (name, artifact) in &self.dependencies {
            compiler
                .arg("--extern")
                .arg(format!("{name}={}", artifact.display()));
        }
        compiler
            .arg("--edition=2024")
            .arg(&path)
            .arg("-o")
            .arg(self.directory.join("run"))
            .output()
            .unwrap()
    }

    pub fn run(&self, source: &str) -> Output {
        success(&self.compile(source));
        let output = Command::new(self.directory.join("run")).output().unwrap();
        success(&output);
        output
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
