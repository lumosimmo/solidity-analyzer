use std::path::PathBuf;
use std::sync::Arc;

use crate::indexer;
use crate::state::ServerState;
use sa_config::ResolvedFoundryConfig;
use sa_ide::AnalysisChange;
use sa_paths::NormalizedPath;
use sa_vfs::VfsChange;
use tracing::{debug, info};

pub fn load(
    state: &mut ServerState,
    root: &NormalizedPath,
    profile: Option<&str>,
) -> anyhow::Result<()> {
    let root_path = PathBuf::from(root.as_str());
    info!(root = %root, profile = ?profile, "loading foundry workspace");
    let resolved = sa_load_foundry::load_foundry(&root_path, profile)?;
    log_resolved_config(&resolved);
    apply_config(state, resolved)?;
    Ok(())
}

pub fn reload(state: &mut ServerState) -> anyhow::Result<()> {
    let Some(root) = state.root_path.clone() else {
        debug!("reload requested without a workspace root");
        return Ok(());
    };
    let profile = state
        .config
        .as_ref()
        .map(|config| config.active_profile().name().to_string());
    load(state, &root, profile.as_deref())
}

fn apply_config(state: &mut ServerState, resolved: ResolvedFoundryConfig) -> anyhow::Result<()> {
    let workspace = resolved.workspace().clone();
    let remappings = resolved.active_profile().remappings();
    let index_result = indexer::index_workspace(&workspace, remappings)?;

    let mut changes = Vec::new();
    let mut new_indexed_paths = std::collections::HashSet::new();

    for indexed_file in index_result.files {
        new_indexed_paths.insert(indexed_file.path.clone());
        if state.open_documents.contains_key(&indexed_file.path) {
            continue;
        }
        changes.push(VfsChange::Set {
            path: indexed_file.path,
            text: Arc::from(indexed_file.text),
        });
    }

    // Remove stale files that were previously indexed but are no longer present.
    for old_path in &state.indexed_files {
        if !new_indexed_paths.contains(old_path) && !state.open_documents.contains_key(old_path) {
            changes.push(VfsChange::Remove {
                path: old_path.clone(),
            });
        }
    }

    state.vfs.apply_changes(changes);
    let snapshot = state.vfs.snapshot();
    let mut change = AnalysisChange::new();
    change.set_vfs(snapshot.clone());
    change.set_config(resolved.clone());
    state.analysis_host.apply_change(change);
    state.vfs_snapshot = Some(snapshot);
    state.indexed_files = new_indexed_paths;
    state.config = Some(resolved);
    Ok(())
}

fn log_resolved_config(resolved: &ResolvedFoundryConfig) {
    let workspace = resolved.workspace();
    let profile = resolved.active_profile();
    info!(
        root = %workspace.root(),
        src = %workspace.src(),
        lib = %workspace.lib(),
        test = %workspace.test(),
        script = %workspace.script(),
        profile = %profile.name(),
        solc_version = ?profile.solc_version(),
        remappings = profile.remappings().len(),
        "foundry workspace loaded"
    );
    debug!(remappings = ?profile.remappings(), "foundry remappings");
}
