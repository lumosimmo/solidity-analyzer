use sa_config::{ResolvedFoundryConfig, formatter_config};
use sa_span::lsp::to_lsp_range;
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{
    DocumentChanges, OneOf, OptionalVersionedTextDocumentIdentifier, TextDocumentEdit, TextEdit,
    Url, WorkspaceEdit,
};

use super::resolve_file_text;

pub fn format_on_save(
    analysis: &sa_ide::Analysis,
    vfs: &VfsSnapshot,
    uri: &Url,
    config: &ResolvedFoundryConfig,
) -> Option<WorkspaceEdit> {
    let (file_id, text) = resolve_file_text(vfs, uri, "did_save")?;

    let formatter = formatter_config(config);
    let edit = analysis.format_document(file_id, &formatter)?;
    let lsp_edit = TextEdit {
        range: to_lsp_range(edit.range, text),
        new_text: edit.new_text,
    };

    let document_edit = TextDocumentEdit {
        text_document: OptionalVersionedTextDocumentIdentifier {
            uri: uri.clone(),
            version: None,
        },
        edits: vec![OneOf::Left(lsp_edit)],
    };

    Some(WorkspaceEdit {
        changes: None,
        document_changes: Some(DocumentChanges::Edits(vec![document_edit])),
        change_annotations: None,
    })
}
