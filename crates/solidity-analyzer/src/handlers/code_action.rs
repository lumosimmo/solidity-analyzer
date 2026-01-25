use std::collections::HashMap;

use sa_ide::{CodeActionDiagnostic, SourceChange};
use sa_span::lsp::from_lsp_range;
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{
    CodeAction as LspCodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    NumberOrString, Url, WorkspaceEdit,
};
use tracing::debug;

use super::text_edit_to_lsp;
use crate::lsp_utils;

pub fn code_action(
    analysis: &sa_ide::Analysis,
    vfs: &VfsSnapshot,
    params: CodeActionParams,
) -> Option<Vec<CodeActionOrCommand>> {
    let uri = &params.text_document.uri;
    let path = match lsp_utils::url_to_path(uri) {
        Some(path) => path,
        None => {
            debug!(%uri, "code_action: invalid document URI");
            return None;
        }
    };
    let file_id = match vfs.file_id(&path) {
        Some(file_id) => file_id,
        None => {
            debug!(path = %path, "code_action: file id not found");
            return None;
        }
    };
    let text = match vfs.file_text(file_id) {
        Some(text) => text,
        None => {
            debug!(path = %path, file_id = ?file_id, "code_action: file text not found");
            return None;
        }
    };

    let diagnostics = params
        .context
        .diagnostics
        .into_iter()
        .filter_map(|diag| {
            let code = match diag.code {
                Some(NumberOrString::String(code)) => Some(code),
                Some(NumberOrString::Number(code)) => Some(code.to_string()),
                None => None,
            }?;
            let range = from_lsp_range(diag.range, text)?;
            Some(CodeActionDiagnostic { range, code })
        })
        .collect::<Vec<_>>();

    let actions = analysis.code_actions(file_id, &diagnostics);
    let mut results = Vec::new();
    for action in actions {
        let edit = match source_change_to_workspace_edit(&action.edit, vfs) {
            Some(edit) => edit,
            None => continue,
        };
        results.push(CodeActionOrCommand::CodeAction(LspCodeAction {
            title: action.title,
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: None,
            edit: Some(edit),
            command: None,
            is_preferred: None,
            disabled: None,
            data: None,
        }));
    }

    Some(results)
}

fn source_change_to_workspace_edit(
    change: &SourceChange,
    vfs: &VfsSnapshot,
) -> Option<WorkspaceEdit> {
    let mut changes = HashMap::new();
    for file_edit in change.edits() {
        let path = match vfs.path(file_edit.file_id) {
            Some(path) => path,
            None => {
                debug!(target_file_id = ?file_edit.file_id, "code_action: missing target path");
                continue;
            }
        };
        let uri = match Url::from_file_path(path.as_str()) {
            Ok(uri) => uri,
            Err(()) => {
                debug!(target_file_id = ?file_edit.file_id, path = %path, "code_action: invalid URI");
                continue;
            }
        };
        let text = match vfs.file_text(file_edit.file_id) {
            Some(text) => text,
            None => {
                debug!(target_file_id = ?file_edit.file_id, path = %path, "code_action: missing text");
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
