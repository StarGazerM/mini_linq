//! Compile and execute generated programs against a caller-selected storage source.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub struct Runtime {
    directory: PathBuf,
}
impl Runtime {
    pub fn new(source: &Path, counters: bool) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("cq-generated-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
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
                .arg(&directory)
                .output()
                .unwrap(),
        );
        Self { directory }
    }

    pub fn compile(&self, source: &str) -> Output {
        let path = self.directory.join("main.rs");
        std::fs::write(&path, source).unwrap();
        crate::runtime_rustc()
            .arg("--edition=2024")
            .arg(&path)
            .arg("--extern")
            .arg(format!(
                "cq_storage={}",
                self.directory.join("libcq_storage.rlib").display()
            ))
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
