use std::fs;
use std::path::Path;

const REQUIRED_MEMBERS: &[&str] = &[
    "crates/sa-base-db",
    "crates/sa-vfs",
    "crates/sa-project-model",
    "crates/sa-ide",
    "crates/sa-ide-db",
    "crates/sa-ide-assists",
    "crates/sa-ide-completion",
    "crates/sa-ide-diagnostics",
    "crates/sa-toolchain",
    "crates/sa-load-foundry",
    "crates/sa-config",
    "crates/sa-paths",
    "crates/sa-intern",
    "crates/sa-span",
    "crates/sa-hir",
    "crates/sa-def",
    "crates/sa-syntax",
    "crates/sa-flycheck",
    "crates/solidity-analyzer",
    "xtask",
];

#[test]
fn workspace_members_are_present() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("xtask should live under the workspace root");
    let cargo_toml = workspace_root.join("Cargo.toml");
    let contents = fs::read_to_string(&cargo_toml)
        .unwrap_or_else(|err| panic!("failed to read {cargo_toml:?}: {err}"));

    for member in REQUIRED_MEMBERS {
        let quoted = format!("\"{member}\"");
        assert!(
            contents.contains(&quoted),
            "workspace missing member {member:?}"
        );
    }
}
