//! Compile and execute generated programs against a caller-selected storage source.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Locate and memoize the Cargo artifact with the exact type identity linked into this test.
/// Callers with custom providers must pass both their provider crate and its
/// storage crate to `Runtime::with_dependencies`.
pub fn linked_dependency<T: 'static>(name: &str, type_name: &str) -> (String, PathBuf) {
    type Key = (PathBuf, String, String, String);
    static ARTIFACTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<Key, PathBuf>>,
    > = std::sync::OnceLock::new();

    let executable = std::env::current_exe().expect("test executable path");
    let dependencies = executable.parent().expect("Cargo dependency directory");
    let key = (
        dependencies.to_owned(),
        name.to_owned(),
        type_name.to_owned(),
        format!("{:?}", std::any::TypeId::of::<T>()),
    );
    let mut artifacts = ARTIFACTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(artifact) = artifacts.get(&key) {
        return (name.to_owned(), artifact.clone());
    }
    let selected = crate::linked_runtime_backend(dependencies, name, type_name, &key.3);
    let artifact = match selected {
        Ok(artifact) => artifact,
        Err(error) => {
            drop(artifacts);
            panic!("{error}");
        }
    };
    artifacts.insert(key, artifact.clone());
    (name.to_owned(), artifact)
}

pub struct Runtime {
    directory: PathBuf,
    dependencies: Vec<(String, PathBuf)>,
}
impl Runtime {
    /// Compile storage once per source path and counters setting in this process.
    /// Cached artifacts live outside each runtime's independent query directory.
    pub fn new(source: &Path, counters: bool) -> Self {
        type Key = (PathBuf, bool);
        static STORAGE: std::sync::LazyLock<
            std::sync::Mutex<std::collections::HashMap<Key, PathBuf>>,
        > = std::sync::LazyLock::new(Default::default);

        let key = (
            source.canonicalize().expect("storage source path"),
            counters,
        );
        let mut artifacts = STORAGE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let artifact = artifacts
            .entry(key)
            .or_insert_with_key(|(source, counters)| {
                let directory =
                    std::env::temp_dir().join(format!("cq-storage-{}", std::process::id()));
                std::fs::create_dir_all(&directory).unwrap();
                static NEXT: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(0);
                let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let artifact = directory.join(format!("libcq_storage_{id}.rlib"));
                let mut compiler = crate::runtime_rustc();
                compiler.args([
                    "--edition=2024",
                    "--crate-name",
                    "cq_storage",
                    "--crate-type",
                    "rlib",
                ]);
                if *counters {
                    compiler.args(["--cfg", "feature=\"counters\""]);
                }
                success(
                    &compiler
                        .arg(source)
                        .arg("-o")
                        .arg(&artifact)
                        .output()
                        .unwrap(),
                );
                artifact
            })
            .clone();
        drop(artifacts);
        Self::with_dependencies(vec![("cq_storage".into(), artifact)])
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
        self.compile_with_dependencies(source, &[])
    }

    /// Link target-owned runtime adapters in addition to this fixture's providers.
    pub fn compile_with_dependencies(
        &self,
        source: &str,
        dependencies: &[(String, PathBuf)],
    ) -> Output {
        let path = self.directory.join("main.rs");
        std::fs::write(&path, source).unwrap();
        let mut compiler = crate::runtime_rustc();
        for (name, artifact) in self.dependencies.iter().chain(dependencies) {
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
        self.run_with_dependencies(source, &[])
    }

    pub fn run_with_dependencies(
        &self,
        source: &str,
        dependencies: &[(String, PathBuf)],
    ) -> Output {
        success(&self.compile_with_dependencies(source, dependencies));
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

#[cfg(test)]
mod tests {
    use super::Runtime;

    #[test]
    fn storage_is_cached_by_source_and_counters_without_sharing_query_directories() {
        let fixture = Runtime::with_dependencies(Vec::new());
        let source = fixture.directory.join("storage.rs");
        std::fs::write(
            &source,
            "pub fn value() -> bool { cfg!(feature = \"counters\") }",
        )
        .unwrap();
        let first = Runtime::new(&source, false);
        let second = Runtime::new(&source, false);
        let counters = Runtime::new(&source, true);
        let other_source = fixture.directory.join("other.rs");
        std::fs::copy(&source, &other_source).unwrap();
        let other = Runtime::new(&other_source, false);
        assert_eq!(first.dependencies, second.dependencies);
        assert_ne!(first.dependencies, counters.dependencies);
        assert_ne!(first.dependencies, other.dependencies);
        assert_ne!(first.directory, second.directory);
        let first_directory = first.directory.clone();
        drop(first);
        assert!(!first_directory.exists());
        assert!(second.dependencies[0].1.exists());
        second.run("fn main() { assert!(!cq_storage::value()); }");
        counters.run("fn main() { assert!(cq_storage::value()); }");
    }
}
