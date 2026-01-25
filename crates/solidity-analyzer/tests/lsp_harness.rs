use sa_paths::NormalizedPath;
use sa_span::TextSize;
use sa_span::lsp::to_lsp_position;
use sa_test_utils::FixtureBuilder;
use sa_test_utils::lsp::LspTestHarness;
use tower_lsp::lsp_types::{
    DidOpenTextDocumentParams, Hover, HoverParams, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, Url, WorkspaceFolder,
};

#[tokio::test]
async fn harness_handles_request_response_flow() {
    let text = r#"
contract Main {
    function foo() public returns (uint256) {
        return 1;
    }
}
"#
    .trim();
    let foundry_toml = r#"
[profile.default]
remappings = ["lib/=lib/forge-std/src/", "src/=src/overrides/"]
"#;
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(foundry_toml)
        .file("src/Main.sol", text)
        .build()
        .expect("fixture");

    let mut harness = LspTestHarness::new(fixture.root(), solidity_analyzer::Server::new).await;
    let main_uri = Url::from_file_path(fixture.root().join("src/Main.sol")).expect("main uri");

    harness
        .notify(
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

    let offset = TextSize::from(text.find("foo").expect("foo offset") as u32);
    let position = to_lsp_position(offset, text);
    let hover_params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
    };

    let hover: Option<Hover> = harness.request("textDocument/hover", hover_params).await;
    assert!(hover.is_some());
}

#[tokio::test]
async fn harness_uses_first_workspace_folder() {
    let fixture_a = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Main.sol",
            r#"
contract Alpha {}
"#,
        )
        .build()
        .expect("fixture a");
    let fixture_b = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Main.sol",
            r#"
contract Beta {}
"#,
        )
        .build()
        .expect("fixture b");

    let root_a = Url::from_file_path(fixture_a.root()).expect("root a uri");
    let root_b = Url::from_file_path(fixture_b.root()).expect("root b uri");
    let folders = vec![
        WorkspaceFolder {
            uri: root_a.clone(),
            name: "alpha".to_string(),
        },
        WorkspaceFolder {
            uri: root_b,
            name: "beta".to_string(),
        },
    ];

    let harness =
        LspTestHarness::new_with_workspace_folders(folders, solidity_analyzer::Server::new).await;
    let (analysis, _) = harness.server().snapshot().await;
    assert_eq!(
        analysis.workspace().root(),
        &NormalizedPath::new(fixture_a.root().to_string_lossy())
    );
}
