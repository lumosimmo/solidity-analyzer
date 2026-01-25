use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
/// Configuration for LSP features.
///
/// On save, diagnostics and linting are controlled independently:
/// `DiagnosticsConfig::enable`/`DiagnosticsConfig::on_save` gate diagnostics on save,
/// `DiagnosticsConfig::on_change` runs diagnostics on every change/keystroke, and
/// `LintConfig::enable`/`LintConfig::on_save` gate lint diagnostics.
pub struct LspConfig {
    pub diagnostics: DiagnosticsConfig,
    pub format: FormatConfig,
    pub lint: LintConfig,
    pub toolchain: ToolchainConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
/// Controls when diagnostics are computed and published.
///
/// Defaults to enabling diagnostics on save and on change.
pub struct DiagnosticsConfig {
    /// Globally enables diagnostics. When false, diagnostics are disabled regardless of
    /// `on_save` or `on_change`. Defaults to true.
    pub enable: bool,
    /// Runs diagnostics after file save. Defaults to true.
    pub on_save: bool,
    /// Runs diagnostics on file edits/keystrokes. Defaults to true.
    pub on_change: bool,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            enable: true,
            on_save: true,
            on_change: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct FormatConfig {
    pub on_save: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LintConfig {
    /// Globally enables lint diagnostics. When false, linting is disabled regardless of
    /// `on_save` or `on_change`. Defaults to true.
    pub enable: bool,
    /// Runs lint diagnostics after file save. Defaults to true.
    pub on_save: bool,
    /// Runs lint diagnostics on file edits/keystrokes. Defaults to false.
    pub on_change: bool,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            enable: true,
            on_save: true,
            on_change: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolchainConfig {
    /// Prompt to install solc when missing. Defaults to true.
    pub prompt_install: bool,
    /// Override the maximum number of parallel solc jobs. Defaults to None.
    pub solc_jobs: Option<usize>,
}

impl Default for ToolchainConfig {
    fn default() -> Self {
        Self {
            prompt_install: true,
            solc_jobs: None,
        }
    }
}

impl LspConfig {
    pub fn from_settings(settings: Value) -> Self {
        parse_settings(settings).unwrap_or_default()
    }
}

fn parse_settings(settings: Value) -> Option<LspConfig> {
    let has_top_level = settings.get("diagnostics").is_some()
        || settings.get("format").is_some()
        || settings.get("lint").is_some()
        || settings.get("toolchain").is_some();
    if has_top_level && let Ok(config) = serde_json::from_value::<LspConfig>(settings.clone()) {
        return Some(config);
    }

    let nested = settings
        .get("solidityAnalyzer")
        .cloned()
        .or_else(|| settings.get("solidity-analyzer").cloned())?;
    serde_json::from_value::<LspConfig>(nested).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::LspConfig;

    #[test]
    fn default_settings_match_extension_defaults() {
        let config = LspConfig::default();
        assert!(config.diagnostics.enable);
        assert!(config.diagnostics.on_save);
        assert!(config.diagnostics.on_change);
        assert!(!config.format.on_save);
        assert!(config.lint.enable);
        assert!(config.lint.on_save);
        assert!(!config.lint.on_change);
        assert!(config.toolchain.prompt_install);
        assert!(config.toolchain.solc_jobs.is_none());
    }

    #[test]
    fn settings_round_trip() {
        let settings = json!({
            "diagnostics": { "enable": true, "onSave": true, "onChange": false },
            "format": { "onSave": true },
            "lint": { "enable": false, "onSave": true, "onChange": true },
            "toolchain": { "promptInstall": false, "solcJobs": 3 }
        });
        let config = LspConfig::from_settings(settings);
        assert!(config.diagnostics.enable);
        assert!(config.diagnostics.on_save);
        assert!(!config.diagnostics.on_change);
        assert!(config.format.on_save);
        assert!(!config.lint.enable);
        assert!(config.lint.on_save);
        assert!(config.lint.on_change);
        assert!(!config.toolchain.prompt_install);
        assert_eq!(config.toolchain.solc_jobs, Some(3));

        let serialized = serde_json::to_value(&config).expect("serialize config");
        let reparsed = LspConfig::from_settings(serialized);
        assert_eq!(reparsed, config);
    }

    #[test]
    fn parses_nested_diagnostics_settings() {
        let settings = json!({
            "solidityAnalyzer": {
                "diagnostics": {
                    "enable": true,
                    "onSave": false,
                    "onChange": true
                }
            }
        });

        let config = LspConfig::from_settings(settings);
        assert!(config.diagnostics.enable);
        assert!(!config.diagnostics.on_save);
        assert!(config.diagnostics.on_change);
    }

    #[test]
    fn parses_top_level_diagnostics_settings() {
        let settings = json!({
            "diagnostics": {
                "enable": false,
                "onSave": true,
                "onChange": false
            }
        });

        let config = LspConfig::from_settings(settings);
        assert!(!config.diagnostics.enable);
        assert!(config.diagnostics.on_save);
        assert!(!config.diagnostics.on_change);
    }
}
