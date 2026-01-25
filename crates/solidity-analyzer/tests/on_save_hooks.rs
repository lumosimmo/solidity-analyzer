use futures::{SinkExt, StreamExt};
use sa_span::lsp::from_lsp_range;
use sa_test_support::lsp::{
    diagnostic_codes, drain_startup_messages, respond_to_request, response_result,
    send_notification, send_request, wait_for_publish,
};
use sa_test_support::setup_foundry_root;
use sa_test_utils::toolchain::{StubSolcOptions, solc_path_for_tests};
use serde_json::json;
use std::cmp::Reverse;
use std::fs;
use tempfile::tempdir;
use test_fixtures::{DiagnosticsTestContext, lint_test_source};
use tokio::time::{Duration, timeout};
use tower_lsp::lsp_types::{
    ApplyWorkspaceEditParams, ApplyWorkspaceEditResponse, ClientCapabilities, DiagnosticSeverity,
    DidChangeConfigurationParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    DocumentChanges, InitializeParams, InitializedParams, NumberOrString, OneOf,
    TextDocumentIdentifier, TextDocumentItem, TextEdit, Url, WorkspaceClientCapabilities,
};

mod test_fixtures;

fn apply_text_edits(text: &str, edits: &[TextEdit]) -> String {
    let mut resolved = edits
        .iter()
        .map(|edit| {
            let range = from_lsp_range(edit.range, text).expect("valid edit range");
            (range, edit.new_text.clone())
        })
        .collect::<Vec<_>>();
    resolved.sort_by_key(|(range, _)| Reverse(range.start()));

    let mut result = text.to_string();
    for (range, new_text) in resolved {
        let start: usize = range.start().into();
        let end: usize = range.end().into();
        result.replace_range(start..end, &new_text);
    }
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn format_on_save_applies_workspace_edit() {
    let root_dir = tempdir().expect("tempdir");
    let root = root_dir.path().canonicalize().expect("canonicalize root");
    setup_foundry_root(&root);

    let foundry_toml = r#"
[profile.default]

[fmt]
style = "space"
tab_width = 2
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let text = r#"
contract Main{function run()public returns(uint256){return 1;}}
"#
    .trim();
    let file_path = root.join("src/Main.sol");
    fs::write(&file_path, text).expect("write source");
    let canonical_file_path = file_path.canonicalize().expect("canonicalize file");

    let root_uri = Url::from_file_path(&root).expect("root uri");
    let (mut service, mut socket) = tower_lsp::LspService::new(solidity_analyzer::Server::new);
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities {
            workspace: Some(WorkspaceClientCapabilities {
                apply_edit: Some(true),
                ..WorkspaceClientCapabilities::default()
            }),
            ..ClientCapabilities::default()
        },
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;
    drain_startup_messages(&mut socket).await;

    let settings = serde_json::json!({
        "solidityAnalyzer": {
            "format": { "onSave": true },
            "lint": { "onSave": false }
        }
    });
    send_notification(
        &mut service,
        "workspace/didChangeConfiguration",
        DidChangeConfigurationParams { settings },
    )
    .await;

    let file_uri = Url::from_file_path(&canonical_file_path).expect("file uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_uri.clone(),
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
            text_document: TextDocumentIdentifier {
                uri: file_uri.clone(),
            },
            text: Some(text.to_string()),
        },
    )
    .await;

    let apply = timeout(Duration::from_secs(10), async {
        loop {
            let request = match socket.next().await {
                Some(request) => request,
                None => return None,
            };
            if request.method() != "workspace/applyEdit" {
                respond_to_request(&mut socket, &request).await;
                continue;
            }
            let id = request.id().cloned().expect("apply edit id");
            let params = match request.params() {
                Some(params) => params.clone(),
                None => return None,
            };
            let apply: ApplyWorkspaceEditParams =
                serde_json::from_value(params).expect("apply edit params");
            let response = tower_lsp::jsonrpc::Response::from_ok(
                id,
                serde_json::to_value(ApplyWorkspaceEditResponse {
                    applied: true,
                    failure_reason: None,
                    failed_change: None,
                })
                .expect("serialize apply edit response"),
            );
            socket
                .send(response)
                .await
                .expect("send apply edit response");
            return Some(apply);
        }
    })
    .await
    .expect("applyEdit timeout")
    .expect("applyEdit request");

    let tower_lsp::lsp_types::WorkspaceEdit {
        changes,
        document_changes,
        ..
    } = apply.edit;
    let file_uri_ref = &file_uri;
    let edits = match document_changes {
        Some(DocumentChanges::Edits(edits)) => {
            let document_edit = edits
                .into_iter()
                .find(|edit| edit.text_document.uri == *file_uri_ref)
                .expect("main edits");
            document_edit
                .edits
                .into_iter()
                .map(|edit| match edit {
                    OneOf::Left(edit) => edit,
                    OneOf::Right(edit) => edit.text_edit,
                })
                .collect::<Vec<_>>()
        }
        Some(DocumentChanges::Operations(_)) => {
            panic!("unexpected document change operations");
        }
        None => {
            let changes = changes.expect("changes");
            changes.get(file_uri_ref).cloned().expect("main edits")
        }
    };
    assert!(!edits.is_empty());
    let formatted = apply_text_edits(text, &edits);
    assert!(formatted.contains("\n  function run()"));
    assert!(formatted.contains("\n    return 1;"));
}

#[tokio::test(flavor = "multi_thread")]
async fn lint_on_save_publishes_diagnostics() {
    let root_dir = tempdir().expect("tempdir");
    let root = root_dir.path().canonicalize().expect("canonicalize root");
    setup_foundry_root(&root);

    let solc_dir = tempdir().expect("solc dir");
    let solc_path = solc_path_for_tests(
        solc_dir.path(),
        "0.8.20",
        StubSolcOptions {
            json: None,
            sleep_seconds: None,
            capture_stdin: false,
        },
    );
    let foundry_toml = format!(
        r#"
[profile.default]
solc = "{solc}"
"#,
        solc = solc_path.to_string_lossy()
    );
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let source = lint_test_source();
    // Keep LintTest.sol aligned with the contract name for this scenario.
    let file_path = root.join("src/LintTest.sol");
    fs::write(&file_path, &source).expect("write source");

    let root_uri = Url::from_file_path(&root).expect("root uri");
    let (mut service, mut socket) = tower_lsp::LspService::new(solidity_analyzer::Server::new);
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities {
            workspace: Some(WorkspaceClientCapabilities::default()),
            ..ClientCapabilities::default()
        },
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;
    drain_startup_messages(&mut socket).await;

    let settings = serde_json::json!({
        "solidityAnalyzer": {
            "format": { "onSave": false },
            "lint": { "onSave": true }
        }
    });
    send_notification(
        &mut service,
        "workspace/didChangeConfiguration",
        DidChangeConfigurationParams { settings },
    )
    .await;

    let canonical_file_path = file_path.canonicalize().expect("canonicalize file");
    let file_uri = Url::from_file_path(&canonical_file_path).expect("file uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: source.to_string(),
            },
        },
    )
    .await;

    send_notification(
        &mut service,
        "textDocument/didSave",
        DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: file_uri.clone(),
            },
            text: Some(source.to_string()),
        },
    )
    .await;

    let publish = wait_for_publish(&mut socket, Duration::from_secs(10), &file_uri, |publish| {
        diagnostic_codes(publish)
            .iter()
            .any(|code| code == "mixed-case-function")
    })
    .await;

    let lint_diag = publish
        .diagnostics
        .iter()
        .find(|diag| {
            matches!(diag.code, Some(NumberOrString::String(ref code)) if code == "mixed-case-function")
        })
        .expect("lint diagnostic");
    assert_eq!(lint_diag.severity, Some(DiagnosticSeverity::INFORMATION));
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_on_lint_off_publishes_solc_only() {
    let context = DiagnosticsTestContext::new();
    let (mut service, mut socket) = context.start_service(None).await;

    let settings = json!({
        "solidityAnalyzer": {
            "diagnostics": { "enable": true, "onSave": true },
            "lint": { "enable": false, "onSave": true }
        }
    });
    send_notification(
        &mut service,
        "workspace/didChangeConfiguration",
        DidChangeConfigurationParams { settings },
    )
    .await;

    context.open_and_save(&mut service).await;

    let publish = wait_for_publish(
        &mut socket,
        Duration::from_secs(10),
        &context.file_uri,
        |publish| diagnostic_codes(publish).iter().any(|code| code == "1234"),
    )
    .await;
    let codes = diagnostic_codes(&publish);
    assert!(codes.iter().any(|code| code == "1234"));
    assert!(!codes.iter().any(|code| code == "mixed-case-function"));
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_off_lint_on_publishes_solar_only() {
    let context = DiagnosticsTestContext::new();
    let (mut service, mut socket) = context.start_service(None).await;

    let settings = json!({
        "solidityAnalyzer": {
            "diagnostics": { "enable": false, "onSave": true },
            "lint": { "enable": true, "onSave": true }
        }
    });
    send_notification(
        &mut service,
        "workspace/didChangeConfiguration",
        DidChangeConfigurationParams { settings },
    )
    .await;

    context.open_and_save(&mut service).await;

    let publish = wait_for_publish(
        &mut socket,
        Duration::from_secs(10),
        &context.file_uri,
        |publish| {
            diagnostic_codes(publish)
                .iter()
                .any(|code| code == "mixed-case-function")
        },
    )
    .await;
    let codes = diagnostic_codes(&publish);
    assert!(codes.iter().any(|code| code == "mixed-case-function"));
    assert!(!codes.iter().any(|code| code == "1234"));
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_and_lint_off_clears_published_diagnostics() {
    let context = DiagnosticsTestContext::new();
    let (mut service, mut socket) = context.start_service(None).await;

    let enabled_settings = json!({
        "solidityAnalyzer": {
            "diagnostics": { "enable": true, "onSave": true },
            "lint": { "enable": true, "onSave": true }
        }
    });
    send_notification(
        &mut service,
        "workspace/didChangeConfiguration",
        DidChangeConfigurationParams {
            settings: enabled_settings,
        },
    )
    .await;

    context.open_and_save(&mut service).await;

    let initial_publish = wait_for_publish(
        &mut socket,
        Duration::from_secs(10),
        &context.file_uri,
        |publish| !publish.diagnostics.is_empty(),
    )
    .await;
    assert!(!initial_publish.diagnostics.is_empty());

    let disabled_settings = json!({
        "solidityAnalyzer": {
            "diagnostics": { "enable": false, "onSave": false },
            "lint": { "enable": false, "onSave": false }
        }
    });
    send_notification(
        &mut service,
        "workspace/didChangeConfiguration",
        DidChangeConfigurationParams {
            settings: disabled_settings,
        },
    )
    .await;

    context.save(&mut service).await;

    let cleared_publish = wait_for_publish(
        &mut socket,
        Duration::from_secs(10),
        &context.file_uri,
        |publish| publish.diagnostics.is_empty(),
    )
    .await;
    assert!(cleared_publish.diagnostics.is_empty());
}
