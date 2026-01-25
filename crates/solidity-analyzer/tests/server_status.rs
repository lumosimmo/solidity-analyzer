use serde_json::json;
use std::time::Duration;
use test_fixtures::DiagnosticsTestContext;
use tokio::time::Instant;
use tower_lsp::lsp_types::{
    ClientCapabilities, DidOpenTextDocumentParams, DidSaveTextDocumentParams, InitializeParams,
    TextDocumentIdentifier, TextDocumentItem, Url,
};

use sa_test_support::lsp::LspTestHarness;
use sa_test_utils::toolchain::{StubSolcOptions, stub_solc_output_empty};
use solidity_analyzer::lsp_ext::{Health, ServerStatusParams};

mod test_fixtures;

const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

async fn wait_for_status<F>(
    harness: &mut LspTestHarness<solidity_analyzer::Server>,
    timeout: Duration,
    predicate: F,
) -> ServerStatusParams
where
    F: Fn(&ServerStatusParams) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for serverStatus");

        let request = harness
            .next_request(remaining)
            .await
            .expect("expected server request");
        if request.method() != "experimental/serverStatus" {
            continue;
        }
        let params = request.params().cloned().expect("server status params");
        let status: ServerStatusParams =
            serde_json::from_value(params).expect("server status payload");
        if predicate(&status) {
            return status;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn server_status_reports_compiling_and_ok() {
    let source = r#"
pragma solidity ^0.8.20;

contract StatusTest {
    function run() public {}
}
"#
    .to_string();

    let context = DiagnosticsTestContext::with_solc_options(
        StubSolcOptions {
            json: Some(stub_solc_output_empty()),
            sleep_seconds: Some(1),
            capture_stdin: false,
        },
        source,
    );

    let capabilities = ClientCapabilities {
        experimental: Some(json!({ "serverStatusNotification": true })),
        ..ClientCapabilities::default()
    };

    let initialization_options = json!({
        "solidityAnalyzer": {
            "diagnostics": { "enable": true, "onSave": true, "onChange": false },
            "lint": { "enable": false, "onSave": false, "onChange": false }
        }
    });

    let params = InitializeParams {
        root_uri: Some(Url::from_file_path(&context.root).expect("root uri")),
        capabilities,
        initialization_options: Some(initialization_options),
        ..InitializeParams::default()
    };

    let mut harness = LspTestHarness::new_with_params(params, solidity_analyzer::Server::new).await;

    harness
        .notify(
            "textDocument/didOpen",
            DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: context.file_uri.clone(),
                    language_id: "solidity".to_string(),
                    version: 1,
                    text: context.source.clone(),
                },
            },
        )
        .await;

    harness
        .notify(
            "textDocument/didSave",
            DidSaveTextDocumentParams {
                text_document: TextDocumentIdentifier {
                    uri: context.file_uri.clone(),
                },
                text: Some(context.source.clone()),
            },
        )
        .await;

    let compiling = wait_for_status(&mut harness, STATUS_TIMEOUT, |status| {
        status.message.as_deref() == Some("Compiling...")
    })
    .await;
    assert_eq!(compiling.health, Health::Ok);
    assert!(!compiling.quiescent);

    let done = wait_for_status(&mut harness, STATUS_TIMEOUT, |status| {
        status.message.as_deref() == Some("OK")
    })
    .await;
    assert_eq!(done.health, Health::Ok);
    assert!(done.quiescent);
}
