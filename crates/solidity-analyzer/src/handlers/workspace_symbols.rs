use sa_def::DefKind;
use sa_span::lsp::to_lsp_range;
use sa_vfs::VfsSnapshot;
use tower_lsp::lsp_types::{Location, SymbolInformation, Url, WorkspaceSymbolParams};
use tracing::debug;

pub fn workspace_symbols(
    analysis: &sa_ide::Analysis,
    vfs: &VfsSnapshot,
    params: WorkspaceSymbolParams,
) -> Option<Vec<SymbolInformation>> {
    let symbols = analysis.workspace_symbols(&params.query);
    let mut infos = Vec::new();

    for symbol in symbols {
        let file_id = symbol.file_id();
        let path = match vfs.path(file_id) {
            Some(path) => path,
            None => {
                debug!(symbol_file_id = ?file_id, "workspace_symbols: missing file path");
                continue;
            }
        };
        let uri = match Url::from_file_path(path.as_str()) {
            Ok(uri) => uri,
            Err(()) => {
                debug!(symbol_file_id = ?file_id, path = %path, "workspace_symbols: invalid uri");
                continue;
            }
        };
        let text = match vfs.file_text(file_id) {
            Some(text) => text,
            None => {
                debug!(symbol_file_id = ?file_id, path = %path, "workspace_symbols: missing text");
                continue;
            }
        };
        let range = to_lsp_range(symbol.range(), text);
        #[allow(deprecated)]
        let info = SymbolInformation {
            name: symbol.name().to_string(),
            kind: symbol_kind_to_lsp(symbol.kind()),
            tags: None,
            deprecated: None,
            location: Location::new(uri, range),
            container_name: None,
        };
        infos.push(info);
    }

    Some(infos)
}

fn symbol_kind_to_lsp(kind: DefKind) -> tower_lsp::lsp_types::SymbolKind {
    match kind {
        DefKind::Contract => tower_lsp::lsp_types::SymbolKind::CLASS,
        DefKind::Function => tower_lsp::lsp_types::SymbolKind::FUNCTION,
        DefKind::Struct => tower_lsp::lsp_types::SymbolKind::STRUCT,
        DefKind::Enum => tower_lsp::lsp_types::SymbolKind::ENUM,
        DefKind::Event => tower_lsp::lsp_types::SymbolKind::EVENT,
        DefKind::Error => tower_lsp::lsp_types::SymbolKind::CLASS,
        DefKind::Modifier => tower_lsp::lsp_types::SymbolKind::METHOD,
        DefKind::Variable => tower_lsp::lsp_types::SymbolKind::VARIABLE,
        DefKind::Udvt => tower_lsp::lsp_types::SymbolKind::TYPE_PARAMETER,
    }
}
