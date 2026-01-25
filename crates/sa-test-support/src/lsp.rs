use std::fs;
use std::path::Path;

use futures::{SinkExt, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, timeout};
use tower::{Service, ServiceExt};
use tower_lsp::jsonrpc::{Request, Response};
use tower_lsp::lsp_types::{
    ClientCapabilities, ConfigurationParams, InitializeParams, InitializedParams, NumberOrString,
    PublishDiagnosticsParams, Url, WorkspaceFolder,
};
use tower_lsp::{Client, LanguageServer, LspService, LspServiceBuilder};

pub async fn send_request<S, P>(
    service: &mut LspService<S>,
    id: i64,
    method: &'static str,
    params: P,
) -> Response
where
    S: tower_lsp::LanguageServer,
    P: Serialize,
{
    let request = Request::build(method)
        .id(id)
        .params(serde_json::to_value(params).expect("serialize request params"))
        .finish();
    let response = service
        .ready()
        .await
        .expect("service ready")
        .call(request)
        .await
        .expect("service call");
    response.expect("response")
}

pub async fn send_notification<S, P>(service: &mut LspService<S>, method: &'static str, params: P)
where
    S: tower_lsp::LanguageServer,
    P: Serialize,
{
    let request = Request::build(method)
        .params(serde_json::to_value(params).expect("serialize notification params"))
        .finish();
    let response = service
        .ready()
        .await
        .expect("service ready")
        .call(request)
        .await
        .expect("service call");
    assert!(
        response.is_none(),
        "notification should not return a response"
    );
}

pub async fn drain_startup_messages(socket: &mut tower_lsp::ClientSocket) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        let request = match timeout(remaining, socket.next()).await {
            Ok(Some(request)) => request,
            Ok(None) | Err(_) => return,
        };
        respond_to_request(socket, &request).await;
    }
}

pub async fn wait_for_publish<F>(
    socket: &mut tower_lsp::ClientSocket,
    timeout_duration: Duration,
    uri: &Url,
    predicate: F,
) -> PublishDiagnosticsParams
where
    F: Fn(&PublishDiagnosticsParams) -> bool,
{
    let message = format!("no publishDiagnostics received matching uri {}", uri);
    timeout(timeout_duration, async {
        loop {
            let request = socket.next().await?;
            if request.method() != "textDocument/publishDiagnostics" {
                respond_to_request(socket, &request).await;
                continue;
            }
            let params = request.params().cloned().expect("diagnostics params");
            let publish: PublishDiagnosticsParams =
                serde_json::from_value(params).expect("publish diagnostics params");
            if publish.uri == *uri && predicate(&publish) {
                return Some(publish);
            }
        }
    })
    .await
    .expect("timed out waiting for textDocument/publishDiagnostics")
    .expect(&message)
}

pub fn diagnostic_codes(publish: &PublishDiagnosticsParams) -> Vec<String> {
    publish
        .diagnostics
        .iter()
        .filter_map(|diag| {
            diag.code.as_ref().map(|code| match code {
                NumberOrString::Number(value) => value.to_string(),
                NumberOrString::String(value) => value.clone(),
            })
        })
        .collect()
}

pub async fn respond_to_request(socket: &mut tower_lsp::ClientSocket, request: &Request) {
    let Some(id) = request.id().cloned() else {
        return;
    };
    let response = Response::from_ok(id, response_value_for_request(request));
    socket.send(response).await.expect("send response");
}

fn response_value_for_request(request: &Request) -> Value {
    if request.method() == "workspace/configuration" {
        configuration_response(request.params())
    } else {
        Value::Null
    }
}

fn configuration_response(params: Option<&Value>) -> Value {
    let items_len = params
        .and_then(|value| serde_json::from_value::<ConfigurationParams>(value.clone()).ok())
        .map(|params| params.items.len())
        .unwrap_or(0);
    Value::Array(std::iter::repeat_n(Value::Null, items_len).collect())
}

pub fn response_result<T: DeserializeOwned>(response: Response) -> T {
    let (_, result) = response.into_parts();
    let value = result.expect("response ok");
    serde_json::from_value(value).expect("deserialize response")
}

pub fn make_server<S, F>(init: F) -> impl FnOnce() -> (LspService<S>, tower_lsp::ClientSocket)
where
    S: tower_lsp::LanguageServer,
    F: FnOnce(Client) -> S,
{
    move || LspService::new(init)
}

/// Creates a basic Foundry workspace directory structure for testing.
///
/// This creates the standard directories (src, lib, test, script) and a
/// minimal foundry.toml configuration file.
pub fn create_foundry_workspace(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::create_dir_all(root.join("lib")).expect("lib dir");
    fs::create_dir_all(root.join("test")).expect("test dir");
    fs::create_dir_all(root.join("script")).expect("script dir");

    let foundry_toml = r#"
[profile.default]
remappings = ["lib/=lib/forge-std/src/"]
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");
}

/// Sets up an LSP service with initialization for testing.
///
/// This creates the service, sends the initialize request, and sends the
/// initialized notification. Returns the ready-to-use service and root URI.
///
/// # Panics
///
/// Panics if the server returns an invalid InitializeResult or if the
/// server_capabilities field is missing.
pub async fn setup_lsp_service<S, F>(root: &Path, make_service: F) -> (LspService<S>, Url)
where
    S: tower_lsp::LanguageServer,
    F: FnOnce() -> (LspService<S>, tower_lsp::ClientSocket),
{
    let root_uri = Url::from_file_path(root).expect("root uri");
    let (mut service, _socket) = make_service();
    let initialize = InitializeParams {
        root_uri: Some(root_uri.clone()),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let init_result = response_result::<tower_lsp::lsp_types::InitializeResult>(response);

    // Validate that the server returned valid capabilities
    assert!(
        init_result.capabilities.text_document_sync.is_some()
            || init_result.capabilities.hover_provider.is_some()
            || init_result.capabilities.completion_provider.is_some()
            || init_result.capabilities.definition_provider.is_some(),
        "server should advertise at least one capability"
    );

    send_notification(&mut service, "initialized", InitializedParams {}).await;

    (service, root_uri)
}

pub struct LspTestHarness<S> {
    service: LspService<S>,
    outgoing: mpsc::Receiver<Request>,
    next_id: i64,
    background_task: Option<JoinHandle<()>>,
}

impl<S> LspTestHarness<S>
where
    S: LanguageServer,
{
    const OUTGOING_CAPACITY: usize = 128;

    pub async fn new(root: &Path, init: impl FnOnce(Client) -> S) -> Self {
        let root_uri = Url::from_file_path(root).expect("root uri");
        let params = InitializeParams {
            root_uri: Some(root_uri),
            capabilities: ClientCapabilities::default(),
            ..InitializeParams::default()
        };
        Self::new_with_params(params, init).await
    }

    pub async fn new_with_workspace_folders(
        folders: Vec<WorkspaceFolder>,
        init: impl FnOnce(Client) -> S,
    ) -> Self {
        let params = InitializeParams {
            workspace_folders: Some(folders),
            capabilities: ClientCapabilities::default(),
            ..InitializeParams::default()
        };
        Self::new_with_params(params, init).await
    }

    pub async fn new_with_params(params: InitializeParams, init: impl FnOnce(Client) -> S) -> Self {
        Self::new_with_params_and_builder(params, init, |builder| builder).await
    }

    pub async fn new_with_params_and_builder(
        params: InitializeParams,
        init: impl FnOnce(Client) -> S,
        customize: impl FnOnce(LspServiceBuilder<S>) -> LspServiceBuilder<S>,
    ) -> Self {
        let (service, socket) = customize(LspService::build(init)).finish();
        let (tx, rx) = mpsc::channel(Self::OUTGOING_CAPACITY);
        let background_task = tokio::spawn(async move {
            let mut socket = socket;
            while let Some(request) = socket.next().await {
                if tx.send(request).await.is_err() {
                    break;
                }
            }
        });

        let mut harness = Self {
            service,
            outgoing: rx,
            next_id: 1,
            background_task: Some(background_task),
        };

        let response = harness.send_request("initialize", params).await;
        let _result: tower_lsp::lsp_types::InitializeResult = Self::response_result(response);
        harness.notify("initialized", InitializedParams {}).await;

        harness
    }

    pub async fn request<P, R>(&mut self, method: &'static str, params: P) -> R
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let response = self.send_request(method, params).await;
        Self::response_result(response)
    }

    pub async fn notify<P>(&mut self, method: &'static str, params: P)
    where
        P: Serialize,
    {
        let request = Request::build(method)
            .params(serde_json::to_value(params).expect("serialize notification params"))
            .finish();
        let response = self
            .service
            .ready()
            .await
            .expect("service ready")
            .call(request)
            .await
            .expect("service call");
        assert!(
            response.is_none(),
            "notification should not return a response"
        );
    }

    pub async fn next_request(&mut self, timeout: Duration) -> Option<Request> {
        tokio::time::timeout(timeout, self.outgoing.recv())
            .await
            .ok()
            .flatten()
    }

    pub async fn wait_for_request(&mut self, method: &str, timeout: Duration) -> Option<Request> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let request = self.next_request(remaining).await?;
            if request.method() == method {
                return Some(request);
            }
        }
    }

    pub fn drain_requests(&mut self) -> Vec<Request> {
        let mut requests = Vec::new();
        while let Ok(request) = self.outgoing.try_recv() {
            requests.push(request);
        }
        requests
    }

    pub fn server(&self) -> &S {
        self.service.inner()
    }

    pub async fn spawn_request<P>(
        &mut self,
        method: &'static str,
        params: P,
    ) -> (i64, JoinHandle<Response>)
    where
        P: Serialize,
    {
        let request_id = self.next_id;
        let request = Request::build(method)
            .id(request_id)
            .params(serde_json::to_value(params).expect("serialize request params"))
            .finish();
        self.next_id += 1;

        let future = self
            .service
            .ready()
            .await
            .unwrap_or_else(|error| {
                panic!("service.ready() failed for LSP request '{method}' id={request_id}: {error}")
            })
            .call(request);
        let handle = tokio::spawn(async move {
            let response = future.await.unwrap_or_else(|error| {
                panic!("service.call() failed for LSP request '{method}' id={request_id}: {error}")
            });
            response.unwrap_or_else(|| {
                panic!("missing response for LSP request '{method}' id={request_id}")
            })
        });

        (request_id, handle)
    }

    async fn send_request<P>(&mut self, method: &'static str, params: P) -> Response
    where
        P: Serialize,
    {
        let request_id = self.next_id;
        let request = Request::build(method)
            .id(request_id)
            .params(serde_json::to_value(params).expect("serialize request params"))
            .finish();
        self.next_id += 1;

        let response = self
            .service
            .ready()
            .await
            .unwrap_or_else(|error| {
                panic!("service.ready() failed for LSP request '{method}' id={request_id}: {error}")
            })
            .call(request)
            .await
            .unwrap_or_else(|error| {
                panic!("service.call() failed for LSP request '{method}' id={request_id}: {error}")
            });
        response.unwrap_or_else(|| {
            panic!("missing response for LSP request '{method}' id={request_id}")
        })
    }

    fn response_result<T: DeserializeOwned>(response: Response) -> T {
        let (id, result) = response.into_parts();
        let value = result
            .unwrap_or_else(|error| panic!("response was error for method id {id:?}: {error:?}"));
        serde_json::from_value(value).unwrap_or_else(|error| {
            panic!("failed to deserialize response for method id {id:?}: {error}")
        })
    }
}

impl<S> Drop for LspTestHarness<S> {
    fn drop(&mut self) {
        if let Some(task) = self.background_task.take() {
            task.abort();
        }
    }
}
