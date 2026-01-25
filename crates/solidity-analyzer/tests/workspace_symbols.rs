use std::fs;

use sa_test_support::lsp::{
    create_foundry_workspace, response_result, send_notification, send_request, setup_lsp_service,
};
use tempfile::tempdir;
use tower_lsp::lsp_types::{
    SymbolInformation, SymbolKind, TextDocumentItem, Url, WorkspaceSymbolParams,
};

async fn workspace_symbol_query<S: tower_lsp::LanguageServer>(
    service: &mut tower_lsp::LspService<S>,
    id: i64,
    query: &str,
) -> Vec<SymbolInformation> {
    let params = WorkspaceSymbolParams {
        query: query.to_string(),
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(service, id, "workspace/symbol", params).await;
    response_result::<Option<Vec<SymbolInformation>>>(response).unwrap_or_default()
}

#[tokio::test]
async fn workspace_symbols_searches_project() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let main_text = r#"contract Main {
    event Ping(address indexed from);
    error Oops(uint256 code);
    modifier onlyOwner() {
        _;
    }
    uint256 value;
    struct Bar {
        uint256 inner;
    }
    enum State {
        On,
        Off
    }
    type UserId is uint256;
    function baz() external {}
}
"#;
    let lib_text = r#"contract Lib {}"#;

    fs::write(root.join("src/Main.sol"), main_text).expect("write main");
    fs::write(root.join("src/Lib.sol"), lib_text).expect("write lib");

    let (mut service, _root_uri) = setup_lsp_service(&root, || {
        tower_lsp::LspService::new(solidity_analyzer::Server::new)
    })
    .await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let lib_uri = Url::from_file_path(root.join("src/Lib.sol")).expect("lib uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: main_text.to_string(),
            },
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: lib_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: lib_text.to_string(),
            },
        },
    )
    .await;

    let mut request_id = 2;
    let result = workspace_symbol_query(&mut service, request_id, "Lib").await;
    request_id += 1;

    // Filter for symbols named "Lib"
    let lib_symbols: Vec<_> = result.iter().filter(|s| s.name == "Lib").collect();

    // Assert exactly one symbol named "Lib" exists
    assert_eq!(
        lib_symbols.len(),
        1,
        "expected exactly one symbol named 'Lib'"
    );

    // Verify the symbol properties
    let lib_symbol = lib_symbols[0];
    assert_eq!(
        lib_symbol.kind,
        SymbolKind::CLASS,
        "contract should be CLASS kind"
    );
    assert_eq!(
        lib_symbol.location.uri, lib_uri,
        "symbol should be located in Lib.sol"
    );

    let event_symbols = workspace_symbol_query(&mut service, request_id, "Ping").await;
    request_id += 1;
    let event_symbol = event_symbols
        .iter()
        .find(|symbol| symbol.name == "Ping")
        .expect("Ping symbol");
    assert_eq!(event_symbol.kind, SymbolKind::EVENT);
    assert_eq!(event_symbol.location.uri, main_uri);

    let error_symbols = workspace_symbol_query(&mut service, request_id, "Oops").await;
    request_id += 1;
    let error_symbol = error_symbols
        .iter()
        .find(|symbol| symbol.name == "Oops")
        .expect("Oops symbol");
    assert_eq!(error_symbol.kind, SymbolKind::CLASS);
    assert_eq!(error_symbol.location.uri, main_uri);

    let modifier_symbols = workspace_symbol_query(&mut service, request_id, "onlyOwner").await;
    request_id += 1;
    let modifier_symbol = modifier_symbols
        .iter()
        .find(|symbol| symbol.name == "onlyOwner")
        .expect("onlyOwner symbol");
    assert_eq!(modifier_symbol.kind, SymbolKind::METHOD);
    assert_eq!(modifier_symbol.location.uri, main_uri);

    let variable_symbols = workspace_symbol_query(&mut service, request_id, "value").await;
    request_id += 1;
    let variable_symbol = variable_symbols
        .iter()
        .find(|symbol| symbol.name == "value")
        .expect("value symbol");
    assert_eq!(variable_symbol.kind, SymbolKind::VARIABLE);
    assert_eq!(variable_symbol.location.uri, main_uri);

    let enum_symbols = workspace_symbol_query(&mut service, request_id, "State").await;
    request_id += 1;
    let enum_symbol = enum_symbols
        .iter()
        .find(|symbol| symbol.name == "State")
        .expect("State symbol");
    assert_eq!(enum_symbol.kind, SymbolKind::ENUM);
    assert_eq!(enum_symbol.location.uri, main_uri);

    let udvt_symbols = workspace_symbol_query(&mut service, request_id, "UserId").await;
    let udvt_symbol = udvt_symbols
        .iter()
        .find(|symbol| symbol.name == "UserId")
        .expect("UserId symbol");
    assert_eq!(udvt_symbol.kind, SymbolKind::TYPE_PARAMETER);
    assert_eq!(udvt_symbol.location.uri, main_uri);
}
