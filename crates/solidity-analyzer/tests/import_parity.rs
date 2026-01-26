use std::fs;
use std::path::Path;
use std::time::Duration;

use sa_span::lsp::to_lsp_position;
use sa_test_support::{
    extract_offset,
    lsp::{
        drain_startup_messages, response_result, send_notification, send_request, wait_for_publish,
    },
    setup_foundry_root,
};
use tempfile::tempdir;
use tower_lsp::LspService;
use tower_lsp::lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    GotoDefinitionParams, GotoDefinitionResponse, InitializeParams, InitializedParams, Location,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, Url, VersionedTextDocumentIdentifier,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn write_foundry_toml(root: &Path) {
    let foundry_toml = r#"
[profile.default]
remappings = ["lib/foo:dep/=lib/foo/deps/", "dep/=lib/default/"]
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");
}

fn contains_missing_file_diag(publish: &tower_lsp::lsp_types::PublishDiagnosticsParams) -> bool {
    publish.diagnostics.iter().any(|diag| {
        diag.source.as_deref() == Some("solar")
            && diag.message.contains("file")
            && diag.message.contains("not found")
    })
}

async fn start_lsp(
    root: &Path,
) -> (
    LspService<solidity_analyzer::Server>,
    tower_lsp::ClientSocket,
) {
    let root_uri = Url::from_file_path(root).expect("root uri");
    let (mut service, mut socket) = LspService::new(solidity_analyzer::Server::new);
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

#[tokio::test(flavor = "multi_thread")]
async fn lsp_diagnostics_and_goto_definition_resolve_remapped_imports() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    setup_foundry_root(&root);
    write_foundry_toml(&root);

    fs::create_dir_all(root.join("lib/foo/src")).expect("foo src dir");
    fs::create_dir_all(root.join("lib/foo/deps")).expect("foo deps dir");

    let dep_text = r#"
pragma solidity ^0.8.20;
contract Thing {}
"#;
    fs::write(root.join("lib/foo/deps/Thing.sol"), dep_text).expect("write dep");

    let (main_text, import_offset) = extract_offset(
        r#"
pragma solidity ^0.8.20;
import "dep/Thi/*caret*/ng.sol";
contract Main { Thing value; }
"#,
    );
    let main_path = root.join("lib/foo/src/Main.sol");
    fs::write(&main_path, &main_text).expect("write main");

    let (mut service, mut socket) = start_lsp(&root).await;
    let dep_uri = Url::from_file_path(root.join("lib/foo/deps/Thing.sol")).expect("dep uri");
    let main_uri = Url::from_file_path(&main_path).expect("main uri");

    send_notification(
        &mut service,
        "textDocument/didOpen",
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: dep_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: dep_text.to_string(),
            },
        },
    )
    .await;

    send_notification(
        &mut service,
        "textDocument/didOpen",
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: main_text.clone(),
            },
        },
    )
    .await;

    send_notification(
        &mut service,
        "textDocument/didChange",
        DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: main_uri.clone(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: main_text.clone(),
            }],
        },
    )
    .await;

    let publish = wait_for_publish(&mut socket, TEST_TIMEOUT, &main_uri, |_| true).await;
    assert!(
        !contains_missing_file_diag(&publish),
        "expected remapped imports to resolve in diagnostics"
    );

    let position = to_lsp_position(import_offset, &main_text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let expected_uri = Url::from_file_path(root.join("lib/foo/deps/Thing.sol")).expect("dep uri");
    assert_eq!(location.uri, expected_uri);
}

#[tokio::test(flavor = "multi_thread")]
async fn lsp_on_change_diagnostics_resolve_remapped_imports() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    setup_foundry_root(&root);
    write_foundry_toml(&root);

    fs::create_dir_all(root.join("lib/foo/src")).expect("foo src dir");
    fs::create_dir_all(root.join("lib/foo/deps")).expect("foo deps dir");

    fs::write(
        root.join("lib/foo/deps/Thing.sol"),
        r#"
pragma solidity ^0.8.20;
contract Thing {}
"#,
    )
    .expect("write dep");

    let broken_text = r#"
pragma solidity ^0.8.20;
import "dep/Missing.sol";
contract Overlay {}
"#;
    let fixed_text = r#"
pragma solidity ^0.8.20;
import "dep/Thing.sol";
contract Overlay {}
"#;

    let overlay_path = root.join("lib/foo/src/Overlay.sol");
    fs::write(&overlay_path, broken_text).expect("write overlay");
    let overlay_uri = Url::from_file_path(&overlay_path).expect("overlay uri");

    let (mut service, mut socket) = start_lsp(&root).await;

    send_notification(
        &mut service,
        "textDocument/didOpen",
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: overlay_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: broken_text.to_string(),
            },
        },
    )
    .await;

    send_notification(
        &mut service,
        "textDocument/didChange",
        DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: overlay_uri.clone(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: broken_text.to_string(),
            }],
        },
    )
    .await;

    let publish = wait_for_publish(&mut socket, TEST_TIMEOUT, &overlay_uri, |publish| {
        contains_missing_file_diag(publish)
    })
    .await;
    assert!(contains_missing_file_diag(&publish));

    send_notification(
        &mut service,
        "textDocument/didChange",
        DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: overlay_uri.clone(),
                version: 3,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: fixed_text.to_string(),
            }],
        },
    )
    .await;

    let publish = wait_for_publish(&mut socket, TEST_TIMEOUT, &overlay_uri, |publish| {
        !contains_missing_file_diag(publish)
    })
    .await;
    assert!(
        !contains_missing_file_diag(&publish),
        "expected remapped imports to resolve after on-change updates"
    );
}
