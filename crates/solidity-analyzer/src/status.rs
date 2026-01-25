use sa_config::ResolvedFoundryConfig;
use sa_project_model::Remapping;
use sa_toolchain::{Toolchain, is_svm_installed};
use tracing::error;

pub fn startup_status(config: &ResolvedFoundryConfig) -> String {
    let workspace = config.workspace();
    let profile = config.active_profile();
    let mut lines = Vec::new();

    lines.push("solidity-analyzer status:".to_string());
    lines.push(format!("profile: {}", profile.name()));
    lines.push(format!("root: {}", workspace.root()));
    lines.push(format!("src: {}", workspace.src()));
    lines.push(format!("lib: {}", workspace.lib()));
    lines.push(format!("test: {}", workspace.test()));
    lines.push(format!("script: {}", workspace.script()));
    lines.push(format!(
        "remappings: {}",
        format_remappings(profile.remappings())
    ));

    let toolchain = Toolchain::new(config.clone());
    lines.push(format!("solc: {}", format_solc_status(&toolchain)));

    lines.join("\n")
}

fn format_remappings(remappings: &[Remapping]) -> String {
    if remappings.is_empty() {
        return "0".to_string();
    }

    let list = remappings
        .iter()
        .take(5)
        .map(format_remapping)
        .collect::<Vec<_>>()
        .join(", ");
    if remappings.len() > 5 {
        format!("{} ({list}, ...)", remappings.len())
    } else {
        format!("{} ({list})", remappings.len())
    }
}

fn format_remapping(remapping: &Remapping) -> String {
    match remapping.context() {
        Some(context) => format!("{context}:{}={}", remapping.from(), remapping.to()),
        None => format!("{}={}", remapping.from(), remapping.to()),
    }
}

fn format_solc_status(toolchain: &Toolchain) -> String {
    if let Some(spec) = toolchain.solc_spec() {
        return match toolchain.resolve() {
            Ok(resolved) => format!(
                "{version} (spec {spec}, path {path})",
                version = resolved.version(),
                path = resolved.path().display()
            ),
            Err(error) => format!("missing (spec {spec}, {error})"),
        };
    }

    match toolchain.auto_detect_versions() {
        Ok(versions) => {
            if versions.is_empty() {
                return "auto-detect: no Solidity sources".to_string();
            }

            let mut items = Vec::with_capacity(versions.len());
            for version in versions {
                let status = match is_svm_installed(&version) {
                    Ok(true) => "installed",
                    Ok(false) => "missing",
                    Err(error) => {
                        error!(?error, %version, "failed to check svm install status");
                        "unknown"
                    }
                };
                items.push(format!("{version} ({status})"));
            }
            format!("auto-detect: {}", items.join(", "))
        }
        Err(error) => format!("auto-detect failed ({error})"),
    }
}
