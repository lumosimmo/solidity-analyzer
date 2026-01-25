use sa_test_utils::FixtureBuilder;
use sa_test_utils::lsp::LspTestHarness;
use tower_lsp::lsp_types::ExecuteCommandParams;

#[tokio::test]
async fn indexed_files_command_returns_workspace_index() {
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Main.sol",
            r#"import "./Dep.sol";

contract Main {
    Dep dep;
}"#,
        )
        .file("src/Dep.sol", r#"contract Dep {}"#)
        .build()
        .expect("fixture");

    let mut harness = LspTestHarness::new(fixture.root(), solidity_analyzer::Server::new).await;
    let params = ExecuteCommandParams {
        command: "solidity-analyzer.indexedFiles".to_string(),
        arguments: Vec::new(),
        work_done_progress_params: Default::default(),
    };
    let result: Option<Vec<String>> = harness.request("workspace/executeCommand", params).await;
    let mut paths = result.expect("command result");
    paths.sort();

    let mut expected = vec![
        fixture
            .root()
            .join("src/Dep.sol")
            .to_string_lossy()
            .to_string(),
        fixture
            .root()
            .join("src/Main.sol")
            .to_string_lossy()
            .to_string(),
    ];
    expected.sort();

    assert_eq!(paths, expected);
}
