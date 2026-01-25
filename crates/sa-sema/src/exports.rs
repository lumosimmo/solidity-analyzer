use std::collections::HashSet;

use solar::ast::{ImportItems, ItemKind};
use solar::interface::Symbol;
use solar::sema::{Gcx, hir};

pub(crate) fn exported_item_names(gcx: Gcx<'_>, source_id: hir::SourceId) -> Vec<Symbol> {
    let mut names = Vec::new();
    let mut visited = HashSet::new();
    collect_exported_names(gcx, source_id, &mut visited, &mut names);
    names
}

pub(crate) fn exported_item_ids(gcx: Gcx<'_>, source_id: hir::SourceId) -> Vec<hir::ItemId> {
    let names = exported_item_names(gcx, source_id);
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for name in names {
        if let Some(item_id) = find_exported_item(gcx, source_id, name)
            && seen.insert(item_id)
        {
            items.push(item_id);
        }
    }
    items
}

pub(crate) fn find_exported_item(
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    name: Symbol,
) -> Option<hir::ItemId> {
    let mut visited = HashSet::new();
    find_exported_item_inner(gcx, source_id, name, &mut visited)
}

fn collect_exported_names(
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    visited: &mut HashSet<hir::SourceId>,
    names: &mut Vec<Symbol>,
) {
    if !visited.insert(source_id) {
        return;
    }

    let source = gcx.hir.source(source_id);
    for &item_id in source.items {
        let Some(ident) = gcx.hir.item(item_id).name() else {
            continue;
        };
        names.push(ident.name);
    }

    let Some(ast) = gcx
        .sources
        .get(source_id)
        .and_then(|source| source.ast.as_ref())
    else {
        return;
    };
    for (item_id, item) in ast.items.iter_enumerated() {
        let ItemKind::Import(import) = &item.kind else {
            continue;
        };
        let Some(import_source_id) = source
            .imports
            .iter()
            .find_map(|(import_id, source_id)| (*import_id == item_id).then_some(*source_id))
        else {
            continue;
        };

        match &import.items {
            ImportItems::Plain(None) => {
                collect_exported_names(gcx, import_source_id, visited, names);
            }
            ImportItems::Aliases(aliases) => {
                for (original, alias) in aliases.iter() {
                    let alias = alias.as_ref().unwrap_or(original);
                    names.push(alias.name);
                }
            }
            ImportItems::Plain(Some(alias)) | ImportItems::Glob(alias) => {
                names.push(alias.name);
            }
        }
    }
}

fn find_exported_item_inner(
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    name: Symbol,
    visited: &mut HashSet<hir::SourceId>,
) -> Option<hir::ItemId> {
    if !visited.insert(source_id) {
        return None;
    }

    let source = gcx.hir.source(source_id);
    for &item_id in source.items {
        if gcx
            .hir
            .item(item_id)
            .name()
            .is_some_and(|ident| ident.name == name)
        {
            return Some(item_id);
        }
    }

    let ast = gcx.sources.get(source_id)?.ast.as_ref()?;
    for (item_id, item) in ast.items.iter_enumerated() {
        let ItemKind::Import(import) = &item.kind else {
            continue;
        };
        let Some(import_source_id) = source
            .imports
            .iter()
            .find_map(|(import_id, source_id)| (*import_id == item_id).then_some(*source_id))
        else {
            continue;
        };
        match &import.items {
            ImportItems::Plain(None) => {
                if let Some(found) = find_exported_item_inner(gcx, import_source_id, name, visited)
                {
                    return Some(found);
                }
            }
            ImportItems::Aliases(aliases) => {
                for (original, alias) in aliases.iter() {
                    let alias = alias.as_ref().unwrap_or(original);
                    if alias.name != name {
                        continue;
                    }
                    if let Some(found) =
                        find_exported_item_inner(gcx, import_source_id, original.name, visited)
                    {
                        return Some(found);
                    }
                }
            }
            ImportItems::Plain(Some(_)) | ImportItems::Glob(_) => {}
        }
    }

    None
}
