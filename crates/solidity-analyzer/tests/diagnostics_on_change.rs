use sa_test_support::lsp::{
    drain_startup_messages, response_result, send_notification, send_request, wait_for_publish,
};
use sa_test_support::setup_foundry_root;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tempfile::tempdir;
use tower_lsp::ClientSocket;
use tower_lsp::lsp_types::{
    ClientCapabilities, DidChangeConfigurationParams, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, InitializeParams, InitializedParams, TextDocumentContentChangeEvent,
    TextDocumentItem, Url, VersionedTextDocumentIdentifier,
};

async fn initialize_server(
    root: &Path,
) -> (
    tower_lsp::LspService<solidity_analyzer::Server>,
    ClientSocket,
) {
    let root_uri = Url::from_file_path(root).expect("root uri");
    let (mut service, mut socket) = tower_lsp::LspService::new(solidity_analyzer::Server::new);
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;
    drain_startup_messages(&mut socket).await;
    (service, socket)
}

async fn enable_diagnostics_on_change(
    service: &mut tower_lsp::LspService<solidity_analyzer::Server>,
) {
    let settings = json!({
        "solidityAnalyzer": {
            "diagnostics": { "enable": true, "onChange": true }
        }
    });
    send_notification(
        service,
        "workspace/didChangeConfiguration",
        DidChangeConfigurationParams { settings },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_on_change_publish_and_clear() {
    let root_dir = tempdir().expect("tempdir");
    let root = root_dir.path().canonicalize().expect("canonicalize root");
    setup_foundry_root(&root);

    let on_disk = r#"
pragma solidity ^0.8.20;
contract Main {
    function run() public {
        uint256 value = 1;
    }
}
"#;
    let broken = r#"
pragma solidity ^0.8.20;
contract Main {
    function run() public {
        uint256 value =
    }
}
"#;

    let file_path = root.join("src/Main.sol");
    fs::write(&file_path, on_disk).expect("write source");
    let canonical_file = file_path.canonicalize().expect("canonicalize file");

    let (mut service, mut socket) = initialize_server(&root).await;
    enable_diagnostics_on_change(&mut service).await;

    let file_uri = Url::from_file_path(&canonical_file).expect("file uri");
    let open_params = DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri: file_uri.clone(),
            language_id: "solidity".to_string(),
            version: 1,
            text: broken.to_string(),
        },
    };
    send_notification(&mut service, "textDocument/didOpen", open_params).await;

    let publish = wait_for_publish(&mut socket, Duration::from_secs(10), &file_uri, |publish| {
        !publish.diagnostics.is_empty()
    })
    .await;
    assert!(!publish.diagnostics.is_empty());

    let change = DidChangeTextDocumentParams {
        text_document: VersionedTextDocumentIdentifier {
            uri: file_uri.clone(),
            version: 2,
        },
        content_changes: vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: on_disk.to_string(),
        }],
    };
    send_notification(&mut service, "textDocument/didChange", change).await;

    let cleared = wait_for_publish(&mut socket, Duration::from_secs(10), &file_uri, |publish| {
        publish.diagnostics.is_empty()
    })
    .await;
    assert!(cleared.diagnostics.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_on_change_publishes_without_foundry_config() {
    let root_dir = tempdir().expect("tempdir");
    let root = root_dir.path().canonicalize().expect("canonicalize root");
    fs::create_dir_all(&root).expect("root dir");
    assert!(
        !root.join("foundry.toml").exists(),
        "unexpected foundry.toml in temp root"
    );

    let file_path = root.join("Main.sol");
    let broken = r#"
contract Main {
"#;
    fs::write(&file_path, broken).expect("write source");
    let canonical_file = file_path.canonicalize().expect("canonicalize file");

    let (mut service, mut socket) = initialize_server(&root).await;
    enable_diagnostics_on_change(&mut service).await;

    let file_uri = Url::from_file_path(&canonical_file).expect("file uri");
    let open_params = DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri: file_uri.clone(),
            language_id: "solidity".to_string(),
            version: 1,
            text: broken.to_string(),
        },
    };
    send_notification(&mut service, "textDocument/didOpen", open_params).await;

    let publish = wait_for_publish(&mut socket, Duration::from_secs(10), &file_uri, |publish| {
        !publish.diagnostics.is_empty()
    })
    .await;
    assert!(!publish.diagnostics.is_empty());
}
