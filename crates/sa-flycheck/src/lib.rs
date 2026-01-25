use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use foundry_compilers::{Project, compilers::multi::MultiCompilerError};
use sa_config::ResolvedFoundryConfig;
use sa_paths::NormalizedPath;
use sa_project_model::FoundryWorkspace;
use sa_span::{TextRange, TextSize};
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Debug, Clone, Copy)]
pub struct FlycheckConfig {
    pub debounce: Duration,
}

impl Default for FlycheckConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(50),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlycheckRequest {
    config: ResolvedFoundryConfig,
    solc_jobs: Option<usize>,
}

impl FlycheckRequest {
    pub fn new(config: ResolvedFoundryConfig) -> Self {
        Self {
            config,
            solc_jobs: None,
        }
    }

    pub fn with_solc_jobs(mut self, solc_jobs: Option<usize>) -> Self {
        self.solc_jobs = solc_jobs;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlycheckDiagnostic {
    pub file_path: NormalizedPath,
    pub range: TextRange,
    pub severity: FlycheckSeverity,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlycheckSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlycheckResult {
    pub generation: u64,
    pub diagnostics: Vec<FlycheckDiagnostic>,
}

#[derive(Debug, Clone)]
pub struct FlycheckHandle {
    sender: mpsc::Sender<FlycheckRequest>,
}

impl FlycheckHandle {
    pub fn spawn(config: FlycheckConfig) -> (Self, mpsc::Receiver<FlycheckResult>) {
        let (request_tx, request_rx) = mpsc::channel(8);
        let (result_tx, result_rx) = mpsc::channel(8);
        tokio::spawn(run_flycheck(request_rx, result_tx, config));
        (Self { sender: request_tx }, result_rx)
    }

    pub async fn check(
        &self,
        request: FlycheckRequest,
    ) -> Result<(), mpsc::error::SendError<FlycheckRequest>> {
        self.sender.send(request).await
    }
}

async fn run_flycheck(
    mut requests: mpsc::Receiver<FlycheckRequest>,
    results: mpsc::Sender<FlycheckResult>,
    config: FlycheckConfig,
) {
    let mut latest_generation: u64 = 0;
    let mut pending = match requests.recv().await {
        Some(request) => {
            latest_generation = latest_generation.wrapping_add(1);
            PendingRequest {
                generation: latest_generation,
                request,
            }
        }
        None => return,
    };

    loop {
        let Some(pending_request) = debounce_request(
            &mut requests,
            pending,
            config.debounce,
            &mut latest_generation,
        )
        .await
        else {
            return;
        };
        let current_generation = pending_request.generation;

        let root = workspace_root(pending_request.request.config.workspace());
        let fallback_path = fallback_error_path(&root);
        let mut compile_task =
            tokio::task::spawn_blocking(move || compile_request(pending_request.request));
        let mut next_request: Option<PendingRequest> = None;

        tokio::select! {
            result = &mut compile_task => {
                match result {
                    Ok(Ok(diagnostics)) => {
                        if current_generation == latest_generation {
                            let _ = results.send(FlycheckResult {
                                generation: current_generation,
                                diagnostics,
                            }).await;
                        }
                    }
                    Ok(Err(error)) => {
                        if current_generation == latest_generation {
                            let _ = results.send(FlycheckResult {
                                generation: current_generation,
                                diagnostics: vec![error_diagnostic(error.to_string(), fallback_path.clone())],
                            }).await;
                        }
                    }
                    Err(error) => {
                        if current_generation == latest_generation {
                            let _ = results.send(FlycheckResult {
                                generation: current_generation,
                                diagnostics: vec![error_diagnostic(error.to_string(), fallback_path.clone())],
                            }).await;
                        }
                    }
                }
            }
            next = requests.recv() => {
                if let Some(request) = next {
                    latest_generation = latest_generation.wrapping_add(1);
                    next_request = Some(PendingRequest {
                        generation: latest_generation,
                        request,
                    });
                    compile_task.abort();
                } else {
                    next_request = None;
                }
            }
        }

        match next_request {
            Some(request) => {
                pending = request;
            }
            None => match requests.recv().await {
                Some(request) => {
                    latest_generation = latest_generation.wrapping_add(1);
                    pending = PendingRequest {
                        generation: latest_generation,
                        request,
                    };
                }
                None => {
                    return;
                }
            },
        }
    }
}

async fn debounce_request(
    requests: &mut mpsc::Receiver<FlycheckRequest>,
    mut current: PendingRequest,
    debounce: Duration,
    latest_generation: &mut u64,
) -> Option<PendingRequest> {
    if debounce.is_zero() {
        return Some(current);
    }
    let delay = tokio::time::sleep(debounce);
    tokio::pin!(delay);
    loop {
        tokio::select! {
            _ = &mut delay => return Some(current),
            next = requests.recv() => match next {
                Some(request) => {
                    *latest_generation = latest_generation.wrapping_add(1);
                    current = PendingRequest {
                        generation: *latest_generation,
                        request,
                    };
                    delay
                        .as_mut()
                        .reset(tokio::time::Instant::now() + debounce);
                }
                None => return None,
            }
        }
    }
}

#[derive(Debug)]
struct PendingRequest {
    generation: u64,
    request: FlycheckRequest,
}

fn compile_request(request: FlycheckRequest) -> Result<Vec<FlycheckDiagnostic>> {
    let solc_jobs = request.solc_jobs;
    let config = request.config;
    let root = workspace_root(config.workspace());
    let project = build_project(&config, solc_jobs)?;
    let output = project.compile()?;
    Ok(solc_diagnostics(&root, output.output().errors.as_slice()))
}

fn build_project(config: &ResolvedFoundryConfig, solc_jobs: Option<usize>) -> Result<Project> {
    let mut project = config
        .foundry_config()
        .ephemeral_project()
        .map_err(anyhow::Error::from)?;
    apply_solc_jobs(&mut project, solc_jobs);
    Ok(project)
}

fn apply_solc_jobs(project: &mut Project, solc_jobs: Option<usize>) {
    let Some(jobs) = solc_jobs else {
        return;
    };
    if jobs == 0 {
        warn!(solc_jobs = jobs, "ignoring invalid solc jobs override");
        return;
    }
    project.set_solc_jobs(jobs);
}

fn workspace_root(workspace: &FoundryWorkspace) -> PathBuf {
    PathBuf::from(workspace.root().as_str())
}

fn solc_diagnostics(root: &Path, errors: &[MultiCompilerError]) -> Vec<FlycheckDiagnostic> {
    let mut diagnostics = Vec::new();
    let fallback_path = fallback_error_path(root);
    let fallback_range = TextRange::new(TextSize::from(0), TextSize::from(0));
    for error in errors {
        let MultiCompilerError::Solc(error) = error else {
            continue;
        };
        let (file_path, range) = match error.source_location.as_ref() {
            Some(location) => match text_range_from_location(location.start, location.end) {
                Some(range) => (normalize_error_path(root, &location.file), range),
                None => (fallback_path.clone(), fallback_range),
            },
            None => (fallback_path.clone(), fallback_range),
        };
        diagnostics.push(FlycheckDiagnostic {
            file_path,
            range,
            severity: match error.severity {
                foundry_compilers::artifacts::Severity::Error => FlycheckSeverity::Error,
                foundry_compilers::artifacts::Severity::Warning => FlycheckSeverity::Warning,
                foundry_compilers::artifacts::Severity::Info => FlycheckSeverity::Info,
            },
            code: error.error_code.map(|code| code.to_string()),
            message: error.message.clone(),
        });
    }
    diagnostics
}

fn normalize_error_path(root: &Path, file: &str) -> NormalizedPath {
    let path = Path::new(file);
    if path.is_absolute() {
        NormalizedPath::new(file)
    } else {
        NormalizedPath::new(root.join(path).to_string_lossy())
    }
}

fn fallback_error_path(root: &Path) -> NormalizedPath {
    let candidate = root.join("foundry.toml");
    if candidate.is_file() {
        return NormalizedPath::new(candidate.to_string_lossy());
    }
    if root.exists() && root.is_dir() {
        return NormalizedPath::new(root.to_string_lossy());
    }
    warn!(
        root = %root.display(),
        "workspace root missing; using current directory for diagnostics"
    );
    match std::env::current_dir() {
        Ok(dir) => NormalizedPath::new(dir.to_string_lossy()),
        Err(error) => {
            warn!(
                ?error,
                "failed to resolve current directory; using fallback path"
            );
            NormalizedPath::new(".")
        }
    }
}

fn text_range_from_location(start: i32, end: i32) -> Option<TextRange> {
    let start = u32::try_from(start).ok()?;
    let end = u32::try_from(end).ok()?;
    if start > end {
        return None;
    }
    Some(TextRange::new(TextSize::from(start), TextSize::from(end)))
}

fn error_diagnostic(message: String, fallback_path: NormalizedPath) -> FlycheckDiagnostic {
    FlycheckDiagnostic {
        file_path: fallback_path,
        range: TextRange::new(TextSize::from(0), TextSize::from(0)),
        severity: FlycheckSeverity::Error,
        code: None,
        message,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use foundry_compilers::artifacts::Severity;
    use foundry_compilers::solc::SolcCompiler;
    use sa_test_support::{setup_foundry_root, setup_foundry_root_with_extras, write_stub_solc};
    use sa_test_utils::{EnvGuard, env_lock, load_foundry_config};
    use tempfile::tempdir;

    use super::build_project;

    #[test]
    fn build_project_uses_foundry_settings_and_paths() {
        let _lock = env_lock();
        let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
        let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
        let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
        let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");

        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        setup_foundry_root_with_extras(root);

        let foundry_toml = r#"
[profile.default]
libs = ["lib", "lib2"]
remappings = ["forge-std/=lib/forge-std/src/"]
optimizer = true
optimizer_runs = 222
via_ir = true
evm_version = "paris"
allow_paths = ["allow"]
include_paths = ["include"]
cache_path = "cache"
out = "out"
build_info_path = "out/build-info"
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let config = load_foundry_config(root, None).expect("load config");
        let project = build_project(&config, None).expect("build project");

        let expected_settings = config.compiler_settings().expect("compiler settings");
        assert_eq!(
            project.settings.solc.optimizer.runs,
            expected_settings.solc.optimizer.runs
        );
        assert_eq!(project.settings.solc.via_ir, expected_settings.solc.via_ir);
        assert_eq!(
            project.settings.solc.evm_version,
            expected_settings.solc.evm_version
        );

        let expected_paths = config.project_paths();
        assert_eq!(project.paths.libraries, expected_paths.libraries);
        assert_eq!(project.paths.remappings, expected_paths.remappings);
        assert_eq!(project.paths.allowed_paths, expected_paths.allowed_paths);
        assert_eq!(project.paths.include_paths, expected_paths.include_paths);
        assert_eq!(project.paths.cache, expected_paths.cache);
        assert_eq!(project.paths.artifacts, expected_paths.artifacts);
        assert_eq!(project.paths.build_infos, expected_paths.build_infos);
    }

    #[test]
    fn build_project_uses_explicit_solc() {
        let _lock = env_lock();
        let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
        let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
        let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
        let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");

        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        setup_foundry_root(root);

        let solc_dir = tempdir().expect("solc dir");
        let solc_path = write_stub_solc(solc_dir.path(), "0.8.20", None);
        let solc_path_toml = solc_path.to_string_lossy().replace('\\', "\\\\");
        let foundry_toml = format!(
            r#"
[profile.default]
solc = "{solc_path_toml}"
"#
        );
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let config = load_foundry_config(root, None).expect("load config");
        let project = build_project(&config, None).expect("build project");

        let solc = match project.compiler.solc {
            Some(SolcCompiler::Specific(solc)) => solc,
            other => panic!("expected explicit solc compiler, got {other:?}"),
        };
        assert_eq!(solc.solc, solc_path);
        assert_eq!(solc.version.to_string(), "0.8.20");
    }

    #[test]
    fn build_project_uses_auto_detect_when_solc_unset() {
        let _lock = env_lock();
        let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
        let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
        let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
        let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");

        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        setup_foundry_root(root);

        let foundry_toml = r#"
[profile.default]
src = "src"
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let config = load_foundry_config(root, None).expect("load config");
        let project = build_project(&config, None).expect("build project");

        assert!(matches!(
            project.compiler.solc,
            Some(SolcCompiler::AutoDetect)
        ));
    }

    #[test]
    fn build_project_applies_ignore_filters() {
        let _lock = env_lock();
        let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
        let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
        let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
        let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");

        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        setup_foundry_root(root);
        fs::write(root.join("src/Ignore.sol"), "contract Ignore {}\n")
            .expect("write ignore source");

        let foundry_toml = r#"
[profile.default]
ignored_error_codes = [1234]
ignored_warnings_from = ["src/Ignore.sol"]
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let config = load_foundry_config(root, None).expect("load config");
        let project = build_project(&config, None).expect("build project");

        assert!(project.ignored_error_codes.contains(&1234));
        assert!(
            project
                .ignored_file_paths
                .contains(&PathBuf::from("src/Ignore.sol"))
        );
    }

    #[test]
    fn build_project_applies_deny_warnings() {
        let _lock = env_lock();
        let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
        let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
        let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
        let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");

        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        setup_foundry_root(root);

        let foundry_toml = r#"
[profile.default]
deny = "warnings"
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let config = load_foundry_config(root, None).expect("load config");
        let project = build_project(&config, None).expect("build project");

        assert_eq!(project.compiler_severity_filter, Severity::Warning);
    }

    #[test]
    fn build_project_wires_additional_settings_and_restrictions() {
        let _lock = env_lock();
        let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
        let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
        let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
        let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");

        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        setup_foundry_root(root);
        fs::write(root.join("src/Restricted.sol"), "contract Restricted {}\n")
            .expect("write restricted source");

        let foundry_toml = r#"
[profile.default]
src = "src"

[[profile.default.additional_compiler_profiles]]
name = "fast"
optimizer_runs = 50
via_ir = true

[[profile.default.compilation_restrictions]]
paths = "src/Restricted.sol"
optimizer_runs = 999
via_ir = true
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let config = load_foundry_config(root, None).expect("load config");
        let project = build_project(&config, None).expect("build project");

        let fast = project
            .additional_settings
            .get("fast")
            .expect("additional settings profile");
        assert_eq!(fast.solc.optimizer.runs, Some(50));
        assert_eq!(fast.solc.via_ir, Some(true));

        let restricted_path = project.paths.root.join("src/Restricted.sol");
        let restriction = project
            .restrictions
            .get(&restricted_path)
            .expect("restriction entry");
        assert_eq!(restriction.solc.optimizer_runs.min, Some(999));
        assert_eq!(restriction.solc.optimizer_runs.max, Some(999));
        assert_eq!(restriction.solc.via_ir, Some(true));
    }

    #[test]
    fn build_project_applies_solc_jobs_override() {
        let _lock = env_lock();
        let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
        let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
        let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
        let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");

        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        setup_foundry_root(root);

        let foundry_toml = r#"
[profile.default]
src = "src"
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let config = load_foundry_config(root, None).expect("load config");
        let project = build_project(&config, Some(1)).expect("build project");

        let debug = format!("{project:?}");
        assert!(debug.contains("solc_jobs: 1"), "project debug: {debug}");
    }

    #[test]
    fn build_project_ignores_zero_solc_jobs_override() {
        let _lock = env_lock();
        let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
        let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
        let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
        let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");

        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        setup_foundry_root(root);

        let foundry_toml = r#"
[profile.default]
src = "src"
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let config = load_foundry_config(root, None).expect("load config");
        let project = build_project(&config, Some(0)).expect("build project");

        let debug = format!("{project:?}");
        assert!(!debug.contains("solc_jobs: 0"), "project debug: {debug}");
    }
}
