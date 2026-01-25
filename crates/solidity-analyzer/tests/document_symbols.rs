use std::fs;

use sa_test_support::lsp::{
    create_foundry_workspace, response_result, send_notification, send_request, setup_lsp_service,
};
use tempfile::tempdir;
use tower_lsp::lsp_types::{
    DocumentSymbolParams, DocumentSymbolResponse, TextDocumentIdentifier, TextDocumentItem, Url,
};

#[tokio::test]
async fn document_symbols_return_nested_structure() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let text = r#"contract Foo {
    struct Bar { uint256 value; }
    function baz() external {}
}
"#;
    fs::write(root.join("src/Main.sol"), text).expect("write main");

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
                text: text.to_string(),
            },
        },
    )
    .await;

    let params = DocumentSymbolParams {
        text_document: TextDocumentIdentifier { uri: main_uri },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/documentSymbol", params).await;
    let result = response_result::<Option<DocumentSymbolResponse>>(response)
        .expect("document symbols response");

    let symbols = match result {
        DocumentSymbolResponse::Nested(symbols) => symbols,
        DocumentSymbolResponse::Flat(flat) => {
            panic!("unexpected flat symbols: {flat:?}");
        }
    };

    assert_eq!(symbols.len(), 1);
    let contract = &symbols[0];
    assert_eq!(contract.name, "Foo");
    assert_eq!(contract.children.as_ref().map(Vec::len), Some(2));
}
