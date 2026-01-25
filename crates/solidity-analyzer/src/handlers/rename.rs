use std::collections::HashMap;

use sa_ide::SourceChange;
use sa_span::lsp::from_lsp_position;
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{RenameParams, Url, WorkspaceEdit};
use tracing::debug;

use super::text_edit_to_lsp;
use crate::lsp_utils;

pub fn rename(
    analysis: &sa_ide::Analysis,
    vfs: &VfsSnapshot,
    params: RenameParams,
) -> Option<WorkspaceEdit> {
    let uri = &params.text_document_position.text_document.uri;
    let path = match lsp_utils::url_to_path(uri) {
        Some(path) => path,
        None => {
            debug!(%uri, "rename: invalid document URI");
            return None;
        }
    };
    let file_id = match vfs.file_id(&path) {
        Some(file_id) => file_id,
        None => {
            debug!(path = %path, "rename: file id not found");
            return None;
        }
    };
    let text = match vfs.file_text(file_id) {
        Some(text) => text,
        None => {
            debug!(path = %path, file_id = ?file_id, "rename: file text not found");
            return None;
        }
    };
    let position = params.text_document_position.position;
    let offset = match from_lsp_position(position, text) {
        Some(offset) => offset,
        None => {
            debug!(
                ?position,
                file_id = ?file_id,
                text_len = text.len(),
                "rename: invalid position"
            );
            return None;
        }
    };

    let change = analysis.rename(file_id, offset, &params.new_name)?;
    source_change_to_workspace_edit(change, vfs)
}

fn source_change_to_workspace_edit(
    change: SourceChange,
    vfs: &VfsSnapshot,
) -> Option<WorkspaceEdit> {
    let mut changes = HashMap::new();
    for file_edit in change.edits() {
        let path = match vfs.path(file_edit.file_id) {
            Some(path) => path,
            None => {
                debug!(target_file_id = ?file_edit.file_id, "rename: missing target path");
                continue;
            }
        };
        let uri = match Url::from_file_path(path.as_str()) {
            Ok(uri) => uri,
            Err(()) => {
                debug!(target_file_id = ?file_edit.file_id, path = %path, "rename: invalid URI");
                continue;
            }
        };
        let text = match vfs.file_text(file_edit.file_id) {
            Some(text) => text,
            None => {
                debug!(target_file_id = ?file_edit.file_id, path = %path, "rename: missing text");
                continue;
            }
        };
        let lsp_edits = file_edit
            .edits
            .iter()
            .map(|edit| text_edit_to_lsp(edit, text))
            .collect::<Vec<_>>();
        changes.insert(uri, lsp_edits);
    }

    if changes.is_empty() {
        return None;
    }

    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}
