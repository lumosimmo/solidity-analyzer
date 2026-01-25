use std::fs;

use sa_config::ResolvedFoundryConfig;
use sa_ide_diagnostics::{
    Diagnostic, DiagnosticSeverity, DiagnosticSource, collect_solar_lints, merge_diagnostics,
};
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryWorkspace};
use sa_span::{TextRange, TextSize};
use tempfile::tempdir;

#[test]
fn merge_deduplicates_by_span_and_code() {
    let file = NormalizedPath::new("/workspace/src/Main.sol");
    let range = TextRange::new(TextSize::from(0), TextSize::from(4));

    let solc = Diagnostic {
        file_path: file.clone(),
        range,
        severity: DiagnosticSeverity::Error,
        code: Some("E100".to_string()),
        source: DiagnosticSource::Solc,
        fixable: false,
        message: "solc error".to_string(),
    };
    let solar = Diagnostic {
        file_path: file.clone(),
        range,
        severity: DiagnosticSeverity::Warning,
        code: Some("E100".to_string()),
        source: DiagnosticSource::ForgeLint,
        fixable: false,
        message: "lint warning".to_string(),
    };

    let merged = merge_diagnostics(vec![solc.clone()], vec![solar]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0], solc);
}

#[test]
fn solar_lints_normalize_code_and_severity() {
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
}
"#;
    let file_path = root.join("src/LintTest.sol");
    fs::write(&file_path, source).expect("write source");

    let root_path = NormalizedPath::new(root.to_string_lossy());
    let profile = FoundryProfile::new("default");
    let workspace = FoundryWorkspace::new(root_path, profile.clone());
    let config = ResolvedFoundryConfig::new(workspace, profile);

    let lints = collect_solar_lints(&config, &[file_path]).expect("collect lints");
    let mixed = lints
        .iter()
        .find(|diag| diag.code.as_deref() == Some("mixed-case-function"))
        .expect("mixed-case-function lint");

    assert_eq!(mixed.severity, DiagnosticSeverity::Info);
    assert_eq!(mixed.source, DiagnosticSource::ForgeLint);
}
