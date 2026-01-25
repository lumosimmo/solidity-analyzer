use std::fs;
use std::path::Path;

use sa_config::ResolvedFoundryConfig;
use sa_ide_diagnostics::collect_solar_lints;
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
    let workspace = FoundryWorkspace::new(root_path, profile.clone());
    ResolvedFoundryConfig::new(workspace, profile)
}

#[test]
fn inline_config_disables_specific_lint() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let config = setup_config(root);

    let source = r#"
pragma solidity ^0.8.20;
contract LintTest {
    function Bad_Name() public {}
    // forge-lint: disable-next-line(mixed-case-function)
    function Bad_Name_Disabled() public {}
}
"#;
    let file_path = root.join("src/LintTest.sol");
    fs::write(&file_path, source).expect("write source");

    let lints = collect_solar_lints(&config, &[file_path]).expect("collect lints");
    let mixed_case_count = lints
        .iter()
        .filter(|diag| diag.code.as_deref() == Some("mixed-case-function"))
        .count();

    assert_eq!(mixed_case_count, 1);
}

#[test]
fn inline_config_ignores_non_leading_block_comment_lines() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let config = setup_config(root);

    let source = r#"
pragma solidity ^0.8.20;
contract LintTest {
    function Bad_Name() public {}
    /*
     * forge-lint: disable-next-line(mixed-case-function)
     */
    function Bad_Name_Disabled() public {}
}
"#;
    let file_path = root.join("src/LintTest.sol");
    fs::write(&file_path, source).expect("write source");

    let lints = collect_solar_lints(&config, &[file_path]).expect("collect lints");
    let mixed_case_count = lints
        .iter()
        .filter(|diag| diag.code.as_deref() == Some("mixed-case-function"))
        .count();

    assert_eq!(mixed_case_count, 2);
}

#[test]
fn inline_config_disable_line_block_comment() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let config = setup_config(root);

    let source = r#"
pragma solidity ^0.8.20;
contract LintTest {
    /* forge-lint: disable-line(mixed-case-function) */ function Bad_Name_Disabled() public {}
    function Bad_Name() public {}
}
"#;
    let file_path = root.join("src/LintTest.sol");
    fs::write(&file_path, source).expect("write source");

    let lints = collect_solar_lints(&config, &[file_path]).expect("collect lints");
    let mixed_case_count = lints
        .iter()
        .filter(|diag| diag.code.as_deref() == Some("mixed-case-function"))
        .count();

    assert_eq!(mixed_case_count, 1);
}

#[test]
fn inline_config_disable_start_end_block_comment() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    let config = setup_config(root);

    let source = r#"
pragma solidity ^0.8.20;
contract LintTest {
    /* forge-lint: disable-start(mixed-case-function) */
    function Bad_Name_Disabled() public {}
    function Also_Bad_Name_Disabled() public {}
    /* forge-lint: disable-end(mixed-case-function) */
    function Bad_Name() public {}
}
"#;
    let file_path = root.join("src/LintTest.sol");
    fs::write(&file_path, source).expect("write source");

    let lints = collect_solar_lints(&config, &[file_path]).expect("collect lints");
    let mixed_case_count = lints
        .iter()
        .filter(|diag| diag.code.as_deref() == Some("mixed-case-function"))
        .count();

    assert_eq!(mixed_case_count, 1);
}
