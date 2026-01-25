use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use anyhow::{Context, Result, anyhow};
use log::debug;
use sa_config::ResolvedFoundryConfig;
use sa_ide::{Analysis, AnalysisChange, AnalysisHost};
use sa_load_foundry::load_foundry;
use sa_paths::NormalizedPath;
use sa_test_support::setup_foundry_root;
use sa_vfs::{FileId, Vfs, VfsChange, VfsSnapshot};
use tempfile::TempDir;
use walkdir::WalkDir;

pub mod lsp {
    pub use sa_test_support::lsp::*;
}
pub mod toolchain;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Environment mutations via `std::env::set_var`/`std::env::remove_var` must be
/// serialized with `env_lock()`, while read-only lookups can proceed without
/// locking.
///
/// Global invariant: any environment mutation in the process must acquire the
/// `MutexGuard<'static, ()>` from `env_lock()` so `ENV_LOCK` serializes writers
/// and writer+reader access; read-only helpers like `slow_tests_enabled()` do
/// not need the lock.
pub fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

/// Restores an environment variable to its previous value on drop.
pub struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    pub fn set(key: &'static str, value: Option<&str>) -> Self {
        let previous = std::env::var(key).ok();
        match value {
            Some(value) => unsafe {
                std::env::set_var(key, value);
            },
            None => unsafe {
                std::env::remove_var(key);
            },
        }
        Self { key, previous }
    }

    pub fn unset(key: &'static str) -> Self {
        Self::set(key, None)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

/// Reads only; safe to call without `env_lock()`.
///
/// If both `SKIP_SLOW_TESTS` and `RUN_SLOW_TESTS` are set, `SKIP_SLOW_TESTS`
/// takes precedence and this returns `false`. Otherwise it returns `true` only
/// when `RUN_SLOW_TESTS` is set.
pub fn slow_tests_enabled() -> bool {
    if std::env::var("SKIP_SLOW_TESTS").is_ok() {
        return false;
    }
    std::env::var("RUN_SLOW_TESTS").is_ok()
}

pub fn skip_slow_tests() -> bool {
    !slow_tests_enabled()
}

#[derive(Debug)]
struct FixtureFile {
    path: PathBuf,
    text: String,
}

pub struct FixtureBuilder {
    root: TempDir,
    files: Vec<FixtureFile>,
    foundry_toml: Option<String>,
}

impl FixtureBuilder {
    pub fn new() -> std::io::Result<Self> {
        let root = tempfile::tempdir()?;
        Ok(Self {
            root,
            files: Vec::new(),
            foundry_toml: None,
        })
    }

    pub fn file(mut self, path: impl AsRef<Path>, text: impl Into<String>) -> Self {
        self.files.push(FixtureFile {
            path: path.as_ref().to_path_buf(),
            text: text.into(),
        });
        self
    }

    pub fn foundry_toml(mut self, text: impl Into<String>) -> Self {
        self.foundry_toml = Some(text.into());
        self
    }

    pub fn build(self) -> Result<Fixture> {
        let root_path = self.root.path().to_path_buf();
        setup_foundry_root(&root_path);
        let foundry_toml = self.foundry_toml.unwrap_or_else(default_foundry_toml);
        fs::write(root_path.join("foundry.toml"), foundry_toml)
            .with_context(|| "write foundry.toml")?;

        for file in self.files {
            validate_fixture_path(&file.path)?;
            let path = root_path.join(file.path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create fixture dir {}", parent.display()))?;
            }
            fs::write(&path, file.text)
                .with_context(|| format!("write fixture file {}", path.display()))?;
        }

        Fixture::load(root_path, Some(self.root))
    }
}

impl Default for FixtureBuilder {
    fn default() -> Self {
        Self::new().expect("tempdir")
    }
}

pub struct Fixture {
    root: PathBuf,
    config: ResolvedFoundryConfig,
    vfs: VfsSnapshot,
    _temp: Option<TempDir>,
}

impl Fixture {
    pub fn from_dir(root: impl AsRef<Path>) -> Result<Self> {
        Self::load(root.as_ref().to_path_buf(), None)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> &ResolvedFoundryConfig {
        &self.config
    }

    pub fn vfs_snapshot(&self) -> &VfsSnapshot {
        &self.vfs
    }

    pub fn analysis_host(&self) -> AnalysisHost {
        let mut host = AnalysisHost::new();
        let mut change = AnalysisChange::new();
        change.set_vfs(self.vfs.clone());
        change.set_config(self.config.clone());
        host.apply_change(change);
        host
    }

    pub fn analysis(&self) -> Analysis {
        self.analysis_host().snapshot()
    }

    pub fn analysis_snapshot(&self) -> (Analysis, VfsSnapshot) {
        (self.analysis(), self.vfs.clone())
    }

    pub fn file_id(&self, relative: impl AsRef<Path>) -> Option<FileId> {
        let path = self.normalized_path(relative)?;
        let file_id = self.vfs.file_id(&path);
        if file_id.is_none() {
            debug!("fixture file id missing for path: {}", path);
        }
        file_id
    }

    pub fn normalized_path(&self, relative: impl AsRef<Path>) -> Option<NormalizedPath> {
        let relative = relative.as_ref();
        if let Err(error) = validate_fixture_path(relative) {
            debug!(
                "fixture path rejected by validation: {} ({:?})",
                relative.display(),
                error
            );
            return None;
        }
        let path = self.root.join(relative);
        let path_str = match path.to_str() {
            Some(path_str) => path_str,
            None => {
                debug!("fixture path is not valid UTF-8: {}", path.display());
                return None;
            }
        };
        Some(NormalizedPath::new(path_str))
    }

    fn load(root: PathBuf, temp: Option<TempDir>) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("canonicalize fixture root {}", root.display()))?;
        let config = load_foundry_config(&root, None)?;
        let vfs = load_fixture_vfs(&config)?;
        Ok(Self {
            root,
            config,
            vfs,
            _temp: temp,
        })
    }
}

pub fn load_foundry_config(root: &Path, profile: Option<&str>) -> Result<ResolvedFoundryConfig> {
    load_foundry(root, profile)
        .with_context(|| format!("load foundry config at {}", root.display()))
}

fn validate_fixture_path(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(anyhow!(
            "fixture path must be relative and not contain '..': {}",
            path.display()
        ));
    }
    Ok(())
}

fn default_foundry_toml() -> String {
    "[profile.default]\n".to_string()
}

fn load_fixture_vfs(config: &ResolvedFoundryConfig) -> Result<VfsSnapshot> {
    let workspace = config.workspace();
    let mut files = Vec::new();
    let roots = [
        workspace.src().as_str(),
        workspace.lib().as_str(),
        workspace.test().as_str(),
        workspace.script().as_str(),
    ];

    for root in roots {
        let path = PathBuf::from(root);
        collect_solidity_files(&path, &mut files)?;
    }

    let mut vfs = Vfs::default();
    for path in files {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("read fixture file {}", path.display()))?;
        let normalized = NormalizedPath::new(path.to_string_lossy());
        vfs.apply_change(VfsChange::Set {
            path: normalized,
            text: Arc::from(text),
        });
    }

    Ok(vfs.snapshot())
}

fn collect_solidity_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(dir).sort_by_file_name() {
        let entry = entry.with_context(|| format!("read entry in {}", dir.display()))?;
        if entry.file_type().is_file() {
            let path = entry.path();
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sol"))
            {
                files.push(path.to_path_buf());
            }
        }
    }

    Ok(())
}
