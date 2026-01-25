use sa_span::lsp::to_lsp_range;
use sa_test_support::find_range;
use sa_test_utils::FixtureBuilder;
use sa_test_utils::lsp::LspTestHarness;
use tower_lsp::lsp_types::{
    CodeActionContext, CodeActionKind, CodeActionOrCommand, CodeActionParams, Diagnostic,
    DidOpenTextDocumentParams, NumberOrString, TextDocumentIdentifier, TextDocumentItem, Url,
};

#[tokio::test]
async fn code_action_returns_quick_fix() {
    let text = r#"
contract Main {
    uint256 FooBar;
}
"#
    .trim();
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
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

    let diagnostic = Diagnostic {
        range: to_lsp_range(find_range(text, "FooBar"), text),
        severity: None,
        code: Some(NumberOrString::String("mixed-case-variable".to_string())),
        code_description: None,
        source: None,
        message: "variables should use mixedCase".to_string(),
        related_information: None,
        tags: None,
        data: None,
    };

    let params = CodeActionParams {
        text_document: TextDocumentIdentifier {
            uri: main_uri.clone(),
        },
        range: diagnostic.range,
        context: CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: None,
            trigger_kind: None,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };

    let actions = harness
        .request::<_, Option<Vec<CodeActionOrCommand>>>("textDocument/codeAction", params)
        .await
        .unwrap_or_default();
    let action = actions
        .into_iter()
        .find_map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => Some(action),
            _ => None,
        })
        .expect("code action");

    assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));

    let edit = action.edit.expect("workspace edit");
    let changes = edit.changes.expect("changes");
    let edits = changes.get(&main_uri).expect("main edits");
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, "fooBar");
}
