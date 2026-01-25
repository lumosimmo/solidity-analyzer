use foundry_compilers::solc::Solc;
use foundry_config::Config;
use sa_config::ResolvedFoundryConfig;
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryWorkspace};
use sa_test_support::setup_foundry_root;
use sa_test_utils::{EnvGuard, env_lock};
use std::fs;
use tempfile::tempdir;

#[test]
fn resolved_config_uses_foundry_compiler_settings_and_paths() {
    let _lock = env_lock();
    let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");

    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    setup_foundry_root(root);

    let foundry_toml = r#"
[profile.default]
optimizer = true
optimizer_runs = 123
via_ir = true
evm_version = "paris"
libs = ["lib"]
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let expected = Config::load_with_root(root)
        .expect("load foundry config")
        .sanitized();

    let root_path = NormalizedPath::new(root.to_string_lossy());
    let profile = FoundryProfile::new("default");
    let workspace = FoundryWorkspace::new(root_path, profile.clone());
    let resolved =
        ResolvedFoundryConfig::new(workspace, profile).with_foundry_config(expected.clone());

    assert_eq!(
        resolved.compiler_settings().expect("compiler settings"),
        expected.compiler_settings().expect("compiler settings")
    );

    let resolved_paths = resolved.project_paths();
    let expected_paths = expected.project_paths::<Solc>();
    assert_eq!(resolved_paths.sources, expected_paths.sources);
    assert_eq!(resolved_paths.libraries, expected_paths.libraries);
}
