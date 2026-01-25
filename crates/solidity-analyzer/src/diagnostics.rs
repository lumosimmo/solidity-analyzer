use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::{AbortHandle, AbortRegistration, Abortable};
use sa_config::ResolvedFoundryConfig;
use sa_flycheck::{
    FlycheckConfig, FlycheckDiagnostic, FlycheckHandle, FlycheckRequest, FlycheckResult,
    FlycheckSeverity,
};
use sa_ide_diagnostics::{
    Diagnostic, DiagnosticSeverity, DiagnosticSource, collect_solar_lints,
    collect_solar_lints_with_overlay, merge_diagnostics,
};
use sa_paths::NormalizedPath;
use sa_span::lsp::to_lsp_range;
use sa_vfs::VfsSnapshot;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tower_lsp::Client;
use tower_lsp::lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticSeverity as LspSeverity, NumberOrString, Position,
    Range, Url,
};
use tracing::warn;

use crate::lsp_ext::{Health, ServerStatusNotification, ServerStatusParams};
use crate::lsp_utils::{path_to_url, url_to_path};
use crate::state::ServerState;

const ON_CHANGE_DEBOUNCE: Duration = Duration::from_millis(250);

pub struct Diagnostics {
    client: Client,
    state: Arc<Mutex<ServerState>>,
    flycheck: FlycheckHandle,
    shared: Arc<Mutex<DiagnosticsState>>,
}

impl Diagnostics {
    pub fn new(client: Client, state: Arc<Mutex<ServerState>>) -> Self {
        let (flycheck, mut results) = FlycheckHandle::spawn(FlycheckConfig::default());
        let shared = Arc::new(Mutex::new(DiagnosticsState::default()));
        let publish_client = client.clone();
        let publish_state = Arc::clone(&state);
        let publish_shared = Arc::clone(&shared);

        tokio::spawn(async move {
            while let Some(result) = results.recv().await {
                publish_flycheck_result(&publish_client, &publish_state, &publish_shared, result)
                    .await;
            }
        });

        Self {
            client,
            state,
            flycheck,
            shared,
        }
    }

    pub async fn publish_status(&self) {
        publish_status(&self.client, &self.state, &self.shared).await;
    }

    pub async fn did_save(&self, uri: &Url, run_solc: bool, run_solar: bool) {
        let Some(path) = url_to_path(uri) else {
            return;
        };
        let (config, solc_jobs) = {
            let state = self.state.lock().await;
            (state.config.clone(), state.lsp_config.toolchain.solc_jobs)
        };
        let should_clear_solc = !run_solc;
        let should_clear_solar = !run_solar;
        if should_clear_solc || should_clear_solar {
            clear_disabled_diagnostics(
                &self.client,
                &self.state,
                &self.shared,
                path.clone(),
                should_clear_solc,
                should_clear_solar,
            )
            .await;
        }

        if !run_solc && !run_solar {
            return;
        }

        let Some(config) = config else {
            return;
        };

        if run_solc {
            {
                let mut data = self.shared.lock().await;
                data.solc_active = true;
            }
            publish_status(&self.client, &self.state, &self.shared).await;
            let flycheck = self
                .flycheck
                .check(FlycheckRequest::new(config.clone()).with_solc_jobs(solc_jobs));
            if let Err(error) = flycheck.await {
                warn!(?error, "failed to enqueue flycheck request");
                {
                    let mut data = self.shared.lock().await;
                    data.solc_active = false;
                }
                publish_status(&self.client, &self.state, &self.shared).await;
            }
        }

        if run_solar {
            let client = self.client.clone();
            let state = Arc::clone(&self.state);
            let shared = Arc::clone(&self.shared);
            let path_clone = path.clone();
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let generation = {
                let mut data = shared.lock().await;
                data.lint_tasks.register(path.clone(), abort_handle)
            };
            publish_status(&client, &state, &shared).await;

            tokio::spawn(async move {
                let Some(lints) =
                    collect_lints(config, path_clone.as_str(), abort_registration).await
                else {
                    return;
                };

                update_solar_diagnostics(&client, &state, &shared, path_clone.clone(), lints).await;
                finish_lint_task(&shared, &path_clone, generation).await;
                publish_status(&client, &state, &shared).await;
            });
        }
    }

    pub async fn did_change(&self, uri: &Url) {
        let Some(path) = url_to_path(uri) else {
            return;
        };
        let (config, snapshot) = {
            let state = self.state.lock().await;
            (state.config.clone(), state.vfs_snapshot.clone())
        };
        let (Some(config), Some(snapshot)) = (config, snapshot) else {
            return;
        };

        let client = self.client.clone();
        let state = Arc::clone(&self.state);
        let shared = Arc::clone(&self.shared);
        let path_clone = path.clone();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let generation = {
            let mut data = shared.lock().await;
            data.change_tasks.register(path.clone(), abort_handle)
        };
        publish_status(&client, &state, &shared).await;

        tokio::spawn(async move {
            let Some(lints) = collect_lints_with_overlay(
                config,
                snapshot,
                path_clone.as_str(),
                abort_registration,
            )
            .await
            else {
                return;
            };

            update_solar_diagnostics(&client, &state, &shared, path_clone.clone(), lints).await;
            finish_change_task(&shared, &path_clone, generation).await;
            publish_status(&client, &state, &shared).await;
        });
    }
}

#[derive(Default)]
struct DiagnosticsState {
    solc: HashMap<NormalizedPath, Vec<Diagnostic>>,
    solar: HashMap<NormalizedPath, Vec<Diagnostic>>,
    last_published: HashSet<NormalizedPath>,
    lint_tasks: TaskTracker,
    change_tasks: TaskTracker,
    solc_active: bool,
    last_status: Option<ServerStatusParams>,
}

struct LintTask {
    generation: u64,
    handle: AbortHandle,
}

#[derive(Default)]
struct TaskTracker {
    tasks: HashMap<NormalizedPath, LintTask>,
    next_generation: u64,
}

impl TaskTracker {
    fn register(&mut self, path: NormalizedPath, handle: AbortHandle) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        if let Some(existing) = self.tasks.insert(path, LintTask { generation, handle }) {
            existing.handle.abort();
        }
        generation
    }

    fn finish(&mut self, path: &NormalizedPath, generation: u64) {
        let should_remove = self
            .tasks
            .get(path)
            .is_some_and(|task| task.generation == generation);
        if should_remove {
            self.tasks.remove(path);
        }
    }

    fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

async fn publish_flycheck_result(
    client: &Client,
    state: &Arc<Mutex<ServerState>>,
    shared: &Arc<Mutex<DiagnosticsState>>,
    result: FlycheckResult,
) {
    let mut solc = HashMap::new();
    for diag in result.diagnostics {
        let diag = flycheck_to_diagnostic(diag);
        solc.entry(diag.file_path.clone())
            .or_insert_with(Vec::new)
            .push(diag);
    }

    let entries = {
        let mut data = shared.lock().await;
        data.solc = solc;
        data.solc_active = false;
        let mut files = HashSet::new();
        files.extend(data.solc.keys().cloned());
        files.extend(data.solar.keys().cloned());
        files.extend(data.last_published.iter().cloned());

        let mut entries = Vec::new();
        let mut next_published = HashSet::new();
        for file in files {
            let merged = merge_diagnostics(
                data.solc.get(&file).cloned().unwrap_or_default(),
                data.solar.get(&file).cloned().unwrap_or_default(),
            );
            if !merged.is_empty() {
                next_published.insert(file.clone());
            }
            entries.push((file, merged));
        }
        data.last_published = next_published;
        entries
    };

    publish_entries(client, state, entries).await;
    publish_status(client, state, shared).await;
}

async fn finish_lint_task(
    shared: &Arc<Mutex<DiagnosticsState>>,
    path: &NormalizedPath,
    generation: u64,
) {
    let mut data = shared.lock().await;
    data.lint_tasks.finish(path, generation);
}

async fn finish_change_task(
    shared: &Arc<Mutex<DiagnosticsState>>,
    path: &NormalizedPath,
    generation: u64,
) {
    let mut data = shared.lock().await;
    data.change_tasks.finish(path, generation);
}

async fn collect_lints(
    config: ResolvedFoundryConfig,
    path: &str,
    abort_registration: AbortRegistration,
) -> Option<Vec<Diagnostic>> {
    let path_buf = PathBuf::from(path);
    let task = tokio::task::spawn_blocking(move || collect_solar_lints(&config, &[path_buf]));
    let result = match Abortable::new(task, abort_registration).await {
        Ok(result) => result,
        Err(_) => return None,
    };
    let lint_result = match result {
        Ok(result) => result,
        Err(error) => {
            warn!(?error, "solar lint task failed");
            return Some(Vec::new());
        }
    };

    match lint_result {
        Ok(lints) => Some(lints),
        Err(error) => {
            warn!(?error, "solar lint failed");
            Some(Vec::new())
        }
    }
}

async fn collect_lints_with_overlay(
    config: ResolvedFoundryConfig,
    snapshot: VfsSnapshot,
    path: &str,
    abort_registration: AbortRegistration,
) -> Option<Vec<Diagnostic>> {
    let path_buf = PathBuf::from(path);
    let task = async move {
        sleep(ON_CHANGE_DEBOUNCE).await;
        tokio::task::spawn_blocking(move || {
            collect_solar_lints_with_overlay(&config, &[path_buf], &snapshot)
        })
        .await
    };

    let result = match Abortable::new(task, abort_registration).await {
        Ok(result) => result,
        Err(_) => return None,
    };
    let lint_result = match result {
        Ok(result) => result,
        Err(error) => {
            warn!(?error, "solar lint task failed");
            return Some(Vec::new());
        }
    };

    match lint_result {
        Ok(lints) => Some(lints),
        Err(error) => {
            warn!(?error, "solar lint failed");
            Some(Vec::new())
        }
    }
}

async fn update_solar_diagnostics(
    client: &Client,
    state: &Arc<Mutex<ServerState>>,
    shared: &Arc<Mutex<DiagnosticsState>>,
    path: NormalizedPath,
    lints: Vec<Diagnostic>,
) {
    let entries = {
        let mut data = shared.lock().await;
        if lints.is_empty() {
            data.solar.remove(&path);
        } else {
            data.solar.insert(path.clone(), lints);
        }
        let merged = merge_diagnostics(
            data.solc.get(&path).cloned().unwrap_or_default(),
            data.solar.get(&path).cloned().unwrap_or_default(),
        );
        if merged.is_empty() {
            data.last_published.remove(&path);
        } else {
            data.last_published.insert(path.clone());
        }
        vec![(path, merged)]
    };

    publish_entries(client, state, entries).await;
}

async fn clear_disabled_diagnostics(
    client: &Client,
    state: &Arc<Mutex<ServerState>>,
    shared: &Arc<Mutex<DiagnosticsState>>,
    path: NormalizedPath,
    clear_solc: bool,
    clear_solar: bool,
) {
    let entries = {
        let mut data = shared.lock().await;
        if clear_solc {
            data.solc.remove(&path);
        }
        if clear_solar {
            data.solar.remove(&path);
        }
        let merged = merge_diagnostics(
            data.solc.get(&path).cloned().unwrap_or_default(),
            data.solar.get(&path).cloned().unwrap_or_default(),
        );
        if merged.is_empty() {
            data.last_published.remove(&path);
        } else {
            data.last_published.insert(path.clone());
        }
        vec![(path, merged)]
    };

    publish_entries(client, state, entries).await;
}

async fn publish_entries(
    client: &Client,
    state: &Arc<Mutex<ServerState>>,
    entries: Vec<(NormalizedPath, Vec<Diagnostic>)>,
) {
    let (snapshot, open_documents) = {
        let state = state.lock().await;
        (state.vfs_snapshot.clone(), state.open_documents.clone())
    };

    let mut text_cache = HashMap::new();
    for (path, diagnostics) in entries {
        let Some(uri) = path_to_url(&path) else {
            warn!(path = %path, "skipping diagnostics for non-file path");
            continue;
        };
        let text = text_cache
            .entry(path.clone())
            .or_insert_with(|| file_text(&snapshot, &path));
        let lsp_diagnostics = diagnostics
            .into_iter()
            .map(|diag| diagnostic_to_lsp(diag, text.as_deref()))
            .collect();
        let version = open_documents.get(&path).map(|doc| doc.version);
        client
            .publish_diagnostics(uri, lsp_diagnostics, version)
            .await;
    }
}

fn diagnostic_to_lsp(diag: Diagnostic, text: Option<&str>) -> LspDiagnostic {
    let range = match text {
        Some(text) => to_lsp_range(diag.range, text),
        None => {
            warn!(
                path = %diag.file_path,
                "missing text for diagnostic range; using fallback position"
            );
            Range::new(Position::new(0, 0), Position::new(0, 0))
        }
    };
    let severity = Some(severity_to_lsp(diag.severity));
    let code = diag.code.map(NumberOrString::String);
    let source = Some(diag.source.as_str().to_string());
    LspDiagnostic::new(range, severity, code, source, diag.message, None, None)
}

fn severity_to_lsp(severity: DiagnosticSeverity) -> LspSeverity {
    match severity {
        DiagnosticSeverity::Error => LspSeverity::ERROR,
        DiagnosticSeverity::Warning => LspSeverity::WARNING,
        DiagnosticSeverity::Info => LspSeverity::INFORMATION,
    }
}

fn flycheck_to_diagnostic(diag: FlycheckDiagnostic) -> Diagnostic {
    Diagnostic {
        file_path: diag.file_path,
        range: diag.range,
        severity: match diag.severity {
            FlycheckSeverity::Error => DiagnosticSeverity::Error,
            FlycheckSeverity::Warning => DiagnosticSeverity::Warning,
            FlycheckSeverity::Info => DiagnosticSeverity::Info,
        },
        code: diag.code,
        source: DiagnosticSource::Solc,
        fixable: false,
        message: diag.message,
    }
}

async fn publish_status(
    client: &Client,
    state: &Arc<Mutex<ServerState>>,
    shared: &Arc<Mutex<DiagnosticsState>>,
) {
    let supports_status = { state.lock().await.supports_server_status };
    if !supports_status {
        return;
    }

    let status = {
        let mut data = shared.lock().await;
        let solc_active = data.solc_active;
        let solar_active = !data.lint_tasks.is_empty() || !data.change_tasks.is_empty();
        let status = build_status(solc_active, solar_active);
        if data.last_status.as_ref() == Some(&status) {
            None
        } else {
            data.last_status = Some(status.clone());
            Some(status)
        }
    };

    if let Some(status) = status {
        client
            .send_notification::<ServerStatusNotification>(status)
            .await;
    }
}

fn build_status(solc_active: bool, solar_active: bool) -> ServerStatusParams {
    let (quiescent, message) = if solc_active && solar_active {
        (false, Some("Compiling and analyzing...".to_string()))
    } else if solc_active {
        (false, Some("Compiling...".to_string()))
    } else if solar_active {
        (false, Some("Analyzing...".to_string()))
    } else {
        (true, Some("OK".to_string()))
    };

    ServerStatusParams {
        health: Health::Ok,
        quiescent,
        message,
    }
}

fn file_text(snapshot: &Option<VfsSnapshot>, path: &NormalizedPath) -> Option<String> {
    if let Some(snapshot) = snapshot.as_ref()
        && let Some(file_id) = snapshot.file_id(path)
        && let Some(text) = snapshot.file_text(file_id)
    {
        return Some(text.to_string());
    }
    std::fs::read_to_string(path.as_str()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::pending;
    use sa_span::{TextRange, TextSize};
    use sa_vfs::{Vfs, VfsChange};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn task_tracker_aborts_previous_handle() {
        let mut tracker = TaskTracker::default();
        let path = NormalizedPath::new("src/Main.sol");

        let (handle_one, registration_one) = AbortHandle::new_pair();
        let (handle_two, _registration_two) = AbortHandle::new_pair();

        let abortable = Abortable::new(async { pending::<()>().await }, registration_one);
        let task = tokio::spawn(abortable);

        tracker.register(path.clone(), handle_one);
        tracker.register(path.clone(), handle_two);

        let result = timeout(Duration::from_millis(50), task)
            .await
            .expect("abortable should resolve");
        assert!(result.expect("join").is_err());
    }

    #[test]
    fn task_tracker_finish_requires_matching_generation() {
        let mut tracker = TaskTracker::default();
        let path = NormalizedPath::new("src/Main.sol");
        let (handle, _registration) = AbortHandle::new_pair();

        let generation = tracker.register(path.clone(), handle);
        tracker.finish(&path, generation + 1);
        assert!(!tracker.is_empty());

        tracker.finish(&path, generation);
        assert!(tracker.is_empty());
    }

    #[test]
    fn build_status_messages_and_quiescence() {
        let status = build_status(true, true);
        assert!(!status.quiescent);
        assert_eq!(
            status.message.as_deref(),
            Some("Compiling and analyzing...")
        );

        let status = build_status(true, false);
        assert!(!status.quiescent);
        assert_eq!(status.message.as_deref(), Some("Compiling..."));

        let status = build_status(false, true);
        assert!(!status.quiescent);
        assert_eq!(status.message.as_deref(), Some("Analyzing..."));

        let status = build_status(false, false);
        assert!(status.quiescent);
        assert_eq!(status.message.as_deref(), Some("OK"));
    }

    #[test]
    fn diagnostic_to_lsp_uses_fallback_range_without_text() {
        let diag = Diagnostic {
            file_path: NormalizedPath::new("src/Main.sol"),
            range: TextRange::new(TextSize::new(5), TextSize::new(7)),
            severity: DiagnosticSeverity::Warning,
            code: Some("W001".to_string()),
            source: DiagnosticSource::Solar,
            fixable: false,
            message: "lint".to_string(),
        };

        let lsp = diagnostic_to_lsp(diag, None);
        assert_eq!(
            lsp.range,
            Range::new(Position::new(0, 0), Position::new(0, 0))
        );
        assert_eq!(lsp.severity, Some(LspSeverity::WARNING));
        assert_eq!(lsp.code, Some(NumberOrString::String("W001".to_string())));
        assert_eq!(lsp.source.as_deref(), Some("solar"));
    }

    #[test]
    fn flycheck_to_diagnostic_maps_severity() {
        let diag = FlycheckDiagnostic {
            file_path: NormalizedPath::new("src/Main.sol"),
            range: TextRange::new(TextSize::new(0), TextSize::new(1)),
            severity: FlycheckSeverity::Info,
            code: None,
            message: "info".to_string(),
        };

        let mapped = flycheck_to_diagnostic(diag);
        assert_eq!(mapped.severity, DiagnosticSeverity::Info);
        assert_eq!(mapped.source, DiagnosticSource::Solc);
    }

    #[test]
    fn file_text_reads_from_snapshot_and_disk() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("Main.sol");
        std::fs::write(&path, "contract Main {}").expect("write file");
        let normalized = NormalizedPath::new(path.to_string_lossy());

        let mut vfs = Vfs::default();
        vfs.apply_change(VfsChange::Set {
            path: normalized.clone(),
            text: Arc::from("contract InVfs {}"),
        });
        let snapshot = vfs.snapshot();

        let from_snapshot = file_text(&Some(snapshot.clone()), &normalized);
        assert_eq!(from_snapshot.as_deref(), Some("contract InVfs {}"));

        let empty_snapshot = Vfs::default().snapshot();
        let from_disk = file_text(&Some(empty_snapshot), &normalized);
        assert_eq!(from_disk.as_deref(), Some("contract Main {}"));
    }
}
