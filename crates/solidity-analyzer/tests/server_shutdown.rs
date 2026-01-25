use sa_test_support::lsp::{make_server, response_result, send_request};
use sa_test_utils::FixtureBuilder;
use tower::{Service, ServiceExt};
use tower_lsp::jsonrpc::Request;
use tower_lsp::lsp_types::{ClientCapabilities, InitializeParams, InitializeResult, Url};

#[tokio::test]
async fn shutdown_returns_ok() {
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", "contract Main {}\n")
        .build()
        .expect("fixture");

    let (mut service, _socket) = make_server(solidity_analyzer::Server::new)();
    let root_uri = Url::from_file_path(fixture.root()).expect("root uri");
    let initialize = InitializeParams {
        root_uri: Some(root_uri),
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<InitializeResult>(response);

    let request = Request::build("shutdown").id(2).finish();
    let response = service
        .ready()
        .await
        .expect("service ready")
        .call(request)
        .await
        .expect("service call");
    let response = response.expect("shutdown response");
    let _: () = response_result(response);
}
