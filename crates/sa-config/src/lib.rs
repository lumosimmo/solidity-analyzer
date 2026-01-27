use std::path::PathBuf;

use foundry_compilers::ProjectPathsConfig;
use foundry_compilers::artifacts::remappings::Remapping as FoundryRemapping;
use foundry_compilers::compilers::multi::MultiCompilerSettings;
use foundry_compilers::error::SolcError;
use foundry_compilers::solc::Solc;
use foundry_config::fmt::FormatterConfig;
use foundry_config::{Config, figment::Profile};
use sa_project_model::{FoundryProfile, FoundryWorkspace};
use solar_config::{ImportRemapping, Opts as SolarOpts};

mod formatting;

pub use formatting::formatter_config;

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFoundryConfig {
    workspace: FoundryWorkspace,
    active_profile: FoundryProfile,
    formatter: FormatterConfig,
    foundry_config: Config,
}

impl ResolvedFoundryConfig {
    pub fn new(workspace: FoundryWorkspace, active_profile: FoundryProfile) -> Self {
        let root = PathBuf::from(workspace.root().as_str());
        let mut foundry_config = Config::with_root(root).sanitized();
        sync_profile(&mut foundry_config, &active_profile);
        Self {
            workspace,
            active_profile,
            formatter: FormatterConfig::default(),
            foundry_config,
        }
    }

    pub fn workspace(&self) -> &FoundryWorkspace {
        &self.workspace
    }

    pub fn active_profile(&self) -> &FoundryProfile {
        &self.active_profile
    }

    pub fn formatter_config(&self) -> &FormatterConfig {
        &self.formatter
    }

    pub fn foundry_config(&self) -> &Config {
        &self.foundry_config
    }

    pub fn compiler_settings(&self) -> Result<MultiCompilerSettings, SolcError> {
        self.foundry_config.compiler_settings()
    }

    pub fn project_paths(&self) -> ProjectPathsConfig<Solc> {
        self.foundry_config.project_paths::<Solc>()
    }

    pub fn with_formatter_config(mut self, formatter: FormatterConfig) -> Self {
        self.formatter = formatter;
        self
    }

    pub fn with_foundry_config(mut self, config: Config) -> Self {
        let mut config = config;
        sync_profile(&mut config, &self.active_profile);
        self.foundry_config = config;
        self
    }
}

pub fn solar_opts_from_config(config: &ResolvedFoundryConfig) -> SolarOpts {
    let project_paths = config.project_paths();
    SolarOpts {
        base_path: Some(project_paths.root.clone()),
        include_paths: project_paths.include_paths.iter().cloned().collect(),
        allow_paths: project_paths.allowed_paths.iter().cloned().collect(),
        import_remappings: solar_remappings(&project_paths.remappings),
        ..SolarOpts::default()
    }
}

fn solar_remappings(remappings: &[FoundryRemapping]) -> Vec<ImportRemapping> {
    remappings
        .iter()
        .map(|remapping| ImportRemapping {
            context: remapping.context.clone().unwrap_or_default(),
            prefix: remapping.name.clone(),
            path: remapping.path.clone(),
        })
        .collect()
}

fn sync_profile(config: &mut Config, profile: &FoundryProfile) {
    let selected = Profile::new(profile.name());
    config.profile = selected.clone();
    if !config.profiles.contains(&selected) {
        config.profiles.push(selected);
    }
}

#[cfg(test)]
mod tests {
    use sa_paths::NormalizedPath;
    use sa_project_model::{FoundryProfile, FoundryWorkspace};

    use super::ResolvedFoundryConfig;

    #[test]
    fn resolved_config_exposes_workspace_and_profile() {
        let root = NormalizedPath::new("/workspace");
        let default_profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root);
        let config = ResolvedFoundryConfig::new(workspace.clone(), default_profile.clone());

        assert_eq!(config.workspace(), &workspace);
        assert_eq!(config.active_profile(), &default_profile);
    }
}
