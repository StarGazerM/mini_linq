//! Child Cargo must not inherit the enclosing `cargo run` package identity.
//! Some native build scripts track these variables, which otherwise invalidates
//! their fingerprints whenever users switch between `cargo xtask` and tests.
pub fn command() -> std::process::Command {
    let mut command =
        std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    remove_package_identity(&mut command, std::env::vars_os().map(|(name, _)| name));
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

#[cfg(test)]
mod tests {
    use super::*;

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
