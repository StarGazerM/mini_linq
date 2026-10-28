//! Child Cargo must not inherit the enclosing `cargo run` package identity.
//! Some native build scripts track these variables, which otherwise invalidates
//! their fingerprints whenever users switch between `cargo xtask` and tests.
pub fn command() -> std::process::Command {
    let mut command =
        std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    remove_package_identity(&mut command, std::env::vars_os().map(|(name, _)| name));
    command
}

pub(super) const BENCH_RELEASE_CHILD: &str = "CQ_XTASK_BENCH_RELEASE_CHILD";

/// Always request the release profile explicitly. Checking debug_assertions
/// alone would incorrectly accept a development profile that disables them.
pub(super) fn release_benchmark(arguments: &[String], oracle: bool) -> std::process::Command {
    let mut command = command();
    command
        .arg("run")
        .arg("--manifest-path")
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .arg("--locked")
        .arg("--release");
    if oracle {
        command.arg("--features").arg("oracle");
    }
    command
        .arg("--")
        .arg("bench")
        .args(arguments)
        .env(BENCH_RELEASE_CHILD, "1");
    command
}

fn remove_package_identity(
    command: &mut std::process::Command,
    names: impl IntoIterator<Item = std::ffi::OsString>,
) {
    for name in names {
        if name.to_str().is_some_and(|name| {
            name.starts_with("CARGO_PKG_")
                || matches!(
                    name,
                    "CARGO_MANIFEST_DIR"
                        | "CARGO_MANIFEST_PATH"
                        | "CARGO_MANIFEST_LINKS"
                        | "CARGO_BIN_NAME"
                        | "CARGO_CRATE_NAME"
                )
        }) {
            command.env_remove(name);
        }
    }
}

/// Resolve generated consumers against the same public packages as this workspace.
/// Local core dependencies keep version + path; packaged core remains registry-only.
pub(super) fn dependencies(names: &[&str]) -> crate::Result<String> {
    static PACKAGES: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    if PACKAGES.get().is_none() {
        let output = command()
            .args([
                "metadata",
                "--offline",
                "--locked",
                "--format-version=1",
                "--manifest-path",
            ])
            .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../Cargo.toml"))
            .output()?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
        }
        let _ = PACKAGES.set(serde_json::from_slice(&output.stdout)?);
    }
    let packages = PACKAGES.get().expect("resolved packages")["packages"]
        .as_array()
        .ok_or("invalid cargo metadata")?;
    let mut manifest = String::new();
    for name in names {
        let package = packages
            .iter()
            .find(|p| p["name"].as_str() == Some(name))
            .ok_or_else(|| format!("missing dependency {name}"))?;
        let version = package["version"].as_str().ok_or("missing version")?;
        let path = if package["source"].is_null() {
            let path = std::path::Path::new(
                package["manifest_path"]
                    .as_str()
                    .ok_or("missing manifest")?,
            )
            .parent()
            .ok_or("manifest parent")?;
            format!(", path = {:?}", path)
        } else {
            String::new()
        };
        manifest.push_str(&format!("{name} = {{ version = \"={version}\"{path} }}\n"));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_child_explicitly_selects_release_and_optional_oracle() {
        for oracle in [false, true] {
            let arguments = vec!["--scale".to_owned(), "24".to_owned()];
            let command = release_benchmark(&arguments, oracle);
            let args = command
                .get_args()
                .map(|arg| arg.to_str().unwrap())
                .collect::<Vec<_>>();
            let mut expected = vec![
                "run",
                "--manifest-path",
                concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
                "--locked",
                "--release",
            ];
            if oracle {
                expected.extend(["--features", "oracle"]);
            }
            expected.extend(["--", "bench", "--scale", "24"]);
            assert_eq!(args, expected);
            assert!(command.get_envs().any(|(key, value)| {
                key == BENCH_RELEASE_CHILD && value == Some(std::ffi::OsStr::new("1"))
            }));
        }
    }

    #[test]
    fn child_cargo_clears_package_identity_but_preserves_user_configuration() {
        let mut command = std::process::Command::new("cargo");
        let values = [
            ("CARGO_MANIFEST_DIR", "/tmp/parent"),
            ("CARGO_PKG_NAME", "parent-package"),
            ("CARGO_PKG_VERSION_MAJOR", "9"),
            ("CARGO_TARGET_DIR", "/tmp/custom-target"),
            ("CARGO_BUILD_JOBS", "2"),
        ];
        command.envs(values);
        remove_package_identity(&mut command, values.map(|(name, _)| name.into()));
        let env = command
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();
        for name in [
            "CARGO_MANIFEST_DIR",
            "CARGO_PKG_NAME",
            "CARGO_PKG_VERSION_MAJOR",
        ] {
            assert_eq!(env[std::ffi::OsStr::new(name)], None);
        }
        for (name, value) in &values[3..] {
            assert_eq!(
                env[std::ffi::OsStr::new(name)],
                Some(std::ffi::OsStr::new(value))
            );
        }
    }
}
