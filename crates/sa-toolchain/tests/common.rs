#![allow(dead_code)]

use std::env;
use std::path::{Path, PathBuf};
use std::sync::{MutexGuard, OnceLock};

use sa_test_utils::{EnvGuard, env_lock};

static SVM_HOME: OnceLock<PathBuf> = OnceLock::new();

pub struct ToolchainTestEnv {
    _lock: MutexGuard<'static, ()>,
    _guards: Vec<EnvGuard>,
}

impl ToolchainTestEnv {
    pub fn new() -> Self {
        let lock = env_lock();
        let guards = vec![
            EnvGuard::unset("FOUNDRY_PROFILE"),
            EnvGuard::unset("FOUNDRY_SOLC_VERSION"),
            EnvGuard::unset("DAPP_SOLC_VERSION"),
            EnvGuard::unset("FOUNDRY_CONFIG"),
        ];
        Self {
            _lock: lock,
            _guards: guards,
        }
    }

    pub fn with_home(mut self, home: &Path) -> Self {
        let home_str = home.to_str().expect("home path");
        self._guards.push(EnvGuard::set("HOME", Some(home_str)));
        self._guards
            .push(EnvGuard::set("USERPROFILE", Some(home_str)));
        self._guards
            .push(EnvGuard::set("XDG_DATA_HOME", Some(home_str)));
        self
    }

    pub fn with_svm_home(self) -> Self {
        self.with_home(shared_svm_home())
    }

    pub fn with_path_prepend(mut self, dir: &Path) -> Self {
        let mut paths = vec![dir.to_path_buf()];
        if let Some(existing) = env::var_os("PATH") {
            paths.extend(env::split_paths(&existing));
        }
        let joined = env::join_paths(paths).expect("join PATH");
        let joined = joined.to_string_lossy().to_string();
        self._guards.push(EnvGuard::set("PATH", Some(&joined)));
        self
    }
}

impl Default for ToolchainTestEnv {
    fn default() -> Self {
        Self::new()
    }
}

pub fn toml_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

pub fn write_svm_solc(home: &Path, version: &str) -> PathBuf {
    let svm_dir = home.join(".svm").join(version);
    std::fs::create_dir_all(&svm_dir).expect("svm dir");
    let solc_path = svm_dir.join(format!("solc-{version}"));
    std::fs::write(&solc_path, "").expect("write svm solc");
    solc_path
}

pub fn svm_home() -> &'static Path {
    shared_svm_home()
}

fn shared_svm_home() -> &'static Path {
    SVM_HOME
        .get_or_init(|| {
            let dir = tempfile::tempdir().expect("svm tempdir");
            dir.keep()
        })
        .as_path()
}
