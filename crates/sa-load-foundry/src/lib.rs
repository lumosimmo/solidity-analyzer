use std::path::{Path, PathBuf};
use std::{env, mem};

use anyhow::Context;
use foundry_config::{Config, SolcReq};
use sa_config::ResolvedFoundryConfig;
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryWorkspace, Remapping};

pub fn load_foundry(root: &Path, profile: Option<&str>) -> anyhow::Result<ResolvedFoundryConfig> {
    let profile_name = profile
        .map(str::to_string)
        .or_else(|| std::env::var("FOUNDRY_PROFILE").ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string());
    let active_config = load_config_with_profile(root, Some(&profile_name))?;

    let root_path = active_config.root.clone();
    let root_normalized = NormalizedPath::new(root_path.to_string_lossy());
    let src = normalize_path(&root_path, &active_config.src);
    let test = normalize_path(&root_path, &active_config.test);
    let script = normalize_path(&root_path, &active_config.script);
    let lib = normalize_lib_path(&root_path, &active_config.libs);

    let workspace = FoundryWorkspace::from_paths(root_normalized, src, lib, test, script);

    let formatter = active_config.fmt.clone();

    let active_profile = profile_from_config(&profile_name, &active_config);
    Ok(ResolvedFoundryConfig::new(workspace, active_profile)
        .with_formatter_config(formatter)
        .with_foundry_config(active_config))
}

fn load_config_with_profile(root: &Path, profile: Option<&str>) -> anyhow::Result<Config> {
    let _guard = profile.map(ProfileEnvGuard::set);
    let config = Config::load_with_root(root).with_context(|| match profile {
        Some(profile) => format!("failed to load foundry config for profile {profile}"),
        None => "failed to load foundry config".to_string(),
    })?;
    Ok(config.sanitized())
}

fn normalize_path(root: &Path, path: &Path) -> NormalizedPath {
    let joined = root.join(path);
    NormalizedPath::new(joined.to_string_lossy())
}

fn normalize_lib_path(root: &Path, libs: &[PathBuf]) -> NormalizedPath {
    let lib = libs
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("lib"));
    normalize_path(root, &lib)
}

fn profile_from_config(profile: &str, config: &Config) -> FoundryProfile {
    let remappings: Vec<Remapping> = config
        .remappings
        .iter()
        .map(Remapping::from_relative)
        .collect();
    let mut profile = FoundryProfile::new(profile);

    if let Some(solc) = &config.solc {
        profile = profile.with_solc_version(solc_version(solc));
    }

    if !remappings.is_empty() {
        profile = profile.with_remappings(remappings);
    }

    profile
}

fn solc_version(solc: &SolcReq) -> String {
    match solc {
        SolcReq::Version(version) => version.to_string(),
        SolcReq::Local(path) => path.to_string_lossy().to_string(),
    }
}

struct ProfileEnvGuard {
    previous: Option<String>,
}

impl ProfileEnvGuard {
    fn set(profile: &str) -> Self {
        let previous = env::var("FOUNDRY_PROFILE").ok();
        unsafe {
            env::set_var("FOUNDRY_PROFILE", profile);
        }
        Self { previous }
    }
}

impl Drop for ProfileEnvGuard {
    fn drop(&mut self) {
        match mem::take(&mut self.previous) {
            Some(value) => unsafe {
                env::set_var("FOUNDRY_PROFILE", value);
            },
            None => unsafe {
                env::remove_var("FOUNDRY_PROFILE");
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use sa_test_support::setup_foundry_root;
    use sa_test_utils::{EnvGuard, env_lock};
    use std::fs;
    use tempfile::tempdir;

    use super::load_foundry;

    #[test]
    fn loads_foundry_config_and_profiles() {
        let _lock = env_lock();
        let _guard = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
        let _dapp_guard = EnvGuard::set("DAPP_SOLC_VERSION", None);
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        setup_foundry_root(root);

        let foundry_toml = r#"
[profile.default]
solc = "0.8.20"
remappings = ["lib/=lib/forge-std/src/"]

[profile.dev]
remappings = ["src/=src/overrides/"]
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let resolved = load_foundry(root, Some("dev")).expect("load config");
        let active = resolved.active_profile();

        assert_eq!(active.name(), "dev");
        assert_eq!(active.solc_version(), Some("0.8.20"));
        assert!(!active.remappings().is_empty());
    }

    #[test]
    fn env_solc_version_overrides_config() {
        let _lock = env_lock();
        let _guard = EnvGuard::set("FOUNDRY_SOLC_VERSION", Some("0.7.6"));
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        setup_foundry_root(root);

        let foundry_toml = r#"
[profile.default]
solc = "0.8.20"
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let resolved = load_foundry(root, None).expect("load config");
        let active = resolved.active_profile();

        assert_eq!(active.solc_version(), Some("0.7.6"));
    }

    #[test]
    fn solc_version_is_accepted_for_backward_compatibility() {
        let _lock = env_lock();
        let _guard = EnvGuard::set("FOUNDRY_SOLC_VERSION", None);
        let dir = tempdir().expect("tempdir");
        let root = dir.path();

        setup_foundry_root(root);

        let foundry_toml = r#"
[profile.default]
solc_version = "0.8.17"
"#;
        fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

        let resolved = load_foundry(root, None).expect("load config");
        let active = resolved.active_profile();

        assert_eq!(active.solc_version(), Some("0.8.17"));
    }
}
