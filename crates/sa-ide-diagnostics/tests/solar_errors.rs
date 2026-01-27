use std::fs;
use std::path::{Path, PathBuf};

use sa_config::ResolvedFoundryConfig;
use sa_ide_diagnostics::{DiagnosticSeverity, collect_solar_lints};
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryWorkspace};
use tempfile::tempdir;

fn setup_config(root: &Path) -> ResolvedFoundryConfig {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::create_dir_all(root.join("lib")).expect("lib dir");
    fs::create_dir_all(root.join("test")).expect("test dir");
    fs::create_dir_all(root.join("script")).expect("script dir");

    let root_path = NormalizedPath::new(root.to_string_lossy());
    let profile = FoundryProfile::new("default");
    let workspace = FoundryWorkspace::new(root_path);
    ResolvedFoundryConfig::new(workspace, profile)
}

fn write_source(root: &Path, name: &str, source: &str) -> PathBuf {
    let file_path = root.join("src").join(name);
    fs::write(&file_path, source).expect("write source");
    file_path
}

fn assert_solar_lints_keep_errors(name: &str, source: &str, message: &str) {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let config = setup_config(root);
    let file_path_buf = write_source(root, name, source);

    let lints =
        collect_solar_lints(&config, std::slice::from_ref(&file_path_buf)).expect("collect lints");
    let normalized_file_path = NormalizedPath::new(file_path_buf.to_string_lossy());
    let has_error = lints.iter().any(|diag| {
        diag.file_path == normalized_file_path && diag.severity == DiagnosticSeverity::Error
    });

    assert!(has_error, "{}", message);
}

#[test]
fn solar_lints_keep_parse_errors() {
    let source = r#"
pragma solidity ^0.8.20;
contract Broken {
    function run() public {
        uint256 value =
    }
}
"#;
    assert_solar_lints_keep_errors("Broken.sol", source, "expected parse error diagnostics");
}

#[test]
fn solar_lints_keep_semantic_errors() {
    let source = r#"
pragma solidity ^0.8.20;
contract Broken {
    function run() public {
        uint256 value = missing;
    }
}
"#;
    assert_solar_lints_keep_errors(
        "Semantic.sol",
        source,
        "expected semantic error diagnostics",
    );
}
