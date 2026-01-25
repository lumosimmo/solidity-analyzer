use std::sync::Arc;

use sa_ide::AnalysisChange;
use sa_span::lsp::from_lsp_range;
use sa_vfs::{VfsChange, VfsSnapshot};
use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, TextDocumentContentChangeEvent,
};
use tracing::{debug, warn};

use crate::indexer;
use crate::lsp_utils::url_to_path;
use crate::state::{OpenDocument, ServerState};
use crate::workspace;

pub fn did_open(state: &mut ServerState, params: DidOpenTextDocumentParams) {
    let path = match url_to_path(&params.text_document.uri) {
        Some(path) => path,
        None => return,
    };

    let text = params.text_document.text;
    state.vfs.apply_change(VfsChange::Set {
        path: path.clone(),
        text: Arc::from(text.clone()),
    });
    let snapshot = state.vfs.snapshot();
    if snapshot.file_id(&path).is_some() {
        state.open_documents.insert(
            path.clone(),
            OpenDocument {
                version: params.text_document.version,
            },
        );
    }
    apply_snapshot(state, snapshot);

    if state.config.is_none()
        && let Some(root) = state.discover_foundry_root(&path)
    {
        if state.root_path.as_ref() != Some(&root) {
            state.root_path = Some(root.clone());
        }
        if let Err(error) = workspace::load(state, &root, None) {
            warn!(?error, root = %root, "did_open: failed to load foundry workspace");
        }
    }

    let (workspace, profile) = match state.config.as_ref() {
        Some(config) => (
            config.workspace().clone(),
            config.active_profile().name().to_string(),
        ),
        None => return,
    };
    let index_result =
        match indexer::index_open_file_imports(&workspace, Some(profile.as_str()), &path, &text) {
            Ok(result) => result,
            Err(error) => {
                warn!(?error, path = %path, "did_open: failed to index file imports");
                return;
            }
        };

    let mut changes = Vec::new();
    for indexed_file in index_result.files {
        state.indexed_files.insert(indexed_file.path.clone());
        if state.open_documents.contains_key(&indexed_file.path) {
            continue;
        }
        changes.push(VfsChange::Set {
            path: indexed_file.path,
            text: Arc::from(indexed_file.text),
        });
    }

    if !changes.is_empty() {
        state.vfs.apply_changes(changes);
        let snapshot = state.vfs.snapshot();
        apply_snapshot(state, snapshot);
    }
}

pub fn did_change(state: &mut ServerState, params: DidChangeTextDocumentParams) {
    let path = match url_to_path(&params.text_document.uri) {
        Some(path) => path,
        None => return,
    };

    let snapshot = state.vfs.snapshot();
    let existing_text = snapshot
        .file_id(&path)
        .and_then(|file_id| snapshot.file_text(file_id))
        .unwrap_or("");
    let Some(new_text) = apply_changes(existing_text, &params.content_changes) else {
        return;
    };

    state.vfs.apply_change(VfsChange::Set {
        path: path.clone(),
        text: Arc::from(new_text),
    });
    let snapshot = state.vfs.snapshot();
    if snapshot.file_id(&path).is_some() {
        state.open_documents.insert(
            path.clone(),
            OpenDocument {
                version: params.text_document.version,
            },
        );
    }
    apply_snapshot(state, snapshot);
}

pub fn did_close(state: &mut ServerState, params: DidCloseTextDocumentParams) {
    let path = match url_to_path(&params.text_document.uri) {
        Some(path) => path,
        None => return,
    };
    let _ = state.open_documents.remove(&path);
    if state.indexed_files.contains(&path) {
        match std::fs::read_to_string(path.as_str()) {
            Ok(text) => state.vfs.apply_change(VfsChange::Set {
                path: path.clone(),
                text: Arc::from(text),
            }),
            Err(error) => {
                debug!(?error, path = %path, "did_close: failed to reload indexed file");
            }
        }
    } else {
        state.vfs.apply_change(VfsChange::Remove { path });
    }
    let snapshot = state.vfs.snapshot();
    apply_snapshot(state, snapshot);
}

pub fn did_save(state: &mut ServerState, params: DidSaveTextDocumentParams) {
    let path = match url_to_path(&params.text_document.uri) {
        Some(path) => path,
        None => return,
    };
    if let Some(text) = params.text {
        state.vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from(text),
        });
        let snapshot = state.vfs.snapshot();
        if snapshot.file_id(&path).is_some() {
            // didSave doesn't include a document version; keep the last known value.
            let version = state
                .open_documents
                .get(&path)
                .map(|doc| doc.version)
                .unwrap_or_default();
            state
                .open_documents
                .insert(path.clone(), OpenDocument { version });
        }
        apply_snapshot(state, snapshot);
    }
}

fn apply_snapshot(state: &mut ServerState, snapshot: VfsSnapshot) {
    let mut change = AnalysisChange::new();
    change.set_vfs(snapshot.clone());
    state.analysis_host.apply_change(change);
    state.vfs_snapshot = Some(snapshot);
}

fn apply_changes(text: &str, changes: &[TextDocumentContentChangeEvent]) -> Option<String> {
    let mut current = text.to_string();
    for change in changes {
        if let Some(range) = &change.range {
            let range = from_lsp_range(*range, &current)?;
            let start: usize = range.start().into();
            let end: usize = range.end().into();
            if start > current.len() || end > current.len() || start > end {
                return None;
            }
            let mut next = String::with_capacity(current.len() + change.text.len());
            next.push_str(&current[..start]);
            next.push_str(&change.text);
            next.push_str(&current[end..]);
            current = next;
        } else {
            current = change.text.clone();
        }
    }
    Some(current)
}
