use sa_test_support::lsp::{diagnostic_codes, send_notification, wait_for_publish};
use serde_json::json;
use test_fixtures::DiagnosticsTestContext;
use tokio::time::Duration;
use tower_lsp::ClientSocket;
use tower_lsp::lsp_types::{
    DiagnosticSeverity, DidChangeConfigurationParams, NumberOrString, PublishDiagnosticsParams, Url,
};

mod test_fixtures;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

async fn wait_for_diagnostics_with_codes(
    socket: &mut ClientSocket,
    file_uri: &Url,
    expected_codes: &[&str],
    timeout: Duration,
) -> PublishDiagnosticsParams {
    wait_for_publish(socket, timeout, file_uri, |publish| {
        let codes = diagnostic_codes(publish);
        expected_codes
            .iter()
            .all(|code| codes.iter().any(|value| value == code))
    })
    .await
}

fn find_diagnostic<'a>(
    publish: &'a PublishDiagnosticsParams,
    code: &str,
) -> &'a tower_lsp::lsp_types::Diagnostic {
    publish
        .diagnostics
        .iter()
        .find(|diag| matches!(diag.code, Some(NumberOrString::String(ref value)) if value == code))
        .unwrap_or_else(|| panic!("diagnostic {code} not found"))
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_publish_solc_and_solar() {
    let context = DiagnosticsTestContext::new();
    let (mut service, mut socket) = context.start_service(None).await;

    let settings = json!({
        "solidityAnalyzer": {
            "lint": { "onSave": true }
        }
    });
    send_notification(
        &mut service,
        "workspace/didChangeConfiguration",
        DidChangeConfigurationParams { settings },
    )
    .await;

    context.open_and_save(&mut service).await;

    let publish = wait_for_diagnostics_with_codes(
        &mut socket,
        &context.file_uri,
        &["1234", "mixed-case-function"],
        TEST_TIMEOUT,
    )
    .await;

    let solc_diag = find_diagnostic(&publish, "1234");
    assert_eq!(solc_diag.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(solc_diag.source.as_deref(), Some("solc"));

    let lint_diag = find_diagnostic(&publish, "mixed-case-function");
    assert_eq!(lint_diag.severity, Some(DiagnosticSeverity::INFORMATION));
    assert_eq!(lint_diag.source.as_deref(), Some("forge-lint"));
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_on_save_from_initialize_options() {
    let context = DiagnosticsTestContext::new();
    let init_options = json!({
        "diagnostics": { "enable": true, "onSave": true },
        "lint": { "onSave": true }
    });
    let (mut service, mut socket) = context.start_service(Some(init_options)).await;

    context.open_and_save(&mut service).await;

    let publish =
        wait_for_diagnostics_with_codes(&mut socket, &context.file_uri, &["1234"], TEST_TIMEOUT)
            .await;

    let solc_diag = find_diagnostic(&publish, "1234");
    assert_eq!(solc_diag.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(solc_diag.source.as_deref(), Some("solc"));
}
