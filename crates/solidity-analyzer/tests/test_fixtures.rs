use sa_test_support::lsp::{
    drain_startup_messages, response_result, send_notification, send_request,
};
use sa_test_support::setup_foundry_root;
use sa_test_utils::toolchain::{
    SolcTestMode, StubSolcOptions, solc_path_for_tests_with_mode, stub_solc_output_error,
};
use std::fs;
use std::path::PathBuf;
use tempfile::{TempDir, tempdir};
use tower_lsp::lsp_types::{
    ClientCapabilities, DidOpenTextDocumentParams, DidSaveTextDocumentParams, InitializeParams,
    InitializedParams, TextDocumentIdentifier, TextDocumentItem, Url, WorkspaceClientCapabilities,
};
use tower_lsp::{ClientSocket, LspService};

pub struct DiagnosticsTestContext {
    _root_dir: TempDir,
    _solc_dir: TempDir,
    pub root: PathBuf,
    pub file_uri: Url,
    pub source: String,
}

impl DiagnosticsTestContext {
    pub fn new() -> Self {
        Self::with_solc_options(
            StubSolcOptions {
                json: Some(stub_solc_output_error()),
                sleep_seconds: None,
                capture_stdin: false,
            },
            lint_test_source(),
        )
    }

    pub fn with_solc_options(options: StubSolcOptions<'static>, source: String) -> Self {
        let root_dir = tempdir().expect("tempdir");
        let root = root_dir.path().canonicalize().expect("canonicalize root");
        setup_foundry_root(&root);

        let solc_dir = tempdir().expect("solc dir");
        let solc_path =
            solc_path_for_tests_with_mode(solc_dir.path(), "0.8.20", options, SolcTestMode::Stub);
        let solc_toml = solc_path
            .to_str()
            .expect("valid UTF-8 path")
            .replace('\\', "/");
        let foundry_toml = format!(
            r#"
[profile.default]
solc = "{solc}"
"#,
            solc = solc_toml
        );
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let file_path = root.join("src/Main.sol");
        fs::write(&file_path, &source).expect("write source");
        let canonical_file = file_path.canonicalize().expect("canonicalize file");
        let file_uri = Url::from_file_path(&canonical_file).expect("file uri");

        Self {
            _root_dir: root_dir,
            _solc_dir: solc_dir,
            root,
            file_uri,
            source,
        }
    }

    #[allow(dead_code)]
    pub async fn start_service(
        &self,
        initialization_options: Option<serde_json::Value>,
    ) -> (LspService<solidity_analyzer::Server>, ClientSocket) {
        let root_uri = Url::from_file_path(&self.root).expect("root uri");
        let (mut service, mut socket) = tower_lsp::LspService::new(solidity_analyzer::Server::new);
        let initialize = InitializeParams {
            root_uri: Some(root_uri),
            initialization_options,
            capabilities: ClientCapabilities {
                workspace: Some(WorkspaceClientCapabilities::default()),
                ..ClientCapabilities::default()
            },
            ..InitializeParams::default()
        };
        let response = send_request(&mut service, 1, "initialize", initialize).await;
        let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
        send_notification(&mut service, "initialized", InitializedParams {}).await;
        drain_startup_messages(&mut socket).await;

        (service, socket)
    }

    #[allow(dead_code)]
    pub async fn open(&self, service: &mut LspService<solidity_analyzer::Server>) {
        send_notification(
            service,
            "textDocument/didOpen",
            DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: self.file_uri.clone(),
                    language_id: "solidity".to_string(),
                    version: 1,
                    text: self.source.clone(),
                },
            },
        )
        .await;
    }

    #[allow(dead_code)]
    pub async fn save(&self, service: &mut LspService<solidity_analyzer::Server>) {
        send_notification(
            service,
            "textDocument/didSave",
            DidSaveTextDocumentParams {
                text_document: TextDocumentIdentifier {
                    uri: self.file_uri.clone(),
                },
                text: Some(self.source.clone()),
            },
        )
        .await;
    }

    #[allow(dead_code)]
    pub async fn open_and_save(&self, service: &mut LspService<solidity_analyzer::Server>) {
        self.open(service).await;
        self.save(service).await;
    }
}

impl Default for DiagnosticsTestContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns a Solidity snippet with a mixed-case `Bad_Name` to trigger lint checks.
pub fn lint_test_source() -> String {
    r#"
pragma solidity ^0.8.20;
contract LintTest {
    function Bad_Name() public {}
}
"#
    .to_string()
}
