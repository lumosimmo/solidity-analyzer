use std::fs;

use sa_span::lsp::to_lsp_position;
use sa_test_support::{
    extract_offset,
    lsp::{
        create_foundry_workspace, response_result, send_notification, send_request,
        setup_lsp_service,
    },
};
use tempfile::tempdir;
use tower_lsp::lsp_types::{
    RenameParams, TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Url,
    WorkspaceEdit,
};

#[tokio::test]
async fn rename_returns_workspace_edit() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (main_text, offset) =
        extract_offset("import \"./Thing.sol\"; contract Main { /*caret*/Lib lib; }");
    let thing_text = "contract Lib {}".to_string();
    fs::write(root.join("src/Main.sol"), &main_text).expect("write main");
    fs::write(root.join("src/Thing.sol"), &thing_text).expect("write thing");

    let (mut service, _root_uri) = setup_lsp_service(&root, || {
        tower_lsp::LspService::new(solidity_analyzer::Server::new)
    })
    .await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let thing_uri = Url::from_file_path(root.join("src/Thing.sol")).expect("thing uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
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
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: thing_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: thing_text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(offset, &main_text);
    let params = RenameParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        new_name: "Renamed".to_string(),
        work_done_progress_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/rename", params).await;
    let result = response_result::<Option<WorkspaceEdit>>(response).expect("rename response");

    let changes = result.changes.expect("changes map");
    let main_edits = changes.get(&main_uri).expect("main edits");
    let thing_edits = changes.get(&thing_uri).expect("thing edits");

    assert!(main_edits.iter().all(|edit| edit.new_text == "Renamed"));
    assert!(thing_edits.iter().all(|edit| edit.new_text == "Renamed"));
}

#[tokio::test]
async fn rename_updates_local_binding_only() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (main_text, offset) = extract_offset(
        r#"
contract Main {
    function foo(uint256 value) public {
        uint256 count = value;
        {
            uint256 count = 2;
            count;
        }
        co/*caret*/unt;
    }
}
"#,
    );
    fs::write(root.join("src/Main.sol"), &main_text).expect("write main");

    let (mut service, _root_uri) = setup_lsp_service(&root, || {
        tower_lsp::LspService::new(solidity_analyzer::Server::new)
    })
    .await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: main_text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(offset, &main_text);
    let params = RenameParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        new_name: "total".to_string(),
        work_done_progress_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/rename", params).await;
    let result = response_result::<Option<WorkspaceEdit>>(response).expect("rename response");

    let changes = result.changes.expect("changes map");
    let main_edits = changes.get(&main_uri).expect("main edits");

    assert_eq!(main_edits.len(), 2);
    assert!(main_edits.iter().all(|edit| edit.new_text == "total"));
}
