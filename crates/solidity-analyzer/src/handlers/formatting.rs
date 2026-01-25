use sa_config::{ResolvedFoundryConfig, formatter_config};
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{DocumentFormattingParams, TextEdit};
use tracing::debug;

use super::{resolve_file_text, text_edit_to_lsp};

pub fn formatting(
    analysis: &sa_ide::Analysis,
    vfs: &VfsSnapshot,
    params: DocumentFormattingParams,
    config: Option<ResolvedFoundryConfig>,
) -> Option<Vec<TextEdit>> {
    let uri = &params.text_document.uri;
    let config = match config {
        Some(config) => config,
        None => {
            debug!(%uri, "formatting: missing ResolvedFoundryConfig");
            return None;
        }
    };
    let (file_id, text) = resolve_file_text(vfs, uri, "formatting")?;

    let formatter = formatter_config(&config);
    let edit = analysis.format_document(file_id, &formatter);
    match edit {
        Some(edit) => Some(vec![text_edit_to_lsp(&edit, text)]),
        None => Some(Vec::new()),
    }
}
