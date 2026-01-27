use std::fs;
use std::path::{Path, PathBuf};

use foundry_config::{Config, SolcReq};
use sa_config::ResolvedFoundryConfig;
use sa_flycheck::{FlycheckConfig, FlycheckHandle, FlycheckRequest, FlycheckSeverity};
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryWorkspace};
use sa_test_support::setup_foundry_root;
use sa_test_utils::toolchain::{
    SolcTestMode, StubSolcOptions, solc_path_for_tests_with_mode, stub_solc_output_error,
};
use tempfile::tempdir;
use tokio::time::{Duration, timeout};

const FLYCHECK_TIMEOUT: Duration = Duration::from_secs(10);
const STALE_TIMEOUT: Duration = Duration::from_secs(1);

fn make_config(root: &Path, solc_path: &Path) -> ResolvedFoundryConfig {
    let root_path = NormalizedPath::new(root.to_string_lossy());
    let profile = FoundryProfile::new("default").with_solc_version(solc_path.to_string_lossy());
    let workspace = FoundryWorkspace::new(root_path.clone());
    let mut foundry_config = Config::with_root(PathBuf::from(root_path.as_str()));
    foundry_config.solc = Some(SolcReq::Local(solc_path.to_path_buf()));
    let foundry_config = foundry_config.sanitized();
    ResolvedFoundryConfig::new(workspace, profile).with_foundry_config(foundry_config)
}

fn write_test_source(root: &Path) {
    let source = r#"
pragma solidity ^0.8.20;

contract Main {
"#;
    fs::write(root.join("src/Main.sol"), source).expect("write source");
}

#[tokio::test(flavor = "multi_thread")]
async fn flycheck_reports_solc_diagnostics() {
    let root_dir = tempdir().expect("tempdir");
    setup_foundry_root(root_dir.path());
    write_test_source(root_dir.path());

    let solc_dir = tempdir().expect("solc dir");
    let solc_json = stub_solc_output_error();
    let solc_path = solc_path_for_tests_with_mode(
        solc_dir.path(),
        "0.8.20",
        StubSolcOptions {
            json: Some(solc_json),
            sleep_seconds: None,
            capture_stdin: false,
        },
        SolcTestMode::Stub,
    );

    let config = make_config(root_dir.path(), &solc_path);
    let (handle, mut results) = FlycheckHandle::spawn(FlycheckConfig::default());

    handle
        .check(FlycheckRequest::new(config))
        .await
        .expect("send flycheck request");

    let result = timeout(FLYCHECK_TIMEOUT, results.recv())
        .await
        .expect("flycheck timeout")
        .expect("flycheck result");

    assert_eq!(result.generation, 1);
    assert_eq!(result.diagnostics.len(), 1);
    let diag = &result.diagnostics[0];
    let expected_path = NormalizedPath::new(root_dir.path().join("src/Main.sol").to_string_lossy());
    assert_eq!(diag.file_path, expected_path);
    assert_eq!(diag.severity, FlycheckSeverity::Error);
    assert_eq!(diag.code.as_deref(), Some("1234"));
    assert_eq!(diag.message, "stub error");
}

#[tokio::test(flavor = "multi_thread")]
async fn flycheck_drops_stale_results() {
    let root_dir = tempdir().expect("tempdir");
    setup_foundry_root(root_dir.path());
    write_test_source(root_dir.path());

    let solc_dir = tempdir().expect("solc dir");
    let sleep_seconds = 1;
    let solc_json = stub_solc_output_error();
    let solc_path = solc_path_for_tests_with_mode(
        solc_dir.path(),
        "0.8.20",
        StubSolcOptions {
            json: Some(solc_json),
            sleep_seconds: Some(sleep_seconds),
            capture_stdin: false,
        },
        SolcTestMode::Stub,
    );

    let config = make_config(root_dir.path(), &solc_path);
    let (handle, mut results) = FlycheckHandle::spawn(FlycheckConfig {
        debounce: Duration::from_millis(10),
    });

    handle
        .check(FlycheckRequest::new(config.clone()))
        .await
        .expect("send first request");
    tokio::time::sleep(Duration::from_millis(20)).await;
    handle
        .check(FlycheckRequest::new(config))
        .await
        .expect("send second request");

    let result = timeout(FLYCHECK_TIMEOUT, results.recv())
        .await
        .expect("flycheck timeout")
        .expect("flycheck result");
    assert_eq!(result.generation, 2);

    let none = timeout(STALE_TIMEOUT, results.recv()).await;
    assert!(none.is_err(), "stale diagnostics should be dropped");
}
