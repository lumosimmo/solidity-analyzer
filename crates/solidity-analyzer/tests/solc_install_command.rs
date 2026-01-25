use futures::{SinkExt, StreamExt};
use sa_test_support::lsp::{
    drain_startup_messages, make_server, respond_to_request, response_result, send_notification,
    send_request,
};
use sa_test_support::write_stub_solc;
use sa_test_utils::{EnvGuard, FixtureBuilder, env_lock};
use serde_json::Value;
use std::collections::HashMap;
use tempfile::tempdir;
use tower_lsp::jsonrpc::{Error, ErrorCode, Response};
use tower_lsp::lsp_types::{
    ClientCapabilities, ExecuteCommandParams, InitializeParams, InitializedParams,
    MessageActionItem, ShowMessageParams, ShowMessageRequestParams, Url,
};

fn response_error(response: Response) -> tower_lsp::jsonrpc::Error {
    let (_, result) = response.into_parts();
    result.expect_err("expected error response")
}

async fn wait_for_show_message_request(
    socket: &mut tower_lsp::ClientSocket,
) -> (tower_lsp::jsonrpc::Id, ShowMessageRequestParams) {
    loop {
        let request = socket.next().await.expect("client request");
        if request.method() == "window/showMessageRequest" {
            let id = request.id().cloned().expect("request id");
            let params = request.params().cloned().expect("request params");
            let params = serde_json::from_value::<ShowMessageRequestParams>(params)
                .expect("showMessage params");
            return (id, params);
        }
        respond_to_request(socket, &request).await;
    }
}

async fn wait_for_show_message(socket: &mut tower_lsp::ClientSocket) -> ShowMessageParams {
    loop {
        let request = socket.next().await.expect("client request");
        if request.method() == "window/showMessage" {
            let params = request.params().cloned().expect("message params");
            return serde_json::from_value::<ShowMessageParams>(params)
                .expect("showMessage params");
        }
        respond_to_request(socket, &request).await;
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn install_solc_command_accepts_local_solc_path() {
    let _lock = env_lock();
    let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
    let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);
    let _env_stub = EnvGuard::set("SA_TEST_SOLC_INSTALL_MESSAGE", None);

    let solc_dir = tempdir().expect("solc dir");
    let solc_path = write_stub_solc(solc_dir.path(), "0.8.20", None);
    let solc_path = solc_path.to_string_lossy().replace('\\', "\\\\");
    let foundry_toml = format!(
        r#"
[profile.default]
solc = "{solc_path}"
remappings = ["lib/=lib/forge-std/src/"]
"#
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(foundry_toml)
        .file(
            "src/Main.sol",
            r#"
contract Main {}
"#,
        )
        .build()
        .expect("fixture");

    let (mut service, mut socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;
    drain_startup_messages(&mut socket).await;

    let params = ExecuteCommandParams {
        command: "solidity-analyzer.installFoundrySolc".to_string(),
        arguments: Vec::new(),
        work_done_progress_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "workspace/executeCommand", params).await;
    let result = response_result::<Option<String>>(response);
    let message = result.expect("command result");

    assert!(message.starts_with("solc configured to local path "));
    assert!(message.contains("version 0.8.20"));
    assert!(!message.contains("auto-detect"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn install_solc_command_uses_auto_detect_without_explicit_solc() {
    let _lock = env_lock();
    let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
    let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .build()
        .expect("fixture");

    let (mut service, mut socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;
    drain_startup_messages(&mut socket).await;

    let params = ExecuteCommandParams {
        command: "solidity-analyzer.installFoundrySolc".to_string(),
        arguments: Vec::new(),
        work_done_progress_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "workspace/executeCommand", params).await;
    let error = response_error(response);

    assert!(error.message.contains("auto-detect"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn install_prompt_runs_solc_install() {
    let _lock = env_lock();
    let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
    let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);
    let _env_stub = EnvGuard::set("SA_TEST_SOLC_INSTALL_MESSAGE", Some("solc install stub"));

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Main.sol",
            r#"
pragma solidity ^0.8.0;

contract Main {}
"#,
        )
        .build()
        .expect("fixture");

    let (mut service, mut socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;

    let (request_id, params) = wait_for_show_message_request(&mut socket).await;
    assert!(params.message.contains("solc"));
    let action = params
        .actions
        .as_ref()
        .and_then(|actions| actions.iter().find(|action| action.title == "Install"))
        .cloned()
        .unwrap_or(MessageActionItem {
            title: "Install".to_string(),
            properties: HashMap::new(),
        });
    let response = Response::from_ok(
        request_id,
        serde_json::to_value(action).expect("serialize action"),
    );
    socket.send(response).await.expect("send response");

    let message = wait_for_show_message(&mut socket).await;
    assert!(message.message.contains("solc install stub"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn install_prompt_dismisses_request() {
    let _lock = env_lock();
    let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
    let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);
    let _env_stub = EnvGuard::set("SA_TEST_SOLC_INSTALL_MESSAGE", Some("solc install stub"));

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Main.sol",
            r#"
pragma solidity ^0.8.0;

contract Main {}
"#,
        )
        .build()
        .expect("fixture");

    let (mut service, mut socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;

    let (request_id, params) = wait_for_show_message_request(&mut socket).await;
    let action = params
        .actions
        .as_ref()
        .and_then(|actions| actions.iter().find(|action| action.title == "Dismiss"))
        .cloned()
        .unwrap_or(MessageActionItem {
            title: "Dismiss".to_string(),
            properties: HashMap::new(),
        });
    let response = Response::from_ok(
        request_id,
        serde_json::to_value(action).expect("serialize action"),
    );
    socket.send(response).await.expect("send response");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn install_prompt_accepts_null_response() {
    let _lock = env_lock();
    let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
    let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);
    let _env_stub = EnvGuard::set("SA_TEST_SOLC_INSTALL_MESSAGE", Some("solc install stub"));

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Main.sol",
            r#"
pragma solidity ^0.8.0;

contract Main {}
"#,
        )
        .build()
        .expect("fixture");

    let (mut service, mut socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;

    let (request_id, _params) = wait_for_show_message_request(&mut socket).await;
    let response = Response::from_ok(request_id, Value::Null);
    socket.send(response).await.expect("send response");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn install_prompt_handles_request_error() {
    let _lock = env_lock();
    let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
    let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);
    let _env_stub = EnvGuard::set("SA_TEST_SOLC_INSTALL_MESSAGE", Some("solc install stub"));

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Main.sol",
            r#"
pragma solidity ^0.8.0;

contract Main {}
"#,
        )
        .build()
        .expect("fixture");

    let (mut service, mut socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;

    let (request_id, _params) = wait_for_show_message_request(&mut socket).await;
    let response = Response::from_error(
        request_id,
        Error {
            code: ErrorCode::InternalError,
            message: "prompt error".into(),
            data: None,
        },
    );
    socket.send(response).await.expect("send response");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn install_prompt_reports_install_failure() {
    let _lock = env_lock();
    let _env_solc = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
    let _env_dapp_solc = EnvGuard::set("DAPP_SOLC_VERSION", None);
    let _env_stub = EnvGuard::set("SA_TEST_SOLC_INSTALL_MESSAGE", None);

    let foundry_toml = r#"
[profile.default]
solc = "not-a-version"
"#;
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(foundry_toml)
        .file(
            "src/Main.sol",
            r#"
pragma solidity ^0.8.0;

contract Main {}
"#,
        )
        .build()
        .expect("fixture");

    let (mut service, mut socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;

    let (request_id, params) = wait_for_show_message_request(&mut socket).await;
    let action = params
        .actions
        .as_ref()
        .and_then(|actions| actions.iter().find(|action| action.title == "Install"))
        .cloned()
        .unwrap_or(MessageActionItem {
            title: "Install".to_string(),
            properties: HashMap::new(),
        });
    let response = Response::from_ok(
        request_id,
        serde_json::to_value(action).expect("serialize action"),
    );
    socket.send(response).await.expect("send response");

    let message = wait_for_show_message(&mut socket).await;
    assert!(message.message.contains("invalid solc specification"));
}
