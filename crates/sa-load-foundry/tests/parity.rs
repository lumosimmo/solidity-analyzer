use foundry_compilers::solc::Solc;
use foundry_config::Config;
use sa_load_foundry::load_foundry;
use sa_test_support::setup_foundry_root_with_extras;
use sa_test_utils::{EnvGuard, env_lock};
use std::fs;
use tempfile::tempdir;

#[test]
fn resolved_config_matches_foundry_settings_and_paths() {
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
remappings = ["forge-std/=lib/forge-std/src/", "@oz/=lib/openzeppelin-contracts/"]
optimizer = true
optimizer_runs = 777
via_ir = true
evm_version = "paris"
allow_paths = ["allow"]
include_paths = ["include"]
cache_path = "cache"
out = "out"
build_info_path = "out/build-info"
extra_args = ["--foo", "--bar=1"]
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let resolved = load_foundry(root, None).expect("load config");
    let expected = Config::load_with_root(root)
        .expect("load foundry config")
        .sanitized();

    let resolved_settings = resolved.compiler_settings().expect("compiler settings");
    let expected_settings = expected.compiler_settings().expect("compiler settings");
    assert_eq!(resolved_settings, expected_settings);

    let resolved_paths = resolved.project_paths();
    let expected_paths = expected.project_paths::<Solc>();

    assert_eq!(resolved_paths.root, expected_paths.root);
    assert_eq!(resolved_paths.sources, expected_paths.sources);
    assert_eq!(resolved_paths.tests, expected_paths.tests);
    assert_eq!(resolved_paths.scripts, expected_paths.scripts);
    assert_eq!(resolved_paths.libraries, expected_paths.libraries);
    assert_eq!(resolved_paths.remappings, expected_paths.remappings);
    assert_eq!(resolved_paths.allowed_paths, expected_paths.allowed_paths);
    assert_eq!(resolved_paths.include_paths, expected_paths.include_paths);
    assert_eq!(resolved_paths.cache, expected_paths.cache);
    assert_eq!(resolved_paths.artifacts, expected_paths.artifacts);
    assert_eq!(resolved_paths.build_infos, expected_paths.build_infos);
}

#[test]
fn local_config_overrides_global_config() {
    let _lock = env_lock();
    let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
    let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
    let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
    let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");
    let _opt_guard = EnvGuard::unset("FOUNDRY_OPTIMIZER_RUNS");
    let _dapp_opt_guard = EnvGuard::unset("DAPP_BUILD_OPTIMIZE_RUNS");

    let home = tempdir().expect("tempdir");
    let home_path = home.path();
    let home_str = home_path.to_str().expect("home path");
    let _home_guard = EnvGuard::set("HOME", Some(home_str));
    let _userprofile_guard = EnvGuard::set("USERPROFILE", Some(home_str));

    let global_dir = home_path.join(".foundry");
    fs::create_dir_all(&global_dir).expect("global foundry dir");
    let global_toml = r#"
[profile.default]
optimizer_runs = 111
"#;
    fs::write(global_dir.join("foundry.toml"), global_toml).expect("write global foundry.toml");

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root_with_extras(root);
    let local_toml = r#"
[profile.default]
optimizer_runs = 222
"#;
    fs::write(root.join("foundry.toml"), local_toml).expect("write local foundry.toml");

    let resolved = load_foundry(root, None).expect("load config");
    let expected = Config::load_with_root(root)
        .expect("load foundry config")
        .sanitized();

    let resolved_settings = resolved.compiler_settings().expect("compiler settings");
    let expected_settings = expected.compiler_settings().expect("compiler settings");

    assert_eq!(resolved_settings, expected_settings);
    assert_eq!(resolved_settings.solc.optimizer.runs, Some(222));
}

#[test]
fn foundry_env_overrides_toml() {
    let _lock = env_lock();
    let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
    let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
    let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
    let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");
    let _dapp_opt_guard = EnvGuard::unset("DAPP_BUILD_OPTIMIZE_RUNS");
    let _opt_guard = EnvGuard::set("FOUNDRY_OPTIMIZER_RUNS", Some("999"));

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root_with_extras(root);

    let foundry_toml = r#"
[profile.default]
optimizer_runs = 200
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let resolved = load_foundry(root, None).expect("load config");
    let expected = Config::load_with_root(root)
        .expect("load foundry config")
        .sanitized();

    let resolved_settings = resolved.compiler_settings().expect("compiler settings");
    let expected_settings = expected.compiler_settings().expect("compiler settings");

    assert_eq!(resolved_settings, expected_settings);
    assert_eq!(resolved_settings.solc.optimizer.runs, Some(999));
}

#[test]
fn dapp_env_overrides_toml() {
    let _lock = env_lock();
    let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
    let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
    let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
    let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");
    let _opt_guard = EnvGuard::unset("FOUNDRY_OPTIMIZER_RUNS");
    let _dapp_opt_guard = EnvGuard::set("DAPP_BUILD_OPTIMIZE_RUNS", Some("333"));

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root_with_extras(root);

    let foundry_toml = r#"
[profile.default]
optimizer_runs = 200
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let resolved = load_foundry(root, None).expect("load config");
    let expected = Config::load_with_root(root)
        .expect("load foundry config")
        .sanitized();

    let resolved_settings = resolved.compiler_settings().expect("compiler settings");
    let expected_settings = expected.compiler_settings().expect("compiler settings");

    assert_eq!(resolved_settings, expected_settings);
    assert_eq!(resolved_settings.solc.optimizer.runs, Some(333));
}

#[test]
fn optimizer_defaults_match_foundry() {
    let _lock = env_lock();
    let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
    let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
    let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");
    let _config_guard = EnvGuard::unset("FOUNDRY_CONFIG");
    let _opt_guard = EnvGuard::unset("FOUNDRY_OPTIMIZER_RUNS");
    let _dapp_opt_guard = EnvGuard::unset("DAPP_BUILD_OPTIMIZE_RUNS");

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root_with_extras(root);

    let foundry_toml = r#"
[profile.default]
src = "src"
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let resolved = load_foundry(root, None).expect("load config");
    let expected = Config::load_with_root(root)
        .expect("load foundry config")
        .sanitized();

    let resolved_settings = resolved.compiler_settings().expect("compiler settings");
    let expected_settings = expected.compiler_settings().expect("compiler settings");

    assert_eq!(resolved_settings, expected_settings);
    assert_eq!(resolved_settings.solc.optimizer.enabled, Some(false));
    assert_eq!(resolved_settings.solc.optimizer.runs, Some(200));
}
