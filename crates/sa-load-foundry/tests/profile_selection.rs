use sa_load_foundry::load_foundry;
use sa_test_support::setup_foundry_root;
use sa_test_utils::{EnvGuard, env_lock};
use std::fs;
use std::path::Path;
use std::sync::MutexGuard;
use tempfile::tempdir;

struct TestEnv {
    _lock: MutexGuard<'static, ()>,
    _guards: Vec<EnvGuard>,
}

impl TestEnv {
    fn new() -> Self {
        let lock = env_lock();
        let guards = vec![
            EnvGuard::unset("FOUNDRY_PROFILE"),
            EnvGuard::unset("FOUNDRY_SOLC_VERSION"),
            EnvGuard::unset("DAPP_SOLC_VERSION"),
            EnvGuard::unset("FOUNDRY_CONFIG"),
            EnvGuard::unset("FOUNDRY_OPTIMIZER_RUNS"),
            EnvGuard::unset("DAPP_BUILD_OPTIMIZE_RUNS"),
        ];
        Self {
            _lock: lock,
            _guards: guards,
        }
    }

    fn with_env(mut self, key: &'static str, value: Option<&str>) -> Self {
        self._guards.push(EnvGuard::set(key, value));
        self
    }
}

fn write_foundry_toml(root: &Path, contents: &str) {
    fs::write(root.join("foundry.toml"), contents).expect("write foundry.toml");
}

#[test]
fn selects_profile_from_env() {
    let env = TestEnv::new().with_env("FOUNDRY_PROFILE", Some("dev"));

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let foundry_toml = r#"
[profile.default]
optimizer_runs = 200

[profile.dev]
optimizer_runs = 777
"#;
    write_foundry_toml(root, foundry_toml);

    let resolved = load_foundry(root, None).expect("load config");
    assert_eq!(resolved.active_profile().name(), "dev");
    drop(env);
}

#[test]
fn explicit_profile_arg_overrides_env() {
    let env = TestEnv::new().with_env("FOUNDRY_PROFILE", Some("dev"));

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let foundry_toml = r#"
[profile.default]
optimizer_runs = 111

[profile.dev]
optimizer_runs = 222
"#;
    write_foundry_toml(root, foundry_toml);

    let resolved = load_foundry(root, Some("default")).expect("load config");
    assert_eq!(resolved.active_profile().name(), "default");
    drop(env);
}

#[test]
fn env_solc_version_overrides_profile_config() {
    let env = TestEnv::new()
        .with_env("FOUNDRY_PROFILE", Some("dev"))
        .with_env("FOUNDRY_SOLC_VERSION", Some("0.8.22"));

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let foundry_toml = r#"
[profile.default]
solc = "0.8.20"
"#;
    write_foundry_toml(root, foundry_toml);

    let resolved = load_foundry(root, Some("dev")).expect("load config");
    let active = resolved.active_profile();

    assert_eq!(active.name(), "dev");
    assert_eq!(active.solc_version(), Some("0.8.22"));
    drop(env);
}

#[test]
fn empty_profile_env_uses_default() {
    let env = TestEnv::new().with_env("FOUNDRY_PROFILE", Some(""));

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let foundry_toml = r#"
[profile.default]
optimizer_runs = 101

[profile.dev]
optimizer_runs = 202
"#;
    write_foundry_toml(root, foundry_toml);

    let resolved = load_foundry(root, None).expect("load config");
    assert_eq!(resolved.active_profile().name(), "default");
    drop(env);
}

#[test]
fn dapp_solc_version_overrides_config_when_foundry_unset() {
    let _env = TestEnv::new().with_env("DAPP_SOLC_VERSION", Some("0.8.19"));

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let foundry_toml = r#"
[profile.default]
solc = "0.8.20"
"#;
    write_foundry_toml(root, foundry_toml);

    let resolved = load_foundry(root, None).expect("load config");
    assert_eq!(resolved.active_profile().solc_version(), Some("0.8.19"));
}

#[test]
fn foundry_solc_version_takes_precedence_over_dapp() {
    let _env = TestEnv::new()
        .with_env("FOUNDRY_SOLC_VERSION", Some("0.8.22"))
        .with_env("DAPP_SOLC_VERSION", Some("0.8.18"));

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let foundry_toml = r#"
[profile.default]
solc = "0.8.20"
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let resolved = load_foundry(root, None).expect("load config");
    assert_eq!(resolved.active_profile().solc_version(), Some("0.8.22"));
}

#[test]
fn foundry_config_env_overrides_root_toml() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let root_toml = r#"
[profile.default]
optimizer_runs = 111
"#;
    fs::write(root.join("foundry.toml"), root_toml).expect("write root foundry.toml");

    let custom_path = root.join("custom-foundry.toml");
    let custom_toml = r#"
[profile.default]
optimizer_runs = 999
"#;
    fs::write(&custom_path, custom_toml).expect("write custom foundry.toml");

    let custom_path = custom_path.to_string_lossy().to_string();
    let _env = TestEnv::new().with_env("FOUNDRY_CONFIG", Some(custom_path.as_str()));

    let resolved = load_foundry(root, None).expect("load config");
    let settings = resolved.compiler_settings().expect("compiler settings");

    assert_eq!(settings.solc.optimizer.runs, Some(999));
}

#[test]
fn load_foundry_reports_profile_context_on_error() {
    let _env = TestEnv::new();

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let foundry_toml = r#"
[profile.default
solc = "0.8.20"
"#;
    write_foundry_toml(root, foundry_toml);

    let err = load_foundry(root, Some("dev")).expect_err("expected load error");
    assert!(
        err.to_string()
            .contains("failed to load foundry config for profile dev")
    );
}

#[test]
fn load_foundry_reports_generic_context_on_error() {
    let _env = TestEnv::new();

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let foundry_toml = r#"
[profile.default
solc = "0.8.20"
"#;
    write_foundry_toml(root, foundry_toml);

    let err = load_foundry(root, None).expect_err("expected load error");
    assert!(err.to_string().contains("failed to load foundry config"));
}

#[test]
fn local_solc_paths_and_context_remappings_are_preserved() {
    let _env = TestEnv::new();

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let solc_path = root.join("bin/solc");
    fs::create_dir_all(solc_path.parent().expect("solc parent")).expect("solc dir");
    fs::write(&solc_path, "").expect("write solc");
    let solc_path = solc_path.to_string_lossy().replace('\\', "/");

    let foundry_toml = format!(
        r#"
[profile.default]
solc = "{solc_path}"
remappings = ["lib/foo:dep/=lib/foo/dep/"]
"#
    );
    write_foundry_toml(root, &foundry_toml);

    let resolved = load_foundry(root, None).expect("load config");
    let active = resolved.active_profile();

    assert_eq!(active.solc_version(), Some(solc_path.as_str()));
    assert_eq!(active.remappings().len(), 1);
    assert_eq!(active.remappings()[0].context(), Some("lib/foo"));
}
