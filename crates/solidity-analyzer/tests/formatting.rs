use std::cmp::Reverse;

use sa_span::lsp::from_lsp_range;
use sa_test_utils::FixtureBuilder;
use sa_test_utils::lsp::LspTestHarness;
use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem, Url,
};

fn apply_text_edits(text: &str, edits: &[tower_lsp::lsp_types::TextEdit]) -> String {
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

#[tokio::test]
async fn formatting_returns_stable_edits() {
    let foundry_toml = r#"
[profile.default]

[fmt]
style = "space"
tab_width = 2
"#;
    let text = r#"
contract Foo{function bar()public returns(uint256){return 1;}}
"#
    .trim();
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

    let params = DocumentFormattingParams {
        text_document: TextDocumentIdentifier {
            uri: main_uri.clone(),
        },
        options: Default::default(),
        work_done_progress_params: Default::default(),
    };
    let edits = harness
        .request::<_, Option<Vec<tower_lsp::lsp_types::TextEdit>>>(
            "textDocument/formatting",
            params,
        )
        .await
        .expect("textDocument/formatting returned null");

    let expected_formatted = r#"
contract Foo {
  function bar() public returns (uint256) {
    return 1;
  }
}
"#
    .trim();
    let formatted = apply_text_edits(text, &edits);
    assert_eq!(formatted.trim(), expected_formatted);

    harness
        .notify(
            "textDocument/didChange",
            DidChangeTextDocumentParams {
                text_document: tower_lsp::lsp_types::VersionedTextDocumentIdentifier {
                    uri: main_uri.clone(),
                    version: 2,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: formatted.clone(),
                }],
            },
        )
        .await;

    let params = DocumentFormattingParams {
        text_document: TextDocumentIdentifier { uri: main_uri },
        options: Default::default(),
        work_done_progress_params: Default::default(),
    };
    let edits = harness
        .request::<_, Option<Vec<tower_lsp::lsp_types::TextEdit>>>(
            "textDocument/formatting",
            params,
        )
        .await
        .expect("textDocument/formatting returned null");
    assert!(edits.is_empty());
}
