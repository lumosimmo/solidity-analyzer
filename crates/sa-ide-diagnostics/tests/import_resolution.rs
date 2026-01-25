use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sa_config::ResolvedFoundryConfig;
use sa_ide_diagnostics::{
    Diagnostic, DiagnosticSource, collect_solar_lints, collect_solar_lints_with_overlay,
};
use sa_paths::NormalizedPath;
use sa_test_utils::load_foundry_config;
use sa_vfs::{Vfs, VfsChange};
use tempfile::tempdir;

fn setup_config(root: &Path, foundry_toml: &str) -> ResolvedFoundryConfig {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::create_dir_all(root.join("lib")).expect("lib dir");
    fs::create_dir_all(root.join("test")).expect("test dir");
    fs::create_dir_all(root.join("script")).expect("script dir");
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    load_foundry_config(root, None).expect("load foundry config")
}

fn write_file(root: &Path, relative: &str, text: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create dir");
    }
    fs::write(&path, text).expect("write file");
    path
}

fn has_missing_file_diagnostic(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|diag| {
        diag.source == DiagnosticSource::Solar
            && diag.message.contains("file")
            && diag.message.contains("not found")
    })
}

#[test]
fn solar_lints_resolve_remapped_imports() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let foundry_toml = r#"
[profile.default]
remappings = ["forge-std/=lib/forge-std/src/"]
"#;
    let config = setup_config(root, foundry_toml);

    write_file(
        root,
        "lib/forge-std/src/Test.sol",
        r#"
pragma solidity ^0.8.20;
contract Test {}
"#,
    );
    let main_path = write_file(
        root,
        "src/Main.sol",
        r#"
pragma solidity ^0.8.20;
import "forge-std/Test.sol";
contract Main { Test t; }
"#,
    );

    let diagnostics = collect_solar_lints(&config, &[main_path]).expect("collect lints");
    assert!(
        !has_missing_file_diagnostic(&diagnostics),
        "expected remapped imports to resolve"
    );
}

#[test]
fn solar_lints_resolve_include_paths() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let foundry_toml = r#"
[profile.default]
include_paths = ["vendor"]
"#;
    let config = setup_config(root, foundry_toml);

    write_file(
        root,
        "vendor/Lib.sol",
        r#"
pragma solidity ^0.8.20;
contract Lib {}
"#,
    );
    let main_path = write_file(
        root,
        "src/Main.sol",
        r#"
pragma solidity ^0.8.20;
import "Lib.sol";
contract Main { Lib lib; }
"#,
    );

    let diagnostics = collect_solar_lints(&config, &[main_path]).expect("collect lints");
    assert!(
        !has_missing_file_diagnostic(&diagnostics),
        "expected include paths to resolve imports"
    );
}

#[test]
fn solar_lints_resolve_base_path() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let foundry_toml = r#"
[profile.default]
"#;
    let config = setup_config(root, foundry_toml);

    write_file(
        root,
        "src/utils/Util.sol",
        r#"
pragma solidity ^0.8.20;
contract Util {}
"#,
    );
    let main_path = write_file(
        root,
        "src/Main.sol",
        r#"
pragma solidity ^0.8.20;
import "src/utils/Util.sol";
contract Main { Util util; }
"#,
    );

    let diagnostics = collect_solar_lints(&config, &[main_path]).expect("collect lints");
    assert!(
        !has_missing_file_diagnostic(&diagnostics),
        "expected base path to resolve workspace-relative imports"
    );
}

#[test]
fn solar_lints_resolve_remapped_imports_with_overlay() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let foundry_toml = r#"
[profile.default]
remappings = ["forge-std/=lib/forge-std/src/"]
"#;
    let config = setup_config(root, foundry_toml);

    write_file(
        root,
        "lib/forge-std/src/Test.sol",
        r#"
pragma solidity ^0.8.20;
contract Test {}
"#,
    );
    let main_path = write_file(
        root,
        "src/Main.sol",
        r#"
pragma solidity ^0.8.20;
contract Main {}
"#,
    );

    let overlay_text = r#"
pragma solidity ^0.8.20;
import "forge-std/Test.sol";
contract Main { Test t; }
"#;
    let mut vfs = Vfs::default();
    vfs.apply_change(VfsChange::Set {
        path: NormalizedPath::new(main_path.to_string_lossy()),
        text: Arc::from(overlay_text),
    });
    let snapshot = vfs.snapshot();

    let diagnostics =
        collect_solar_lints_with_overlay(&config, &[main_path], &snapshot).expect("collect lints");
    assert!(
        !has_missing_file_diagnostic(&diagnostics),
        "expected overlay imports to resolve with remappings"
    );
}

#[test]
fn solar_lints_resolve_contracts_segment_remappings() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let foundry_toml = r#"
[profile.default]
remappings = ["@oz/=lib/openzeppelin-contracts/contracts/"]
"#;
    let config = setup_config(root, foundry_toml);

    write_file(
        root,
        "lib/openzeppelin-contracts/contracts/token/ERC20/ERC20.sol",
        r#"
pragma solidity ^0.8.20;
contract ERC20 {}
"#,
    );
    let main_path = write_file(
        root,
        "src/Main.sol",
        r#"
pragma solidity ^0.8.20;
import "@oz/token/ERC20/ERC20.sol";
contract Main { ERC20 token; }
"#,
    );

    let diagnostics = collect_solar_lints(&config, &[main_path]).expect("collect lints");
    assert!(
        !has_missing_file_diagnostic(&diagnostics),
        "expected contracts/ remapping to resolve imports"
    );
}
