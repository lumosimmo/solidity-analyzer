use std::fs;

use sa_config::ResolvedFoundryConfig;
use sa_ide_diagnostics::collect_solar_lints;
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryWorkspace};
use tempfile::tempdir;

#[test]
fn fixable_lints_are_marked() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::create_dir_all(root.join("lib")).expect("lib dir");
    fs::create_dir_all(root.join("test")).expect("test dir");
    fs::create_dir_all(root.join("script")).expect("script dir");

    let source = r#"
pragma solidity ^0.8.20;
contract LintTest {
    function Bad_Name() public {}

    function compute(uint256 shift) public pure returns (uint256) {
        uint256 value = 1 << shift;
        return value;
    }
}
"#;
    let file_path = root.join("src/LintTest.sol");
    fs::write(&file_path, source).expect("write source");

    let root_path = NormalizedPath::new(root.to_string_lossy());
    let profile = FoundryProfile::new("default");
    let workspace = FoundryWorkspace::new(root_path, profile.clone());
    let config = ResolvedFoundryConfig::new(workspace, profile);

    let lints = collect_solar_lints(&config, &[file_path]).expect("collect lints");
    let mixed_case = lints
        .iter()
        .find(|diag| diag.code.as_deref() == Some("mixed-case-function"))
        .expect("mixed-case-function lint");
    let incorrect_shift = lints
        .iter()
        .find(|diag| diag.code.as_deref() == Some("incorrect-shift"))
        .expect("incorrect-shift lint");

    assert!(mixed_case.fixable);
    assert!(!incorrect_shift.fixable);
}
