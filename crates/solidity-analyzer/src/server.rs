use std::collections::HashMap;
use std::env;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result as AnyhowResult;
use futures::future::{AbortHandle, Abortable};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::task;
use tower_lsp::jsonrpc::{Error, ErrorCode, Result};
use tower_lsp::lsp_types::request::Request;
use tower_lsp::lsp_types::{
    CodeActionOrCommand, CodeActionParams, CodeActionProviderCapability, CompletionOptions,
    CompletionParams, CompletionResponse, DidChangeConfigurationParams,
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, DocumentFormattingParams,
    DocumentSymbolParams, DocumentSymbolResponse, ExecuteCommandOptions, ExecuteCommandParams,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverParams, InitializeParams,
    InitializeResult, InitializedParams, Location, MessageActionItem, MessageType, OneOf,
    ReferenceParams, RenameParams, ServerCapabilities, SignatureHelp, SignatureHelpOptions,
    SignatureHelpParams, SymbolInformation, TextDocumentSyncCapability, TextDocumentSyncKind,
    WorkspaceEdit, WorkspaceFoldersServerCapabilities, WorkspaceServerCapabilities,
    WorkspaceSymbolParams, request,
};
use tower_lsp::{Client, LanguageServer};
use tracing::{debug, error, info_span, warn};

use crate::config;
use crate::diagnostics::Diagnostics;
use crate::document;
use crate::handlers;
use crate::lsp_utils;
use crate::profile;
use crate::state::ServerState;
use crate::status;
use crate::task_pool::TaskPool;
use crate::workspace;
use sa_config::ResolvedFoundryConfig;
use sa_toolchain::{Toolchain, is_svm_installed};

const PROFILE_METHOD_SLOW_REQUEST: &str = "solidity-analyzer/slowRequest";
const METHOD_GOTO_DEFINITION: &str = request::GotoDefinition::METHOD;
const METHOD_HOVER: &str = request::HoverRequest::METHOD;
const METHOD_SIGNATURE_HELP: &str = request::SignatureHelpRequest::METHOD;
const METHOD_COMPLETION: &str = request::Completion::METHOD;
const METHOD_FORMATTING: &str = request::Formatting::METHOD;
const METHOD_CODE_ACTION: &str = request::CodeActionRequest::METHOD;
const METHOD_REFERENCES: &str = request::References::METHOD;
const METHOD_RENAME: &str = request::Rename::METHOD;
const METHOD_DOCUMENT_SYMBOL: &str = request::DocumentSymbolRequest::METHOD;
const METHOD_WORKSPACE_SYMBOL: &str = request::WorkspaceSymbolRequest::METHOD;
const COMMAND_INSTALL_FOUNDRY_SOLC: &str = "solidity-analyzer.installFoundrySolc";
const COMMAND_LIST_INDEXED_FILES: &str = "solidity-analyzer.indexedFiles";
const ERROR_SERVER_NOT_INITIALIZED: i64 = -32002;

pub struct Server {
    client: Client,
    state: Arc<Mutex<ServerState>>,
    task_pool: TaskPool,
    diagnostics: Diagnostics,
}

impl Server {
    pub fn new(client: Client) -> Self {
        // Keep profiling init here so tests/harnesses enable SA_PROFILE_PATH without init_tracing.
        // Safe to call repeatedly because init_from_env is idempotent.
        profile::init_from_env();
        let state = Arc::new(Mutex::new(ServerState::new()));
        let diagnostics = Diagnostics::new(client.clone(), Arc::clone(&state));
        Self {
            client,
            state,
            task_pool: TaskPool::new(),
            diagnostics,
        }
    }

    pub async fn snapshot(&self) -> (sa_ide::Analysis, Option<sa_vfs::VfsSnapshot>) {
        let state = self.state.lock().await;
        (state.analysis_host.snapshot(), state.vfs_snapshot.clone())
    }

    pub async fn slow_request(&self, delay_ms: u64) -> Result<()> {
        let task = self.task_pool.spawn(move || {
            let _profile = profile::ProfileSpan::new(PROFILE_METHOD_SLOW_REQUEST);
            std::thread::sleep(Duration::from_millis(delay_ms));
        });
        match task.await {
            Ok(()) => Ok(()),
            Err(error) if error.is_cancelled() => Err(Error::request_cancelled()),
            Err(_) => Err(Error::internal_error()),
        }
    }

    /// Runs a handler function with the standard snapshot/VFS/cancellation pattern.
    ///
    /// This helper encapsulates the common pattern for request handlers:
    /// 1. Takes a snapshot of the analysis and VFS
    /// 2. Returns Ok(None) if VFS is not available
    /// 3. Spawns the handler on the task pool with cancellation support
    /// 4. Maps results to appropriate JSON-RPC responses
    async fn run_handler<T, F>(&self, method: &'static str, handler: F) -> Result<Option<T>>
    where
        T: Send + 'static,
        F: FnOnce(&sa_ide::Analysis, &sa_vfs::VfsSnapshot) -> Option<T> + Send + 'static,
    {
        let (analysis, vfs) = self.snapshot().await;
        let Some(vfs) = vfs else {
            return Ok(None);
        };
        let task = self.task_pool.spawn(move || {
            let _profile = profile::ProfileSpan::new(method);
            let span = info_span!("lsp_request", method = %method);
            span.in_scope(|| salsa::Cancelled::catch(AssertUnwindSafe(|| handler(&analysis, &vfs))))
        });
        match task.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => Err(Error::request_cancelled()),
            Err(error) if error.is_cancelled() => Err(Error::request_cancelled()),
            Err(_) => Err(Error::internal_error()),
        }
    }

    fn capabilities() -> ServerCapabilities {
        ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(
                TextDocumentSyncKind::INCREMENTAL,
            )),
            definition_provider: Some(OneOf::Left(true)),
            hover_provider: Some(tower_lsp::lsp_types::HoverProviderCapability::Simple(true)),
            signature_help_provider: Some(SignatureHelpOptions {
                trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                retrigger_characters: None,
                work_done_progress_options: Default::default(),
            }),
            completion_provider: Some(CompletionOptions {
                trigger_characters: Some(vec![".".to_string(), "\"".to_string(), "/".to_string()]),
                all_commit_characters: None,
                resolve_provider: Some(false),
                completion_item: None,
                work_done_progress_options: Default::default(),
            }),
            code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
            references_provider: Some(OneOf::Left(true)),
            rename_provider: Some(OneOf::Left(true)),
            document_formatting_provider: Some(OneOf::Left(true)),
            document_symbol_provider: Some(OneOf::Left(true)),
            workspace_symbol_provider: Some(OneOf::Left(true)),
            workspace: Some(WorkspaceServerCapabilities {
                workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                    supported: Some(true),
                    change_notifications: Some(OneOf::Left(true)),
                }),
                file_operations: None,
            }),
            execute_command_provider: Some(ExecuteCommandOptions {
                commands: vec![
                    COMMAND_INSTALL_FOUNDRY_SOLC.to_string(),
                    COMMAND_LIST_INDEXED_FILES.to_string(),
                ],
                work_done_progress_options: Default::default(),
            }),
            ..ServerCapabilities::default()
        }
    }

    fn log_status_for_config(&self, config: Option<ResolvedFoundryConfig>) {
        let Some(config) = config else {
            return;
        };
        let client = self.client.clone();
        tokio::spawn(async move {
            let message = match task::spawn_blocking(move || status::startup_status(&config)).await
            {
                Ok(message) => message,
                Err(error) => {
                    warn!(?error, "failed to build startup status");
                    return;
                }
            };
            client.log_message(MessageType::INFO, message).await;
        });
    }

    async fn install_foundry_solc(&self) -> Result<String> {
        let config = { self.state.lock().await.config.clone() };
        let Some(config) = config else {
            return Err(Error {
                code: ErrorCode::ServerError(ERROR_SERVER_NOT_INITIALIZED),
                message: "workspace configuration unavailable; server not initialized".into(),
                data: None,
            });
        };

        Self::install_foundry_solc_with_config(config).await
    }

    async fn install_foundry_solc_with_config(config: ResolvedFoundryConfig) -> Result<String> {
        if let Some(message) = test_solc_install_message() {
            return Ok(message);
        }

        let toolchain = Toolchain::new(config);
        let result = task::spawn_blocking(move || toolchain.install_solc()).await;
        match result {
            Ok(Ok(message)) => Ok(message),
            Ok(Err(error)) => Err(Error {
                code: ErrorCode::InternalError,
                message: format!("solc install failed: {error}").into(),
                data: None,
            }),
            Err(error) => Err(Error {
                code: ErrorCode::InternalError,
                message: format!("solc install task failed: {error}").into(),
                data: None,
            }),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Server {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut state = self.state.lock().await;
        if let Some(settings) = params.initialization_options.clone() {
            state.lsp_config = config::LspConfig::from_settings(settings);
        }
        state.supports_server_status = params
            .capabilities
            .experimental
            .as_ref()
            .and_then(|value| value.get("serverStatusNotification"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let root_path = params
            .workspace_folders
            .as_ref()
            .and_then(|folders| folders.first())
            .and_then(|folder| lsp_utils::url_to_path(&folder.uri))
            .or_else(|| params.root_uri.as_ref().and_then(lsp_utils::url_to_path));
        let mut discovered_root = root_path.as_ref().and_then(|root| {
            let path = Path::new(root.as_str());
            if lsp_utils::contains_foundry_config(path) {
                Some(root.clone())
            } else {
                state.discover_foundry_root(root)
            }
        });
        if discovered_root.is_none()
            && let Ok(cwd) = env::current_dir()
        {
            let cwd = lsp_utils::normalize_path(&cwd);
            discovered_root = state.discover_foundry_root(&cwd);
        }
        state.root_path = discovered_root.clone().or(root_path.clone());
        if let Some(root) = discovered_root.as_ref()
            && let Err(error) = workspace::load(&mut state, root, None)
        {
            warn!(?error, root = %root, "failed to load foundry workspace");
        }
        let result = InitializeResult {
            capabilities: Self::capabilities(),
            server_info: None,
        };
        drop(state);
        Ok(result)
    }

    async fn initialized(&self, _: InitializedParams) {
        let status_config = { self.state.lock().await.config.clone() };
        self.log_status_for_config(status_config);
        self.diagnostics.publish_status().await;

        let client = self.client.clone();
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            prompt_install_solc(client, state).await;
        });
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let lsp_config = {
            let mut state = self.state.lock().await;
            document::did_open(&mut state, params);
            state.lsp_config.clone()
        };
        if lsp_config.diagnostics.enable && lsp_config.diagnostics.on_change {
            self.diagnostics.did_change(&uri).await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let lsp_config = {
            let mut state = self.state.lock().await;
            document::did_change(&mut state, params);
            state.lsp_config.clone()
        };
        if lsp_config.diagnostics.enable && lsp_config.diagnostics.on_change {
            self.diagnostics.did_change(&uri).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut state = self.state.lock().await;
        document::did_close(&mut state, params);
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let (analysis, vfs, config, lsp_config) = {
            let mut state = self.state.lock().await;
            document::did_save(&mut state, params);
            (
                state.analysis_host.snapshot(),
                state.vfs_snapshot.clone(),
                state.config.clone(),
                state.lsp_config.clone(),
            )
        };

        let run_solc = lsp_config.diagnostics.enable && lsp_config.diagnostics.on_save;
        let run_solar = lsp_config.lint.enable && lsp_config.lint.on_save;
        self.diagnostics.did_save(&uri, run_solc, run_solar).await;

        if lsp_config.format.on_save {
            let (Some(config), Some(vfs)) = (config, vfs) else {
                return;
            };
            let Some(path) = lsp_utils::url_to_path(&uri) else {
                return;
            };

            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let generation = {
                let mut state = self.state.lock().await;
                state.format_tasks.register(path.clone(), abort_handle)
            };
            let client = self.client.clone();
            let task_pool = self.task_pool.clone();
            let state = Arc::clone(&self.state);
            tokio::spawn(async move {
                let uri_for_log = uri.clone();
                let state_for_task = Arc::clone(&state);
                let path_for_task = path.clone();
                let format_result = Abortable::new(
                    async move {
                        let uri_for_task = uri.clone();
                        let task = task_pool.spawn(move || {
                            handlers::did_save::format_on_save(
                                &analysis,
                                &vfs,
                                &uri_for_task,
                                &config,
                            )
                        });

                        let mut edit = None;
                        match task.await {
                            Ok(Some(result)) => {
                                edit = Some(result);
                            }
                            Ok(None) => {
                                debug!(%uri, "format-on-save: no edits returned");
                            }
                            Err(join_err) => {
                                error!(%uri, ?join_err, "format-on-save: task failed");
                            }
                        }

                        let is_current = {
                            let state = state_for_task.lock().await;
                            state.format_tasks.is_current(&path_for_task, generation)
                        };
                        if !is_current {
                            debug!(%uri, "format-on-save: stale result ignored");
                            return;
                        }

                        if let Some(edit) = edit
                            && let Err(error) = client.apply_edit(edit).await
                        {
                            client
                                .log_message(
                                    MessageType::ERROR,
                                    format!(
                                        "format-on-save: failed to apply edit for {uri}: {error}"
                                    ),
                                )
                                .await;
                        }
                    },
                    abort_registration,
                )
                .await;

                if format_result.is_err() {
                    debug!(%uri_for_log, "format-on-save: task aborted");
                }

                let mut state = state.lock().await;
                state.format_tasks.finish(&path, generation);
            });
        }
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        let mut state = self.state.lock().await;
        state.lsp_config = config::LspConfig::from_settings(params.settings);
        if let Err(error) = workspace::reload(&mut state) {
            warn!(?error, "failed to reload foundry workspace");
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let should_reload = params.changes.iter().any(|change| {
            lsp_utils::url_to_path(&change.uri)
                .is_some_and(|path| lsp_utils::is_foundry_config_path(&path))
        });
        if should_reload {
            let mut state = self.state.lock().await;
            if let Err(error) = workspace::reload(&mut state) {
                warn!(?error, "failed to reload foundry workspace");
            }
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        self.run_handler(METHOD_GOTO_DEFINITION, move |analysis, vfs| {
            handlers::definition::goto_definition(analysis, vfs, params)
        })
        .await
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        self.run_handler(METHOD_HOVER, move |analysis, vfs| {
            handlers::hover::hover(analysis, vfs, params)
        })
        .await
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        self.run_handler(METHOD_SIGNATURE_HELP, move |analysis, vfs| {
            handlers::signature_help::signature_help(analysis, vfs, params)
        })
        .await
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        self.run_handler(METHOD_COMPLETION, move |analysis, vfs| {
            handlers::completion::completion(analysis, vfs, params)
        })
        .await
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> Result<Option<Vec<tower_lsp::lsp_types::TextEdit>>> {
        let config = { self.state.lock().await.config.clone() };
        self.run_handler(METHOD_FORMATTING, move |analysis, vfs| {
            handlers::formatting::formatting(analysis, vfs, params, config)
        })
        .await
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<Vec<CodeActionOrCommand>>> {
        self.run_handler(METHOD_CODE_ACTION, move |analysis, vfs| {
            handlers::code_action::code_action(analysis, vfs, params)
        })
        .await
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        match params.command.as_str() {
            COMMAND_INSTALL_FOUNDRY_SOLC => {
                let message = self.install_foundry_solc().await?;
                Ok(Some(Value::String(message)))
            }
            COMMAND_LIST_INDEXED_FILES => {
                let state = self.state.lock().await;
                let mut paths = state
                    .indexed_files
                    .iter()
                    .map(|path| path.as_str().to_string())
                    .collect::<Vec<_>>();
                paths.sort();
                Ok(Some(Value::Array(
                    paths.into_iter().map(Value::String).collect(),
                )))
            }
            _ => Ok(None),
        }
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        self.run_handler(METHOD_REFERENCES, move |analysis, vfs| {
            handlers::references::references(analysis, vfs, params)
        })
        .await
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        self.run_handler(METHOD_RENAME, move |analysis, vfs| {
            handlers::rename::rename(analysis, vfs, params)
        })
        .await
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        self.run_handler(METHOD_DOCUMENT_SYMBOL, move |analysis, vfs| {
            handlers::document_symbols::document_symbols(analysis, vfs, params)
        })
        .await
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        self.run_handler(METHOD_WORKSPACE_SYMBOL, move |analysis, vfs| {
            handlers::workspace_symbols::workspace_symbols(analysis, vfs, params)
        })
        .await
    }
}

async fn prompt_install_solc(client: Client, state: Arc<Mutex<ServerState>>) {
    let (config, lsp_config, already_prompted) = {
        let state = state.lock().await;
        (
            state.config.clone(),
            state.lsp_config.clone(),
            state.prompted_solc_install,
        )
    };

    if already_prompted || !lsp_config.toolchain.prompt_install {
        return;
    }

    let Some(config) = config else {
        return;
    };

    let config_for_check = config.clone();
    let prompt =
        match task::spawn_blocking(move || detect_missing_solc_prompt(config_for_check)).await {
            Ok(Ok(prompt)) => prompt,
            Ok(Err(error)) => {
                warn!(?error, "failed to detect missing solc");
                return;
            }
            Err(error) => {
                warn!(?error, "solc prompt task failed");
                return;
            }
        };

    let Some(message) = prompt else {
        return;
    };

    {
        let mut state = state.lock().await;
        if state.prompted_solc_install {
            return;
        }
        state.prompted_solc_install = true;
    }

    let actions = vec![
        MessageActionItem {
            title: "Install".to_string(),
            properties: HashMap::new(),
        },
        MessageActionItem {
            title: "Dismiss".to_string(),
            properties: HashMap::new(),
        },
    ];
    let response = client
        .show_message_request(MessageType::INFO, message, Some(actions))
        .await;
    let action = match response {
        Ok(Some(action)) => action,
        Ok(None) => return,
        Err(error) => {
            warn!(?error, "failed to request solc install prompt");
            return;
        }
    };

    if action.title != "Install" {
        return;
    }

    match Server::install_foundry_solc_with_config(config).await {
        Ok(message) => {
            if !message.is_empty() {
                client.show_message(MessageType::INFO, message).await;
            }
        }
        Err(error) => {
            client.show_message(MessageType::ERROR, error.message).await;
        }
    }
}

fn detect_missing_solc_prompt(config: ResolvedFoundryConfig) -> AnyhowResult<Option<String>> {
    if test_solc_install_message().is_some() {
        return Ok(Some(
            "solc is missing for this workspace. Install with Foundry?".to_string(),
        ));
    }

    let toolchain = Toolchain::new(config);
    if let Some(spec) = toolchain.solc_spec() {
        if toolchain.solc_spec_is_path() {
            return Ok(None);
        }

        return match toolchain.resolve() {
            Ok(_) => Ok(None),
            Err(_) => Ok(Some(format!(
                "solc {spec} is not installed. Install with Foundry?"
            ))),
        };
    }

    let versions = toolchain.auto_detect_versions()?;
    if versions.is_empty() {
        return Ok(None);
    }

    let mut missing = Vec::new();
    for version in versions {
        match is_svm_installed(&version) {
            Ok(true) => {}
            Ok(false) => missing.push(version.to_string()),
            Err(error) => {
                warn!(?error, %version, "failed to check solc install status");
                missing.push(version.to_string());
            }
        }
    }

    if missing.is_empty() {
        return Ok(None);
    }

    let message = if missing.len() == 1 {
        format!(
            "solc {} is not installed. Install with Foundry?",
            missing[0]
        )
    } else {
        format!(
            "solc versions {} are not installed. Install with Foundry?",
            missing.join(", ")
        )
    };

    Ok(Some(message))
}

fn test_solc_install_message() -> Option<String> {
    env::var("SA_TEST_SOLC_INSTALL_MESSAGE")
        .ok()
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sa_paths::NormalizedPath;
    use sa_project_model::{FoundryProfile, FoundryWorkspace};
    use sa_test_support::setup_foundry_root;
    use sa_test_support::write_stub_solc;
    use sa_test_utils::{EnvGuard, env_lock};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::tempdir;
    use tower_lsp::LspService;

    #[tokio::test]
    async fn run_handler_returns_none_without_vfs() {
        let (service, _socket) = LspService::new(Server::new);
        let server = service.inner();
        let called = Arc::new(AtomicBool::new(false));
        let called_for_handler = Arc::clone(&called);

        let result: Result<Option<()>> = server
            .run_handler("test", move |_analysis, _vfs| {
                called_for_handler.store(true, Ordering::SeqCst);
                Some(())
            })
            .await;

        assert!(result.expect("run handler result").is_none());
        assert!(!called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn install_foundry_solc_errors_without_config() {
        let (service, _socket) = LspService::new(Server::new);
        let server = service.inner();

        let error = server
            .install_foundry_solc()
            .await
            .expect_err("expected install error");
        assert_eq!(
            error.code,
            ErrorCode::ServerError(ERROR_SERVER_NOT_INITIALIZED)
        );
    }

    #[test]
    fn detect_missing_solc_prompt_skips_path_spec() {
        let _lock = env_lock();
        let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
        let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);
        let _env_stub = EnvGuard::set("SA_TEST_SOLC_INSTALL_MESSAGE", None);

        let temp = tempdir().expect("tempdir");
        setup_foundry_root(temp.path());
        let solc_path = write_stub_solc(temp.path(), "0.8.20", None);
        let root = NormalizedPath::new(temp.path().to_string_lossy());
        let profile = FoundryProfile::new("default").with_solc_version(solc_path.to_string_lossy());
        let workspace = FoundryWorkspace::new(root);
        let config = ResolvedFoundryConfig::new(workspace, profile);

        let prompt = detect_missing_solc_prompt(config).expect("prompt result");
        assert!(prompt.is_none());
    }

    #[test]
    fn detect_missing_solc_prompt_reports_missing_spec() {
        let _lock = env_lock();
        let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
        let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);
        let _env_stub = EnvGuard::set("SA_TEST_SOLC_INSTALL_MESSAGE", None);

        let root = NormalizedPath::new("/workspace");
        let profile = FoundryProfile::new("default").with_solc_version("99.99.99");
        let workspace = FoundryWorkspace::new(root);
        let config = ResolvedFoundryConfig::new(workspace, profile);

        let prompt = detect_missing_solc_prompt(config).expect("prompt result");
        let message = prompt.expect("missing solc prompt");
        assert!(message.contains("99.99.99"));
    }

    #[test]
    fn detect_missing_solc_prompt_auto_detects_single_missing_version() {
        let _lock = env_lock();
        let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
        let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);
        let _env_stub = EnvGuard::set("SA_TEST_SOLC_INSTALL_MESSAGE", None);

        let svm_root = tempdir().expect("svm tempdir");
        let data_dir = svm_root.path().join(".local/share");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        let home = svm_root.path().to_string_lossy().to_string();
        let data = data_dir.to_string_lossy().to_string();
        let _env_home = EnvGuard::set("HOME", Some(home.as_str()));
        let _env_data = EnvGuard::set("XDG_DATA_HOME", Some(data.as_str()));

        let temp = tempdir().expect("tempdir");
        setup_foundry_root(temp.path());
        std::fs::write(
            temp.path().join("src/Main.sol"),
            "pragma solidity =0.8.20;\n\ncontract Main {}\n",
        )
        .expect("write Main.sol");

        let root = NormalizedPath::new(temp.path().to_string_lossy());
        let default_profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root);
        let config = ResolvedFoundryConfig::new(workspace, default_profile);

        let prompt = detect_missing_solc_prompt(config).expect("prompt result");
        let message = prompt.expect("missing solc prompt");
        assert!(message.contains("solc 0.8.20"));
    }
}
