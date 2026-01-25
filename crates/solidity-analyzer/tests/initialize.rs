use sa_test_support::lsp::{response_result, send_request};
use tower_lsp::lsp_types::{
    ClientCapabilities, InitializeParams, OneOf, TextDocumentSyncCapability, TextDocumentSyncKind,
    Url, WorkspaceFoldersServerCapabilities,
};

#[tokio::test]
async fn initialize_advertises_basic_capabilities() {
    let (mut service, _socket) = tower_lsp::LspService::new(solidity_analyzer::Server::new);

    let params = InitializeParams {
        root_uri: Some(Url::parse("file:///workspace").expect("root uri")),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };

    let response = send_request(&mut service, 1, "initialize", params).await;
    let result = response_result::<tower_lsp::lsp_types::InitializeResult>(response);

    assert_eq!(
        result.capabilities.text_document_sync,
        Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL
        ))
    );
    let workspace = result
        .capabilities
        .workspace
        .expect("workspace capabilities");
    let folders = workspace
        .workspace_folders
        .as_ref()
        .expect("workspace folders support");
    assert_eq!(
        folders,
        &WorkspaceFoldersServerCapabilities {
            supported: Some(true),
            change_notifications: Some(OneOf::Left(true)),
        }
    );
    assert_eq!(workspace.file_operations, None);

    let execute = result
        .capabilities
        .execute_command_provider
        .expect("execute command provider");
    assert_eq!(
        execute.commands,
        vec![
            "solidity-analyzer.installFoundrySolc".to_string(),
            "solidity-analyzer.indexedFiles".to_string(),
        ]
    );
}
