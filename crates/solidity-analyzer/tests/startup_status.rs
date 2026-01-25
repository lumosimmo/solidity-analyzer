use std::time::Duration;

use sa_test_support::write_stub_solc;
use sa_test_utils::FixtureBuilder;
use sa_test_utils::lsp::LspTestHarness;
use tempfile::tempdir;
use tower_lsp::lsp_types::LogMessageParams;

#[tokio::test]
async fn startup_status_logs_foundry_and_solc() {
    let solc_dir = tempdir().expect("solc dir");
    let solc_path = write_stub_solc(solc_dir.path(), "0.8.20", None);
    let solc_path = solc_path.to_string_lossy().replace('\\', "\\\\");
    let foundry_toml = format!(
        r#"
[profile.default]
solc = "{solc_path}"
remappings = ["lib/=lib/forge-std/src/"]
"#
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(foundry_toml)
        .file(
            "src/Main.sol",
            r#"
contract Main {}
"#,
        )
        .build()
        .expect("fixture");

    let mut harness = LspTestHarness::new(fixture.root(), solidity_analyzer::Server::new).await;
    let request = harness
        .wait_for_request("window/logMessage", Duration::from_secs(2))
        .await
        .expect("startup log message");
    let params = request.params().expect("log params");
    let log: LogMessageParams =
        serde_json::from_value(params.clone()).expect("deserialize log params");

    assert!(log.message.contains("solidity-analyzer status"));
    assert!(log.message.contains("remappings"));
    assert!(log.message.contains("0.8.20"));
}
