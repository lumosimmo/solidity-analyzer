use sa_base_db::FileId;
use sa_span::TextRange;
use solar::ast::FunctionKind;
use solar::sema::hir;

use crate::{ResolvedSymbolKind, SemaSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemaSymbol {
    pub kind: ResolvedSymbolKind,
    pub name: String,
    pub file_id: FileId,
    pub range: TextRange,
    pub selection_range: TextRange,
    pub children: Vec<SemaSymbol>,
}

impl SemaSnapshot {
    pub fn workspace_symbols(&self, query: &str) -> Vec<SemaSymbol> {
        self.with_gcx(|gcx| {
            let mut symbols = Vec::new();
            for item_id in gcx.hir.item_ids() {
                let Some(symbol) = symbol_from_item(self, gcx, item_id) else {
                    continue;
                };
                if symbol.name.contains(query) {
                    symbols.push(symbol);
                }
            }
            symbols
        })
    }

    pub fn document_symbols(&self, file_id: FileId) -> Option<Vec<SemaSymbol>> {
        let source_id = self.source_id_for_file(file_id)?;
        let symbols = self.with_gcx(|gcx| {
            let source = gcx.hir.source(source_id);
            let mut symbols = Vec::new();
            for &item_id in source.items {
                if let Some(symbol) = document_symbol_for_item(self, gcx, item_id, file_id) {
                    symbols.push(symbol);
                }
            }
            symbols
        });
        Some(symbols)
    }
}

fn document_symbol_for_item(
    snapshot: &SemaSnapshot,
    gcx: solar::sema::Gcx<'_>,
    item_id: hir::ItemId,
    file_id: FileId,
) -> Option<SemaSymbol> {
    let mut symbol = symbol_from_item(snapshot, gcx, item_id)?;
    if symbol.file_id != file_id {
        return None;
    }

    if let hir::ItemId::Contract(contract_id) = item_id {
        let contract = gcx.hir.contract(contract_id);
        let mut children = Vec::new();
        for &child_id in contract.items {
            if let Some(child) = symbol_from_item(snapshot, gcx, child_id)
                && child.file_id == file_id
            {
                children.push(child);
            }
        }
        symbol.children = children;
    }

    Some(symbol)
}

fn symbol_from_item(
    snapshot: &SemaSnapshot,
    gcx: solar::sema::Gcx<'_>,
    item_id: hir::ItemId,
) -> Option<SemaSymbol> {
    let item = gcx.hir.item(item_id);
    let name = item.name()?;
    let selection_range = snapshot.span_to_text_range(name.span)?;
    let range = snapshot.span_to_text_range(item.span())?;
    let file_id = *snapshot.file_id_by_source.get(&item.source())?;

    let kind = match item_id {
        hir::ItemId::Contract(_) => ResolvedSymbolKind::Contract,
        hir::ItemId::Function(id) => {
            let func = gcx.hir.function(id);
            if func.kind == FunctionKind::Modifier {
                ResolvedSymbolKind::Modifier
            } else {
                ResolvedSymbolKind::Function
            }
        }
        hir::ItemId::Struct(_) => ResolvedSymbolKind::Struct,
        hir::ItemId::Enum(_) => ResolvedSymbolKind::Enum,
        hir::ItemId::Event(_) => ResolvedSymbolKind::Event,
        hir::ItemId::Error(_) => ResolvedSymbolKind::Error,
        hir::ItemId::Udvt(_) => ResolvedSymbolKind::Udvt,
        hir::ItemId::Variable(id) => {
            let var = gcx.hir.variable(id);
            if !matches!(var.kind, hir::VarKind::Global | hir::VarKind::State) {
                return None;
            }
            ResolvedSymbolKind::Variable
        }
    };

    Some(SemaSymbol {
        kind,
        name: name.as_str().to_string(),
        file_id,
        range,
        selection_range,
        children: Vec::new(),
    })
}
