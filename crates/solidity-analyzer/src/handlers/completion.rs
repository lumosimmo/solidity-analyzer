use sa_ide::{CompletionInsertTextFormat, CompletionItem, CompletionItemKind};
use sa_span::lsp::{from_lsp_position, to_lsp_range};
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{
    CompletionItem as LspCompletionItem, CompletionItemKind as LspCompletionItemKind,
    CompletionItemLabelDetails, CompletionParams, CompletionResponse, CompletionTextEdit,
    InsertTextFormat, TextEdit,
};
use tracing::debug;

use crate::lsp_utils;

pub fn completion(
    analysis: &sa_ide::Analysis,
    vfs: &VfsSnapshot,
    params: CompletionParams,
) -> Option<CompletionResponse> {
    let uri = &params.text_document_position.text_document.uri;
    let path = match lsp_utils::url_to_path(uri) {
        Some(path) => path,
        None => {
            debug!(%uri, "completion: invalid document URI");
            return None;
        }
    };
    let file_id = match vfs.file_id(&path) {
        Some(file_id) => file_id,
        None => {
            debug!(path = %path, "completion: file id not found");
            return None;
        }
    };
    let text = match vfs.file_text(file_id) {
        Some(text) => text,
        None => {
            debug!(path = %path, file_id = ?file_id, "completion: file text not found");
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
                "completion: invalid position"
            );
            return None;
        }
    };
    let completions = analysis.completions(file_id, offset);
    let items = completions
        .into_iter()
        .map(|item| completion_item_to_lsp(item, text))
        .collect::<Vec<_>>();

    Some(CompletionResponse::Array(items))
}

fn completion_item_to_lsp(item: CompletionItem, text: &str) -> LspCompletionItem {
    let range = to_lsp_range(item.replacement_range, text);
    let label = item.label;
    let insert_text = item.insert_text.clone().unwrap_or_else(|| label.clone());
    let insert_text_format = match item.insert_text_format {
        CompletionInsertTextFormat::Plain => InsertTextFormat::PLAIN_TEXT,
        CompletionInsertTextFormat::Snippet => InsertTextFormat::SNIPPET,
    };
    let label_details = item.origin.as_deref().map(|origin| {
        let description = if origin == "builtin" {
            "(builtin)".to_string()
        } else {
            format!("(from {origin})")
        };
        CompletionItemLabelDetails {
            detail: None,
            description: Some(description),
        }
    });
    LspCompletionItem {
        kind: Some(completion_kind_to_lsp(item.kind)),
        detail: item.detail,
        label_details,
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: insert_text,
        })),
        insert_text_format: Some(insert_text_format),
        label,
        ..LspCompletionItem::default()
    }
}

fn completion_kind_to_lsp(kind: CompletionItemKind) -> LspCompletionItemKind {
    match kind {
        CompletionItemKind::Contract => LspCompletionItemKind::CLASS,
        CompletionItemKind::Function => LspCompletionItemKind::FUNCTION,
        CompletionItemKind::Struct => LspCompletionItemKind::STRUCT,
        CompletionItemKind::Enum => LspCompletionItemKind::ENUM,
        CompletionItemKind::Event => LspCompletionItemKind::EVENT,
        // Solidity `error` definitions are custom error types; EVENT provides
        // a visually distinctive icon (LSP lacks a dedicated ERROR kind).
        CompletionItemKind::Error => LspCompletionItemKind::EVENT,
        CompletionItemKind::Modifier => LspCompletionItemKind::KEYWORD,
        CompletionItemKind::Variable => LspCompletionItemKind::VARIABLE,
        CompletionItemKind::Type => LspCompletionItemKind::CLASS,
        CompletionItemKind::File => LspCompletionItemKind::FILE,
    }
}
