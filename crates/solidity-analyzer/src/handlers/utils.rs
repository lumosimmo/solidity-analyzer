use sa_vfs::{FileId, VfsSnapshot};
use tower_lsp::lsp_types::Url;
use tracing::debug;

use crate::lsp_utils;
use sa_span::lsp::to_lsp_range;

pub(crate) fn resolve_file_text<'a>(
    vfs: &'a VfsSnapshot,
    uri: &Url,
    context: &str,
) -> Option<(FileId, &'a str)> {
    let path = match lsp_utils::url_to_path(uri) {
        Some(path) => path,
        None => {
            debug!(%uri, %context, "invalid document URI");
            return None;
        }
    };
    let file_id = match vfs.file_id(&path) {
        Some(file_id) => file_id,
        None => {
            debug!(path = %path, %context, "file id not found");
            return None;
        }
    };
    let text = match vfs.file_text(file_id) {
        Some(text) => text,
        None => {
            debug!(path = %path, file_id = ?file_id, %context, "file text not found");
            return None;
        }
    };

    Some((file_id, text))
}

pub(crate) fn text_edit_to_lsp(
    edit: &sa_ide::TextEdit,
    text: &str,
) -> tower_lsp::lsp_types::TextEdit {
    tower_lsp::lsp_types::TextEdit {
        range: to_lsp_range(edit.range, text),
        new_text: edit.new_text.clone(),
    }
}
