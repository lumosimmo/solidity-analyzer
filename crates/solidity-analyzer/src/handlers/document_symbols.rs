use sa_ide::{SymbolInfo, SymbolKind};
use sa_span::lsp::to_lsp_range;
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse};
use tracing::debug;

use crate::lsp_utils;

pub fn document_symbols(
    analysis: &sa_ide::Analysis,
    vfs: &VfsSnapshot,
    params: DocumentSymbolParams,
) -> Option<DocumentSymbolResponse> {
    let uri = &params.text_document.uri;
    let path = match lsp_utils::url_to_path(uri) {
        Some(path) => path,
        None => {
            debug!(%uri, "document_symbols: invalid document URI");
            return None;
        }
    };
    let file_id = match vfs.file_id(&path) {
        Some(file_id) => file_id,
        None => {
            debug!(path = %path, "document_symbols: file id not found");
            return None;
        }
    };
    let text = match vfs.file_text(file_id) {
        Some(text) => text,
        None => {
            debug!(path = %path, file_id = ?file_id, "document_symbols: file text not found");
            return None;
        }
    };

    let symbols = analysis.document_symbols(file_id);
    let lsp_symbols = symbols
        .into_iter()
        .map(|symbol| symbol_to_lsp(symbol, text))
        .collect::<Vec<_>>();

    Some(DocumentSymbolResponse::Nested(lsp_symbols))
}

fn symbol_to_lsp(symbol: SymbolInfo, text: &str) -> DocumentSymbol {
    let children = if symbol.children.is_empty() {
        None
    } else {
        Some(
            symbol
                .children
                .into_iter()
                .map(|child| symbol_to_lsp(child, text))
                .collect::<Vec<_>>(),
        )
    };

    #[allow(deprecated)]
    let symbol = DocumentSymbol {
        name: symbol.name,
        detail: None,
        kind: symbol_kind_to_lsp(symbol.kind),
        tags: None,
        deprecated: None,
        range: to_lsp_range(symbol.range, text),
        selection_range: to_lsp_range(symbol.selection_range, text),
        children,
    };
    symbol
}

fn symbol_kind_to_lsp(kind: SymbolKind) -> tower_lsp::lsp_types::SymbolKind {
    match kind {
        SymbolKind::Contract => tower_lsp::lsp_types::SymbolKind::CLASS,
        SymbolKind::Function => tower_lsp::lsp_types::SymbolKind::FUNCTION,
        SymbolKind::Struct => tower_lsp::lsp_types::SymbolKind::STRUCT,
        SymbolKind::Enum => tower_lsp::lsp_types::SymbolKind::ENUM,
        SymbolKind::Event => tower_lsp::lsp_types::SymbolKind::EVENT,
        SymbolKind::Error => tower_lsp::lsp_types::SymbolKind::CLASS,
        SymbolKind::Modifier => tower_lsp::lsp_types::SymbolKind::METHOD,
        SymbolKind::Variable => tower_lsp::lsp_types::SymbolKind::VARIABLE,
        SymbolKind::Udvt => tower_lsp::lsp_types::SymbolKind::TYPE_PARAMETER,
    }
}
