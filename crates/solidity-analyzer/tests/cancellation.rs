use sa_span::TextSize;
use sa_span::lsp::to_lsp_position;
use sa_test_utils::FixtureBuilder;
use sa_test_utils::lsp::LspTestHarness;
use std::path::Path;
use tower_lsp::jsonrpc::{ErrorCode, Response};
use tower_lsp::lsp_types::{
    CancelParams, ClientCapabilities, DidOpenTextDocumentParams, Hover, HoverParams,
    InitializeParams, NumberOrString, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, Url,
};

async fn setup_cancellation_harness(root: &Path) -> LspTestHarness<solidity_analyzer::Server> {
    let root_uri = Url::from_file_path(root).expect("root uri");
    let params = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    LspTestHarness::new_with_params_and_builder(params, solidity_analyzer::Server::new, |builder| {
        builder.custom_method(
            "solidity-analyzer/slowRequest",
            solidity_analyzer::Server::slow_request,
        )
    })
    .await
}

async fn spawn_and_cancel_slow_request(
    harness: &mut LspTestHarness<solidity_analyzer::Server>,
    delay_ms: u64,
) -> Response {
    let (request_id, slow_task) = harness
        .spawn_request("solidity-analyzer/slowRequest", delay_ms)
        .await;

    let request_id = i32::try_from(request_id).expect("slow request id should fit in i32");
    harness
        .notify(
            "$/cancelRequest",
            CancelParams {
                id: NumberOrString::Number(request_id),
            },
        )
        .await;

    slow_task.await.expect("slow join")
}

#[tokio::test]
async fn cancel_request_returns_request_cancelled_and_keeps_server_alive() {
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .build()
        .expect("fixture");
    let mut harness = setup_cancellation_harness(fixture.root()).await;

    let response = spawn_and_cancel_slow_request(&mut harness, 300).await;
    assert!(response.is_error());
    let error = response.error().expect("cancel error");
    assert_eq!(error.code, ErrorCode::RequestCancelled);

    let _: () = harness.request("solidity-analyzer/slowRequest", 0u64).await;
}

#[tokio::test]
async fn cancelled_request_allows_followup_lsp_request() {
    let text = r#"
contract Main {
    function foo() public returns (uint256) {
        return 1;
    }
}
"#
    .trim();
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", text)
        .build()
        .expect("fixture");
    let file_path = fixture.root().join("src/Main.sol");
    let mut harness = setup_cancellation_harness(fixture.root()).await;

    let file_uri = Url::from_file_path(&file_path).expect("file uri");
    harness
        .notify(
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

    let response = spawn_and_cancel_slow_request(&mut harness, 300).await;
    assert!(response.is_error());
    let error = response.error().expect("cancel error");
    assert_eq!(error.code, ErrorCode::RequestCancelled);

    let offset = TextSize::from(text.find("foo").expect("foo offset") as u32);
    let position = to_lsp_position(offset, text);
    let hover_params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: file_uri },
            position,
        },
        work_done_progress_params: Default::default(),
    };
    let hover: Option<Hover> = harness.request("textDocument/hover", hover_params).await;
    assert!(hover.is_some());
}
