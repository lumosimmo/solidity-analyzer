use std::fs;

use sa_span::lsp::to_lsp_position;
use sa_test_support::{
    extract_offset,
    lsp::{
        create_foundry_workspace, make_server, response_result, send_notification, send_request,
        setup_lsp_service,
    },
};
use tempfile::tempdir;
use tower_lsp::lsp_types::{
    CompletionParams, CompletionResponse, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, Url,
};

#[tokio::test]
async fn completion_returns_contract_items() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offset) =
        extract_offset("contract Alpha {}\ncontract Main { Al/*caret*/pha value; }");
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(offset, &text);
    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: None,
    };
    let response = send_request(&mut service, 2, "textDocument/completion", params).await;
    let result =
        response_result::<Option<CompletionResponse>>(response).expect("completion response");

    let items = match result {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => list.items,
    };

    assert!(items.iter().any(|item| item.label == "Alpha"));
}

#[tokio::test]
async fn completion_excludes_unrelated_contract_members() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offset) = extract_offset(
        r#"
import "./Dep.sol";

contract Alpha {
    uint256 numberA;

    function test() public {
        /*caret*/
    }
}
"#
        .trim(),
    );
    fs::write(root.join("src/Main.sol"), &text).expect("write main");
    fs::write(
        root.join("src/Dep.sol"),
        r#"
contract Beta {
    uint256 numberB;
}
"#
        .trim(),
    )
    .expect("write dep");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(offset, &text);
    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: None,
    };
    let response = send_request(&mut service, 2, "textDocument/completion", params).await;
    let result =
        response_result::<Option<CompletionResponse>>(response).expect("completion response");

    let items = match result {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => list.items,
    };

    assert!(items.iter().any(|item| item.label == "numberA"));
    assert!(!items.iter().any(|item| item.label == "numberB"));
}

#[tokio::test]
async fn completion_includes_inherited_members() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offset) = extract_offset(
        r#"
contract Base { function ping() public {} uint256 public baseValue; }
contract Derived is Base { function derived() public {} }
contract Main {
    function test() public {
        Derived d;
        d.p/*caret*/;
    }
}
"#
        .trim(),
    );
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(offset, &text);
    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: None,
    };
    let response = send_request(&mut service, 2, "textDocument/completion", params).await;
    let result =
        response_result::<Option<CompletionResponse>>(response).expect("completion response");

    let items = match result {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => list.items,
    };

    assert!(items.iter().any(|item| item.label == "ping"));
}
