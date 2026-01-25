use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use foundry_compilers::{
    Graph, Project,
    compilers::multi::MultiCompiler,
    solc::{Solc, SolcCompiler},
};
use sa_config::ResolvedFoundryConfig;
use semver::{Version, VersionReq};
use tracing::warn;

static INSTALLING_VERSIONS: OnceLock<Mutex<HashSet<Version>>> = OnceLock::new();
static INSTALL_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolcPath {
    path: PathBuf,
    version: Version,
}

impl SolcPath {
    pub fn new(path: PathBuf, version: Version) -> Self {
        Self { path, version }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn to_solc(&self) -> Solc {
        Solc::new_with_version(self.path.clone(), self.version.clone())
    }
}

pub struct Toolchain {
    config: ResolvedFoundryConfig,
}

impl Toolchain {
    pub fn new(config: ResolvedFoundryConfig) -> Self {
        Self { config }
    }

    pub fn solc_spec(&self) -> Option<&str> {
        self.config.active_profile().solc_version()
    }

    pub fn solc_spec_is_path(&self) -> bool {
        match self.solc_spec() {
            Some(spec) => looks_like_path(spec),
            None => false,
        }
    }

    pub fn resolve(&self) -> Result<SolcPath> {
        match self.solc_spec() {
            Some(spec) => resolve_spec(spec),
            None => resolve_from_path("solc"),
        }
    }

    pub fn auto_detect_versions(&self) -> Result<Vec<Version>> {
        let paths = self.config.project_paths().with_language();
        let compiler = MultiCompiler::new(Some(SolcCompiler::AutoDetect), None)?;
        let project = Project::builder()
            .paths(paths)
            .no_artifacts()
            .ephemeral()
            .build(compiler)?;
        let graph = Graph::resolve(&project.paths)?;
        let resolved = graph.into_sources_by_version(&project)?;
        let mut versions = resolved
            .sources
            .values()
            .flat_map(|entries| entries.iter().map(|(version, _, _)| version.clone()))
            .collect::<Vec<_>>();
        versions.sort();
        versions.dedup();
        Ok(versions)
    }

    pub fn install_solc(&self) -> Result<String> {
        let offline = self.config.foundry_config().offline;
        match self.solc_spec() {
            Some(spec) => {
                if offline && !looks_like_path(spec) {
                    bail!("can't install solc {spec} in offline mode");
                }
                install_from_spec(spec)
            }
            None => {
                if offline {
                    bail!("can't install solc in offline mode");
                }
                install_from_auto_detect(self)
            }
        }
    }
}

pub fn is_svm_installed(version: &Version) -> Result<bool> {
    Ok(Solc::find_svm_installed_version(version)?.is_some())
}

fn install_from_spec(spec: &str) -> Result<String> {
    if looks_like_path(spec) {
        let resolved = resolve_from_path(spec)?;
        return Ok(format!(
            "solc configured to local path {} (version {})",
            resolved.path().display(),
            resolved.version()
        ));
    }

    if let Ok(version) = Version::parse(spec) {
        let (installed, already) = install_versions([version])?;
        return Ok(format_install_summary(
            &format!("version {spec}"),
            &installed,
            &already,
        ));
    }

    if let Ok(version_req) = VersionReq::parse(spec) {
        let version = select_version_for_requirement(&version_req)?;
        let (installed, already) = install_versions([version])?;
        return Ok(format_install_summary(
            &format!("version requirement {spec}"),
            &installed,
            &already,
        ));
    }

    Err(anyhow!("invalid solc specification: {spec}"))
}

fn install_from_auto_detect(toolchain: &Toolchain) -> Result<String> {
    let versions = toolchain.auto_detect_versions()?;
    if versions.is_empty() {
        bail!("no Solidity sources found for auto-detect");
    }

    let (installed, already) = install_versions(versions)?;
    Ok(format_install_summary("auto-detect", &installed, &already))
}

/// Returns the queue depth for pre-spawned installer threads.
/// `SOLIDITY_ANALYZER_INSTALL_CONCURRENCY` sets this queue depth; actual installs
/// are serialized by `INSTALL_MUTEX`.
fn install_queue_depth() -> usize {
    let default = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    env::var("SOLIDITY_ANALYZER_INSTALL_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

struct InstallOutcome {
    version: Version,
    result: Result<()>,
}

enum InstallMessage {
    Started(Version),
    Finished(InstallOutcome),
}

fn spawn_install(version: Version, tx: mpsc::Sender<InstallMessage>) {
    thread::spawn(move || {
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let mutex = INSTALL_MUTEX.get_or_init(|| Mutex::new(()));
            let _guard = mutex
                .lock()
                .map_err(|error| anyhow!("install mutex poisoned: {error}"))?;
            let _ = tx.send(InstallMessage::Started(version.clone()));
            Solc::blocking_install(&version)
                .map(|_| ())
                .map_err(Into::into)
        }));
        let outcome = match result {
            Ok(result) => result,
            Err(_) => Err(anyhow!("install panicked for solc {version}")),
        };
        let _ = tx.send(InstallMessage::Finished(InstallOutcome {
            version: version.clone(),
            result: outcome,
        }));
        finish_install(&version);
    });
}

fn finish_versions<'a, I>(versions: I)
where
    I: IntoIterator<Item = &'a Version>,
{
    for version in versions {
        finish_install(version);
    }
}

fn install_versions<I>(versions: I) -> Result<(Vec<Version>, Vec<Version>)>
where
    I: IntoIterator<Item = Version>,
{
    const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);

    let mut installed = Vec::new();
    let mut already = Vec::new();
    let mut candidates = VecDeque::new();
    let mut seen = HashSet::new();
    for version in versions {
        if !seen.insert(version.clone()) {
            continue;
        }
        if Solc::find_svm_installed_version(&version)?.is_some() {
            already.push(version);
        } else {
            candidates.push_back(version);
        }
    }

    if candidates.is_empty() {
        return Ok((installed, already));
    }

    let mut registered = Vec::new();
    let mut pending = VecDeque::new();
    for version in candidates {
        match register_install(&version) {
            Ok(RegisterOutcome::Acquired) => {
                registered.push(version.clone());
                pending.push_back(version);
            }
            Ok(RegisterOutcome::Waited) => {
                already.push(version);
            }
            Err(error) => {
                finish_versions(&registered);
                return Err(error);
            }
        }
    }

    if pending.is_empty() {
        return Ok((installed, already));
    }

    let queue_depth = install_queue_depth();
    let (tx, rx) = mpsc::channel::<InstallMessage>();
    let mut in_flight: HashMap<Version, Option<Instant>> = HashMap::new();

    while in_flight.len() < queue_depth {
        let Some(version) = pending.pop_front() else {
            break;
        };
        in_flight.insert(version.clone(), None);
        spawn_install(version, tx.clone());
    }

    while !in_flight.is_empty() {
        let next_deadline = in_flight
            .iter()
            .filter_map(|(version, deadline)| deadline.map(|deadline| (version.clone(), deadline)))
            .min_by_key(|(_, deadline)| *deadline);
        let message = match next_deadline {
            Some((next_version, next_deadline)) => {
                let timeout = next_deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(timeout) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let in_flight_versions = in_flight.keys().cloned().collect::<Vec<_>>();
                        let pending_versions = pending.iter().cloned().collect::<Vec<_>>();
                        warn!(
                            next_version = %next_version,
                            in_flight = %join_versions(&in_flight_versions),
                            pending = %join_versions(&pending_versions),
                            "abandoning install threads running blocking_install under INSTALL_MUTEX; finish_install will run when they complete"
                        );
                        finish_versions(in_flight.keys());
                        finish_versions(pending.iter());
                        return Err(anyhow!("install timed out for solc {next_version}"));
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        let in_flight_versions = in_flight.keys().cloned().collect::<Vec<_>>();
                        let pending_versions = pending.iter().cloned().collect::<Vec<_>>();
                        warn!(
                            next_version = %next_version,
                            in_flight = %join_versions(&in_flight_versions),
                            pending = %join_versions(&pending_versions),
                            "abandoning install threads running blocking_install under INSTALL_MUTEX; finish_install will run when they complete"
                        );
                        finish_versions(in_flight.keys());
                        finish_versions(pending.iter());
                        return Err(anyhow!(
                            "install failed for solc {next_version}: worker thread disconnected"
                        ));
                    }
                }
            }
            None => match rx.recv() {
                Ok(message) => message,
                Err(mpsc::RecvError) => {
                    let next_version = in_flight
                        .keys()
                        .next()
                        .cloned()
                        .expect("in-flight installs");
                    let in_flight_versions = in_flight.keys().cloned().collect::<Vec<_>>();
                    let pending_versions = pending.iter().cloned().collect::<Vec<_>>();
                    warn!(
                        next_version = %next_version,
                        in_flight = %join_versions(&in_flight_versions),
                        pending = %join_versions(&pending_versions),
                        "abandoning install threads running blocking_install under INSTALL_MUTEX; finish_install will run when they complete"
                    );
                    finish_versions(in_flight.keys());
                    finish_versions(pending.iter());
                    return Err(anyhow!(
                        "install failed for solc {next_version}: worker thread disconnected"
                    ));
                }
            },
        };

        match message {
            InstallMessage::Started(version) => {
                if let Some(deadline) = in_flight.get_mut(&version) {
                    *deadline = Some(Instant::now() + INSTALL_TIMEOUT);
                }
            }
            InstallMessage::Finished(outcome) => {
                in_flight.remove(&outcome.version);
                if let Err(error) = outcome.result {
                    let in_flight_versions = in_flight.keys().cloned().collect::<Vec<_>>();
                    let pending_versions = pending.iter().cloned().collect::<Vec<_>>();
                    warn!(
                        error = %error,
                        failed_version = %outcome.version,
                        in_flight = %join_versions(&in_flight_versions),
                        pending = %join_versions(&pending_versions),
                        "abandoning install threads running blocking_install under INSTALL_MUTEX; finish_install will run when they complete"
                    );
                    finish_versions(in_flight.keys());
                    finish_versions(pending.iter());
                    return Err(error);
                }
                installed.push(outcome.version);
                if let Some(version) = pending.pop_front() {
                    in_flight.insert(version.clone(), None);
                    spawn_install(version, tx.clone());
                }
            }
        }
    }
    Ok((installed, already))
}

enum RegisterOutcome {
    Acquired,
    Waited,
}

fn register_install(version: &Version) -> Result<RegisterOutcome> {
    const INSTALL_POLL_INTERVAL: Duration = Duration::from_millis(50);
    let versions = INSTALLING_VERSIONS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut waited = false;
    loop {
        let mut guard = versions
            .lock()
            .map_err(|error| anyhow!("installing versions lock poisoned: {error}"))?;
        if guard.contains(version) {
            waited = true;
            drop(guard);
            thread::sleep(INSTALL_POLL_INTERVAL);
            continue;
        }
        if waited {
            drop(guard);
            if is_svm_installed(version)? {
                return Ok(RegisterOutcome::Waited);
            }
            waited = false;
            continue;
        }
        guard.insert(version.clone());
        return Ok(RegisterOutcome::Acquired);
    }
}

fn finish_install(version: &Version) {
    let versions = INSTALLING_VERSIONS.get_or_init(|| Mutex::new(HashSet::new()));
    match versions.lock() {
        Ok(mut guard) => {
            guard.remove(version);
        }
        Err(poisoned) => {
            warn!(
                %version,
                "installing versions lock poisoned; recovering to clear version"
            );
            let mut guard = poisoned.into_inner();
            guard.remove(version);
        }
    };
}

pub fn select_version_for_requirement(req: &VersionReq) -> Result<Version> {
    let mut installed = Solc::installed_versions();
    installed.sort();
    if let Some(version) = installed.iter().rev().find(|version| req.matches(version)) {
        return Ok(version.clone());
    }

    let mut released = Solc::released_versions();
    released.sort();
    released
        .into_iter()
        .rev()
        .find(|version| req.matches(version))
        .ok_or_else(|| anyhow!("no solc version matches requirement {req}"))
}

fn format_install_summary(label: &str, installed: &[Version], already: &[Version]) -> String {
    let mut parts = Vec::new();
    if !installed.is_empty() {
        parts.push(format!("installed {}", join_versions(installed)));
    }
    if !already.is_empty() {
        parts.push(format!("already installed {}", join_versions(already)));
    }
    if parts.is_empty() {
        parts.push("nothing to install".to_string());
    }
    format!("solc {label}: {}", parts.join("; "))
}

fn join_versions(versions: &[Version]) -> String {
    versions
        .iter()
        .map(|version| version.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn resolve_spec(spec: &str) -> Result<SolcPath> {
    if looks_like_path(spec) {
        return resolve_from_path(spec);
    }

    if let Ok(version) = Version::parse(spec) {
        if let Some(solc) = find_installed_version(&version)? {
            return Ok(SolcPath::new(solc.solc, solc.version));
        }
        bail!("solc {version} is not installed");
    }

    if let Ok(version_req) = VersionReq::parse(spec) {
        if let Some(solc) = find_matching_installed_version(&version_req)? {
            return Ok(SolcPath::new(solc.solc, solc.version));
        }
        bail!("no installed solc version matches requirement {version_req}");
    }

    Err(anyhow!("invalid solc specification: {spec}"))
}

fn resolve_from_path(path: &str) -> Result<SolcPath> {
    let resolved = resolve_executable(path).unwrap_or_else(|| PathBuf::from(path));
    let solc = Solc::new(resolved).with_context(|| format!("failed to resolve solc at {path}"))?;
    Ok(SolcPath::new(solc.solc, solc.version))
}

fn looks_like_path(spec: &str) -> bool {
    spec.contains(std::path::MAIN_SEPARATOR)
        || spec.contains('/')
        || spec.contains('\\')
        || spec.ends_with(".exe")
}

fn resolve_executable(path: &str) -> Option<PathBuf> {
    let path_ref = Path::new(path);
    if path_ref.is_file() {
        return Some(path_ref.to_path_buf());
    }

    if looks_like_path(path) {
        return None;
    }

    let path_var = env::var_os("PATH")?;
    let extensions = executable_extensions(path_ref);
    for dir in env::split_paths(&path_var) {
        for ext in &extensions {
            let candidate = if ext.is_empty() {
                dir.join(path)
            } else {
                dir.join(format!("{path}{ext}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn executable_extensions(path: &Path) -> Vec<&'static str> {
    if path.extension().is_some() {
        return vec![""];
    }
    if cfg!(windows) {
        vec![".exe", ".cmd", ".bat"]
    } else {
        vec![""]
    }
}

fn find_installed_version(version: &Version) -> Result<Option<Solc>> {
    Ok(Solc::find_svm_installed_version(version)?)
}

fn find_matching_installed_version(version_req: &VersionReq) -> Result<Option<Solc>> {
    let mut installed = Solc::installed_versions();
    installed.sort();
    for version in installed.into_iter().rev() {
        if version_req.matches(&version) {
            return Ok(Solc::find_svm_installed_version(&version)?);
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use sa_test_utils::{EnvGuard, env_lock};
    use tempfile::tempdir;

    static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn test_guard() -> MutexGuard<'static, ()> {
        TEST_MUTEX
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test mutex")
    }

    struct TestEnv {
        _lock: MutexGuard<'static, ()>,
        _guards: Vec<EnvGuard>,
        home: tempfile::TempDir,
    }

    impl TestEnv {
        fn new() -> Self {
            let _lock = env_lock();
            let home = tempdir().expect("home tempdir");
            let home_str = home.path().to_str().expect("home path");
            let guards = vec![
                EnvGuard::set("HOME", Some(home_str)),
                EnvGuard::set("USERPROFILE", Some(home_str)),
                EnvGuard::set("XDG_DATA_HOME", Some(home_str)),
            ];
            Self {
                _lock,
                _guards: guards,
                home,
            }
        }

        fn svm_home(&self) -> PathBuf {
            let _ = self.home.path();
            Solc::svm_home().expect("svm home")
        }
    }

    fn write_svm_stub(svm_home: &Path, version: &Version) -> PathBuf {
        let version_dir = svm_home.join(version.to_string());
        fs::create_dir_all(&version_dir).expect("svm version dir");
        let solc_path = version_dir.join(format!("solc-{version}"));
        fs::write(&solc_path, "").expect("svm solc stub");
        solc_path
    }

    #[test]
    fn install_queue_depth_uses_env_override_and_fallback() {
        let _serial = test_guard();
        let _lock = env_lock();
        let default = thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1);

        {
            let _guard = EnvGuard::set("SOLIDITY_ANALYZER_INSTALL_CONCURRENCY", Some("2"));
            assert_eq!(install_queue_depth(), 2);
        }
        {
            let _guard = EnvGuard::set("SOLIDITY_ANALYZER_INSTALL_CONCURRENCY", Some("0"));
            assert_eq!(install_queue_depth(), default);
        }
        {
            let _guard = EnvGuard::set("SOLIDITY_ANALYZER_INSTALL_CONCURRENCY", Some("bogus"));
            assert_eq!(install_queue_depth(), default);
        }
    }

    #[test]
    fn resolve_executable_prefers_direct_path_and_searches_path() {
        let _serial = test_guard();
        let _lock = env_lock();

        let temp = tempdir().expect("temp dir");
        let exe_name = if cfg!(windows) { "solc.exe" } else { "solc" };
        let exe_path = temp.path().join(exe_name);
        fs::write(&exe_path, "").expect("write solc");

        let direct = exe_path.to_string_lossy().to_string();
        assert_eq!(resolve_executable(&direct), Some(exe_path.clone()));

        let _guard = EnvGuard::set("PATH", Some(temp.path().to_str().expect("path")));
        assert_eq!(resolve_executable("solc"), Some(exe_path.clone()));

        let missing = temp.path().join("missing").join("solc");
        assert_eq!(resolve_executable(missing.to_string_lossy().as_ref()), None);
    }

    #[test]
    fn register_install_waits_for_existing_install_and_returns_waited() {
        let _serial = test_guard();
        let env = TestEnv::new();
        let version = Version::parse("9.9.9").expect("version");
        let _solc_path = write_svm_stub(&env.svm_home(), &version);

        let versions = INSTALLING_VERSIONS.get_or_init(|| Mutex::new(HashSet::new()));
        {
            let mut guard = versions.lock().expect("installing versions");
            guard.insert(version.clone());
        }

        let version_clone = version.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(60));
            finish_install(&version_clone);
        });

        let outcome = register_install(&version).expect("register install");
        assert!(matches!(outcome, RegisterOutcome::Waited));
        finish_install(&version);
    }

    #[test]
    fn register_install_acquires_when_uncontended() {
        let _serial = test_guard();
        let _env = TestEnv::new();
        let version = Version::parse("9.9.8").expect("version");

        finish_install(&version);
        let outcome = register_install(&version).expect("register install");
        assert!(matches!(outcome, RegisterOutcome::Acquired));
        finish_install(&version);
    }

    #[test]
    fn install_versions_dedupes_and_reports_already_installed() {
        let _serial = test_guard();
        let env = TestEnv::new();
        let version_a = Version::parse("9.9.1").expect("version");
        let version_b = Version::parse("9.9.2").expect("version");
        write_svm_stub(&env.svm_home(), &version_a);
        write_svm_stub(&env.svm_home(), &version_b);

        let (installed, already) = install_versions(vec![
            version_a.clone(),
            version_a.clone(),
            version_b.clone(),
        ])
        .expect("install versions");

        assert!(installed.is_empty());
        assert!(already.contains(&version_a));
        assert!(already.contains(&version_b));
        assert_eq!(already.len(), 2);
    }

    #[test]
    fn format_install_summary_handles_empty_and_mixed_lists() {
        let _serial = test_guard();
        let summary = format_install_summary("test", &[], &[]);
        assert!(summary.contains("nothing to install"));

        let installed = [Version::parse("1.2.3").expect("version")];
        let summary = format_install_summary("test", &installed, &[]);
        assert!(summary.contains("installed"));

        let already = [Version::parse("2.0.0").expect("version")];
        let summary = format_install_summary("test", &[], &already);
        assert!(summary.contains("already installed"));
    }
}
