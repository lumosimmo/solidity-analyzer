use sa_paths::NormalizedPath;
use sa_span::{TextRange, TextSize, lsp::to_lsp_range};
use sa_test_support::lsp::{response_result, send_notification, send_request};
use tower_lsp::lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, InitializeParams, InitializedParams, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, Url, VersionedTextDocumentIdentifier,
};

#[tokio::test]
async fn document_sync_updates_vfs_and_analysis() {
    let (mut service, _socket) = tower_lsp::LspService::new(solidity_analyzer::Server::new);
    let initialize = InitializeParams {
        root_uri: Some(Url::parse("file:///workspace").expect("root uri")),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;

    let uri = Url::parse("file:///workspace/src/Main.sol").expect("file uri");
    let open_params = DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri: uri.clone(),
            language_id: "solidity".to_string(),
            version: 1,
            text: "contract Foo {}".to_string(),
        },
    };
    send_notification(&mut service, "textDocument/didOpen", open_params).await;

    let (analysis, vfs) = service.inner().snapshot().await;
    let vfs = vfs.expect("vfs snapshot after open");
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let file_id = vfs.file_id(&path).expect("file id");
    assert_eq!(analysis.file_text(file_id).as_ref(), "contract Foo {}");
    let first_version = analysis.file_version(file_id);
    drop(analysis);
    drop(vfs);

    let range = TextRange::at(TextSize::from(9), TextSize::from(3));
    let change = TextDocumentContentChangeEvent {
        range: Some(to_lsp_range(range, "contract Foo {}")),
        range_length: None,
        text: "Bar".to_string(),
    };
    let change_params = DidChangeTextDocumentParams {
        text_document: VersionedTextDocumentIdentifier {
            uri: uri.clone(),
            version: 2,
        },
        content_changes: vec![change],
    };
    send_notification(&mut service, "textDocument/didChange", change_params).await;

    let (analysis, vfs) = service.inner().snapshot().await;
    let vfs = vfs.expect("vfs snapshot after change");
    let file_id = vfs.file_id(&path).expect("file id");
    assert_eq!(analysis.file_text(file_id).as_ref(), "contract Bar {}");
    assert!(analysis.file_version(file_id) > first_version);
    drop(analysis);
    drop(vfs);

    let close_params = DidCloseTextDocumentParams {
        text_document: TextDocumentIdentifier { uri },
    };
    send_notification(&mut service, "textDocument/didClose", close_params).await;

    let (_analysis, vfs) = service.inner().snapshot().await;
    let vfs = vfs.expect("vfs snapshot after close");
    assert!(vfs.file_id(&path).is_none());
}
