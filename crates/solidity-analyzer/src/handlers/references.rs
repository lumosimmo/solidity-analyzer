use sa_ide::Analysis;
use sa_span::lsp::{from_lsp_position, to_lsp_range};
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{Location, ReferenceParams, Url};
use tracing::debug;

use crate::lsp_utils;

pub fn references(
    analysis: &Analysis,
    vfs: &VfsSnapshot,
    params: ReferenceParams,
) -> Option<Vec<Location>> {
    let uri = &params.text_document_position.text_document.uri;
    let path = match lsp_utils::url_to_path(uri) {
        Some(path) => path,
        None => {
            debug!(%uri, "references: invalid document URI");
            return None;
        }
    };
    let file_id = match vfs.file_id(&path) {
        Some(file_id) => file_id,
        None => {
            debug!(path = %path, "references: file id not found");
            return None;
        }
    };
    let text = match vfs.file_text(file_id) {
        Some(text) => text,
        None => {
            debug!(path = %path, file_id = ?file_id, "references: file text not found");
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
                "references: invalid position"
            );
            return None;
        }
    };

    let references = analysis.find_references(file_id, offset);
    let mut locations = Vec::new();
    for reference in references {
        let target_path = match vfs.path(reference.file_id()) {
            Some(path) => path,
            None => {
                debug!(target_file_id = ?reference.file_id(), "references: missing target path");
                continue;
            }
        };
        let target_uri = match Url::from_file_path(target_path.as_str()) {
            Ok(uri) => uri,
            Err(()) => {
                debug!(
                    target_file_id = ?reference.file_id(),
                    target_path = %target_path,
                    "references: failed to convert target path to URI"
                );
                continue;
            }
        };
        let target_text = match vfs.file_text(reference.file_id()) {
            Some(text) => text,
            None => {
                debug!(
                    target_file_id = ?reference.file_id(),
                    target_path = %target_path,
                    "references: target text not found"
                );
                continue;
            }
        };
        let target_range = to_lsp_range(reference.range(), target_text);
        locations.push(Location::new(target_uri, target_range));
    }

    Some(locations)
}
