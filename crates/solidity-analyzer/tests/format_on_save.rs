use futures::{SinkExt, StreamExt};
use sa_test_support::lsp::{
    drain_startup_messages, make_server, respond_to_request, response_result, send_notification,
    send_request,
};
use sa_test_utils::FixtureBuilder;
use serde_json::json;
use tokio::time::{Duration, Instant, timeout};
use tower_lsp::jsonrpc::{Error, ErrorCode, Response};
use tower_lsp::lsp_types::{
    ClientCapabilities, DidOpenTextDocumentParams, DidSaveTextDocumentParams, InitializeParams,
    InitializedParams, TextDocumentIdentifier, TextDocumentItem, Url,
};

async fn wait_for_apply_edit_request(
    socket: &mut tower_lsp::ClientSocket,
) -> tower_lsp::jsonrpc::Id {
    loop {
        let request = socket.next().await.expect("client request");
        if request.method() == "workspace/applyEdit" {
            return request.id().cloned().expect("apply edit id");
        }
        respond_to_request(socket, &request).await;
    }
}

async fn assert_no_apply_edit(socket: &mut tower_lsp::ClientSocket, timeout_duration: Duration) {
    let deadline = Instant::now() + timeout_duration;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        let request = match timeout(remaining, socket.next()).await {
            Ok(Some(request)) => request,
            Ok(None) | Err(_) => return,
        };
        if request.method() == "workspace/applyEdit" {
            panic!("unexpected workspace/applyEdit request");
        }
        respond_to_request(socket, &request).await;
    }
}

fn initialize_params(root_uri: Url) -> InitializeParams {
    let initialization_options = json!({
        "solidityAnalyzer": {
            "format": { "onSave": true },
            "diagnostics": { "enable": false, "onSave": false, "onChange": false },
            "lint": { "enable": false, "onSave": false, "onChange": false },
            "toolchain": { "promptInstall": false }
        }
    });
    InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        initialization_options: Some(initialization_options),
        ..InitializeParams::default()
    }
}

#[tokio::test]
async fn format_on_save_reports_apply_edit_error() {
    let foundry_toml = r#"
[profile.default]

[fmt]
style = "space"
tab_width = 2
"#;
    let text = r#"
contract Foo{function bar()public returns(uint256){return 1;}}
"#
    .trim();
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(foundry_toml)
        .file("src/Main.sol", text)
        .build()
        .expect("fixture");

    let (mut service, mut socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let response = send_request(&mut service, 1, "initialize", initialize_params(root_uri)).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;
    drain_startup_messages(&mut socket).await;

    let main_uri = Url::from_file_path(fixture.root().join("src/Main.sol")).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: text.to_string(),
            },
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didSave",
        DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            text: Some(text.to_string()),
        },
    )
    .await;

    let request_id = wait_for_apply_edit_request(&mut socket).await;
    let response = Response::from_error(
        request_id,
        Error {
            code: ErrorCode::InternalError,
            message: "apply edit failed".into(),
            data: None,
        },
    );
    socket.send(response).await.expect("send response");
}

#[tokio::test]
async fn format_on_save_skips_when_no_edits() {
    let foundry_toml = r#"
[profile.default]

[fmt]
style = "space"
tab_width = 2
"#;
    let text =
        "contract Foo {\n  function bar() public returns (uint256) {\n    return 1;\n  }\n}\n";
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(foundry_toml)
        .file("src/Main.sol", text)
        .build()
        .expect("fixture");

    let (mut service, mut socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let response = send_request(&mut service, 1, "initialize", initialize_params(root_uri)).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;
    drain_startup_messages(&mut socket).await;

    let main_uri = Url::from_file_path(fixture.root().join("src/Main.sol")).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: text.to_string(),
            },
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didSave",
        DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            text: Some(text.to_string()),
        },
    )
    .await;

    assert_no_apply_edit(&mut socket, Duration::from_millis(300)).await;
}
