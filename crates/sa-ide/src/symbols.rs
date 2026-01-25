use sa_base_db::{FileId, ProjectId};
use sa_hir::HirDatabase;
use sa_sema::{ResolvedSymbolKind, SemaSymbol};

pub type WorkspaceSymbol = sa_ide_db::Symbol;

pub fn workspace_symbols(
    db: &dyn sa_ide_db::IdeDatabase,
    project_id: ProjectId,
    query: &str,
) -> Vec<WorkspaceSymbol> {
    sa_ide_db::symbol_search(db, project_id, query)
}

pub fn document_symbols(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
) -> Option<Vec<crate::SymbolInfo>> {
    let project = db.project_input(project_id);
    let snapshot = sa_sema::sema_snapshot_for_project(db, project);
    let snapshot = snapshot.for_file(file_id)?;
    let symbols = snapshot.document_symbols(file_id)?;
    Some(symbols.into_iter().map(symbol_info_from_sema).collect())
}

fn symbol_info_from_sema(symbol: SemaSymbol) -> crate::SymbolInfo {
    let children = symbol
        .children
        .into_iter()
        .map(symbol_info_from_sema)
        .collect();
    crate::SymbolInfo {
        kind: symbol_kind_from_sema(symbol.kind),
        name: symbol.name,
        range: symbol.range,
        selection_range: symbol.selection_range,
        children,
    }
}

fn symbol_kind_from_sema(kind: ResolvedSymbolKind) -> crate::SymbolKind {
    match kind {
        ResolvedSymbolKind::Contract => crate::SymbolKind::Contract,
        ResolvedSymbolKind::Function => crate::SymbolKind::Function,
        ResolvedSymbolKind::Modifier => crate::SymbolKind::Modifier,
        ResolvedSymbolKind::Struct => crate::SymbolKind::Struct,
        ResolvedSymbolKind::Enum => crate::SymbolKind::Enum,
        ResolvedSymbolKind::Event => crate::SymbolKind::Event,
        ResolvedSymbolKind::Error => crate::SymbolKind::Error,
        ResolvedSymbolKind::Variable => crate::SymbolKind::Variable,
        ResolvedSymbolKind::Udvt => crate::SymbolKind::Udvt,
    }
}
