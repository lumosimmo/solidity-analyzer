use std::collections::HashSet;

use sa_base_db::FileId;
use sa_span::{TextRange, TextSize, range_contains};
use solar::ast::{FunctionKind, ImportItems, ItemKind};
use solar::interface::Ident;
use solar::sema::builtins::Member;
use solar::sema::hir;
use solar::sema::ty::TyKind;
use solar::sema::{Gcx, Ty};

use crate::contract_members::{ContractMemberAccess, contract_id_from_type, contract_type_members};
use crate::exports;
use crate::ty_utils::default_memory_if_ref;
use crate::{ResolveOutcome, ResolvedSymbol, ResolvedSymbolKind, SemaSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemaCompletionItem {
    pub label: String,
    pub kind: SemaCompletionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemaCompletionKind {
    Contract,
    Function,
    Struct,
    Enum,
    Event,
    Error,
    Modifier,
    Variable,
    Type,
}

impl SemaSnapshot {
    pub fn identifier_completions(
        &self,
        file_id: FileId,
        offset: TextSize,
    ) -> Option<Vec<SemaCompletionItem>> {
        let source_id = self.source_id_for_file(file_id)?;
        let items = self.with_gcx(|gcx| {
            let mut items = Vec::new();
            let mut seen = HashSet::new();

            collect_source_items(gcx, source_id, &mut items, &mut seen);
            collect_imported_items(gcx, source_id, &mut items, &mut seen);

            if let Some(contract_id) = contract_at_offset(self, gcx, source_id, offset) {
                for item in contract_scope_items(gcx, contract_id) {
                    push_completion_item(&mut items, &mut seen, item);
                }
            }

            if let Some(function_id) = function_at_offset(self, gcx, source_id, offset) {
                for item in local_items_for_function(self, gcx, function_id, offset) {
                    push_completion_item(&mut items, &mut seen, item);
                }
            }

            items
        });
        Some(items)
    }

    pub fn member_completions(
        &self,
        file_id: FileId,
        offset: TextSize,
        receiver_range: TextRange,
        receiver: &str,
    ) -> Option<Vec<SemaCompletionItem>> {
        let source_id = self.source_id_for_file(file_id)?;
        let receiver_offset = receiver_range.start();
        let resolved = match receiver {
            "super" | "this" => None,
            _ => match self.resolve_definition(file_id, receiver_offset) {
                ResolveOutcome::Resolved(symbol) => Some(symbol),
                ResolveOutcome::Unresolved { .. } | ResolveOutcome::Unavailable => None,
            },
        };

        let items = self.with_gcx(|gcx| {
            let current_contract = contract_at_offset(self, gcx, source_id, offset);
            let current_function = function_at_offset(self, gcx, source_id, offset);
            if receiver == "super" {
                return current_contract
                    .map(|contract_id| super_member_items(gcx, contract_id))
                    .unwrap_or_default();
            }
            if receiver == "this" {
                return current_contract
                    .map(|contract_id| {
                        let ty = gcx.type_of_item(contract_id.into());
                        member_items_for_type(gcx, ty, source_id, current_contract)
                    })
                    .unwrap_or_default();
            }

            if let Some(resolved) = resolved.as_ref() {
                return match resolved.kind {
                    ResolvedSymbolKind::Contract => {
                        let Some(contract_id) = contract_id_for_symbol(self, gcx, resolved) else {
                            return Vec::new();
                        };
                        let ty = gcx.type_of_item(contract_id.into()).make_type_type(gcx);
                        member_items_for_type(gcx, ty, source_id, current_contract)
                    }
                    ResolvedSymbolKind::Variable => {
                        let Some(var_id) = variable_id_for_symbol(self, gcx, resolved) else {
                            return Vec::new();
                        };
                        let ty = gcx.type_of_item(var_id.into());
                        member_items_for_type(gcx, ty, source_id, current_contract)
                    }
                    ResolvedSymbolKind::Struct
                    | ResolvedSymbolKind::Enum
                    | ResolvedSymbolKind::Event
                    | ResolvedSymbolKind::Error
                    | ResolvedSymbolKind::Modifier
                    | ResolvedSymbolKind::Function
                    | ResolvedSymbolKind::Udvt => Vec::new(),
                };
            }

            match resolve_receiver_by_name(
                gcx,
                source_id,
                current_contract,
                current_function,
                receiver,
            ) {
                Some(ReceiverResolution::Variable(var_id)) => {
                    let ty = gcx.type_of_item(var_id.into());
                    member_items_for_type(gcx, ty, source_id, current_contract)
                }
                Some(ReceiverResolution::Contract(contract_id)) => {
                    let ty = gcx.type_of_item(contract_id.into()).make_type_type(gcx);
                    member_items_for_type(gcx, ty, source_id, current_contract)
                }
                None => Vec::new(),
            }
        });

        Some(items)
    }
}

fn completion_item_for_item(gcx: Gcx<'_>, item_id: hir::ItemId) -> Option<SemaCompletionItem> {
    let name = gcx.item_name_opt(item_id)?;
    let kind = match item_id {
        hir::ItemId::Contract(_) => SemaCompletionKind::Contract,
        hir::ItemId::Function(id) => {
            let func = gcx.hir.function(id);
            match func.kind {
                FunctionKind::Modifier => SemaCompletionKind::Modifier,
                _ => SemaCompletionKind::Function,
            }
        }
        hir::ItemId::Struct(_) => SemaCompletionKind::Struct,
        hir::ItemId::Enum(_) => SemaCompletionKind::Enum,
        hir::ItemId::Event(_) => SemaCompletionKind::Event,
        hir::ItemId::Error(_) => SemaCompletionKind::Error,
        hir::ItemId::Udvt(_) => SemaCompletionKind::Type,
        hir::ItemId::Variable(id) => {
            let var = gcx.hir.variable(id);
            if var.function.is_some() {
                return None;
            }
            let _ = var.name?;
            SemaCompletionKind::Variable
        }
    };

    Some(SemaCompletionItem {
        label: name.to_string(),
        kind,
    })
}

fn push_completion_item(
    items: &mut Vec<SemaCompletionItem>,
    seen: &mut HashSet<(String, SemaCompletionKind)>,
    item: SemaCompletionItem,
) {
    if seen.insert((item.label.clone(), item.kind)) {
        items.push(item);
    }
}

fn collect_source_items(
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    items: &mut Vec<SemaCompletionItem>,
    seen: &mut HashSet<(String, SemaCompletionKind)>,
) {
    let source = gcx.hir.source(source_id);
    for &item_id in source.items {
        if let Some(item) = completion_item_for_item(gcx, item_id) {
            push_completion_item(items, seen, item);
        }
    }
}

fn collect_imported_items(
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    items: &mut Vec<SemaCompletionItem>,
    seen: &mut HashSet<(String, SemaCompletionKind)>,
) {
    let Some(source) = gcx.sources.get(source_id) else {
        return;
    };
    let hir_source = gcx.hir.source(source_id);
    let Some(ast) = source.ast.as_ref() else {
        for &(_, import_source_id) in hir_source.imports {
            let imported = gcx.hir.source(import_source_id);
            for &imported_item_id in imported.items {
                if let Some(item) = completion_item_for_item(gcx, imported_item_id) {
                    push_completion_item(items, seen, item);
                }
            }
        }
        return;
    };

    for (item_id, item) in ast.items.iter_enumerated() {
        let ItemKind::Import(import) = &item.kind else {
            continue;
        };
        let Some(import_source_id) = hir_source
            .imports
            .iter()
            .find_map(|(import_id, source_id)| (*import_id == item_id).then_some(*source_id))
        else {
            continue;
        };

        match &import.items {
            ImportItems::Plain(None) => {
                for imported_item_id in exports::exported_item_ids(gcx, import_source_id) {
                    if let Some(item) = completion_item_for_item(gcx, imported_item_id) {
                        push_completion_item(items, seen, item);
                    }
                }
            }
            ImportItems::Plain(Some(alias)) | ImportItems::Glob(alias) => {
                let item = SemaCompletionItem {
                    label: alias.as_str().to_string(),
                    kind: SemaCompletionKind::Type,
                };
                push_completion_item(items, seen, item);
            }
            ImportItems::Aliases(aliases) => {
                for (original, alias) in aliases.iter() {
                    let alias_ident = alias.as_ref().unwrap_or(original);
                    let Some(item_id) =
                        exports::find_exported_item(gcx, import_source_id, original.name)
                    else {
                        continue;
                    };
                    let Some(mut item) = completion_item_for_item(gcx, item_id) else {
                        continue;
                    };
                    item.label = alias_ident.as_str().to_string();
                    push_completion_item(items, seen, item);
                }
            }
        }
    }
}

fn function_at_offset(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    offset: TextSize,
) -> Option<hir::FunctionId> {
    let mut best: Option<(TextRange, hir::FunctionId)> = None;
    for function_id in gcx.hir.function_ids() {
        let func = gcx.hir.function(function_id);
        if func.source != source_id {
            continue;
        }
        let spans = [func.body_span, func.span];
        for span in spans {
            let Some(range) = snapshot.span_to_text_range(span) else {
                continue;
            };
            if !range_contains(range, offset) {
                continue;
            }
            let replace = best
                .as_ref()
                .map(|(best_range, _)| range_len(range) < range_len(*best_range))
                .unwrap_or(true);
            if replace {
                best = Some((range, function_id));
            }
        }
    }
    best.map(|(_, id)| id)
}

enum ReceiverResolution {
    Variable(hir::VariableId),
    Contract(hir::ContractId),
}

fn resolve_receiver_by_name(
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    contract_id: Option<hir::ContractId>,
    function_id: Option<hir::FunctionId>,
    receiver: &str,
) -> Option<ReceiverResolution> {
    if let Some(function_id) = function_id
        && let Some(var_id) = find_variable_in_function(gcx, function_id, receiver)
    {
        return Some(ReceiverResolution::Variable(var_id));
    }
    if let Some(contract_id) = contract_id
        && let Some(var_id) = find_variable_in_contract(gcx, contract_id, receiver)
    {
        return Some(ReceiverResolution::Variable(var_id));
    }
    find_contract_by_name(gcx, source_id, receiver).map(ReceiverResolution::Contract)
}

fn find_variable_in_function(
    gcx: Gcx<'_>,
    function_id: hir::FunctionId,
    receiver: &str,
) -> Option<hir::VariableId> {
    let func = gcx.hir.function(function_id);
    for &var_id in func.parameters.iter().chain(func.returns) {
        if variable_name_matches(gcx, var_id, receiver) {
            return Some(var_id);
        }
    }
    let body = func.body?;
    find_variable_in_block(gcx, &body, receiver)
}

fn find_variable_in_block(
    gcx: Gcx<'_>,
    block: &hir::Block<'_>,
    receiver: &str,
) -> Option<hir::VariableId> {
    for stmt in block.stmts {
        if let Some(var_id) = find_variable_in_stmt(gcx, stmt, receiver) {
            return Some(var_id);
        }
    }
    None
}

fn find_variable_in_stmt(
    gcx: Gcx<'_>,
    stmt: &hir::Stmt<'_>,
    receiver: &str,
) -> Option<hir::VariableId> {
    match &stmt.kind {
        hir::StmtKind::DeclSingle(var_id) => {
            if variable_name_matches(gcx, *var_id, receiver) {
                Some(*var_id)
            } else {
                None
            }
        }
        hir::StmtKind::DeclMulti(vars, _expr) => vars
            .iter()
            .flatten()
            .copied()
            .find(|var_id| variable_name_matches(gcx, *var_id, receiver)),
        hir::StmtKind::Block(block)
        | hir::StmtKind::UncheckedBlock(block)
        | hir::StmtKind::Loop(block, _) => find_variable_in_block(gcx, block, receiver),
        hir::StmtKind::If(_cond, then_branch, else_branch) => {
            find_variable_in_stmt(gcx, then_branch, receiver).or_else(|| {
                else_branch.and_then(|branch| find_variable_in_stmt(gcx, branch, receiver))
            })
        }
        hir::StmtKind::Try(stmt_try) => {
            for clause in stmt_try.clauses {
                for &var_id in clause.args {
                    if variable_name_matches(gcx, var_id, receiver) {
                        return Some(var_id);
                    }
                }
                if let Some(var_id) = find_variable_in_block(gcx, &clause.block, receiver) {
                    return Some(var_id);
                }
            }
            None
        }
        hir::StmtKind::Emit(_)
        | hir::StmtKind::Revert(_)
        | hir::StmtKind::Return(_)
        | hir::StmtKind::Break
        | hir::StmtKind::Continue
        | hir::StmtKind::Expr(_)
        | hir::StmtKind::Placeholder
        | hir::StmtKind::Err(_) => None,
    }
}

fn find_variable_in_contract(
    gcx: Gcx<'_>,
    contract_id: hir::ContractId,
    receiver: &str,
) -> Option<hir::VariableId> {
    for item_id in gcx.hir.contract_item_ids(contract_id) {
        let hir::ItemId::Variable(var_id) = item_id else {
            continue;
        };
        if variable_name_matches(gcx, var_id, receiver) {
            return Some(var_id);
        }
    }
    None
}

fn find_contract_by_name(
    gcx: Gcx<'_>,
    _source_id: hir::SourceId,
    receiver: &str,
) -> Option<hir::ContractId> {
    for contract_id in gcx.hir.contract_ids() {
        let contract = gcx.hir.contract(contract_id);
        if contract.name.as_str() == receiver {
            return Some(contract_id);
        }
    }
    None
}

fn variable_name_matches(gcx: Gcx<'_>, var_id: hir::VariableId, receiver: &str) -> bool {
    let var = gcx.hir.variable(var_id);
    let Some(name) = var.name else {
        return false;
    };
    name.as_str() == receiver
}

fn contract_at_offset(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    offset: TextSize,
) -> Option<hir::ContractId> {
    let mut best: Option<(TextRange, hir::ContractId)> = None;
    for contract_id in gcx.hir.contract_ids() {
        let contract = gcx.hir.contract(contract_id);
        if contract.source != source_id {
            continue;
        }
        let Some(range) = snapshot.span_to_text_range(contract.span) else {
            continue;
        };
        if !range_contains(range, offset) {
            continue;
        }
        let replace = best
            .as_ref()
            .map(|(best_range, _)| range_len(range) < range_len(*best_range))
            .unwrap_or(true);
        if replace {
            best = Some((range, contract_id));
        }
    }
    best.map(|(_, id)| id)
}

fn contract_scope_items(gcx: Gcx<'_>, contract_id: hir::ContractId) -> Vec<SemaCompletionItem> {
    let mut items = Vec::new();
    let contract = gcx.hir.contract(contract_id);
    let bases = if contract.linearized_bases.is_empty() {
        vec![contract_id]
    } else {
        contract.linearized_bases.to_vec()
    };

    for (idx, base_id) in bases.iter().enumerate() {
        let base = gcx.hir.contract(*base_id);
        for &item_id in base.items {
            if idx > 0 {
                let item = gcx.hir.item(item_id);
                if !item.is_visible_in_derived_contracts() {
                    continue;
                }
            }
            if let Some(item) = completion_item_for_item(gcx, item_id) {
                items.push(item);
            }
        }
    }

    items
}

fn local_items_for_function(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    function_id: hir::FunctionId,
    offset: TextSize,
) -> Vec<SemaCompletionItem> {
    let func = gcx.hir.function(function_id);
    let mut items = Vec::new();

    for &var_id in func.parameters.iter().chain(func.returns) {
        if var_declared_before_offset(snapshot, gcx, var_id, offset) {
            push_variable_item(gcx, var_id, &mut items);
        }
    }
    if let Some(body) = func.body {
        collect_local_vars_in_block(snapshot, gcx, &body, offset, &mut items);
    }

    items
}

fn collect_local_vars_in_block(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    block: &hir::Block<'_>,
    offset: TextSize,
    items: &mut Vec<SemaCompletionItem>,
) {
    for stmt in block.stmts {
        collect_local_vars_in_stmt(snapshot, gcx, stmt, offset, items);
    }
}

fn collect_local_vars_in_stmt(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    stmt: &hir::Stmt<'_>,
    offset: TextSize,
    items: &mut Vec<SemaCompletionItem>,
) {
    match &stmt.kind {
        hir::StmtKind::DeclSingle(var_id) => {
            if var_declared_before_offset(snapshot, gcx, *var_id, offset) {
                push_variable_item(gcx, *var_id, items);
            }
        }
        hir::StmtKind::DeclMulti(vars, _expr) => {
            for var_id in vars.iter().flatten() {
                if var_declared_before_offset(snapshot, gcx, *var_id, offset) {
                    push_variable_item(gcx, *var_id, items);
                }
            }
        }
        hir::StmtKind::Block(block)
        | hir::StmtKind::UncheckedBlock(block)
        | hir::StmtKind::Loop(block, _) => {
            collect_local_vars_in_block(snapshot, gcx, block, offset, items);
        }
        hir::StmtKind::If(_cond, then_branch, else_branch) => {
            collect_local_vars_in_stmt(snapshot, gcx, then_branch, offset, items);
            if let Some(else_branch) = else_branch {
                collect_local_vars_in_stmt(snapshot, gcx, else_branch, offset, items);
            }
        }
        hir::StmtKind::Try(stmt_try) => {
            for clause in stmt_try.clauses {
                for &var_id in clause.args {
                    if var_declared_before_offset(snapshot, gcx, var_id, offset) {
                        push_variable_item(gcx, var_id, items);
                    }
                }
                collect_local_vars_in_block(snapshot, gcx, &clause.block, offset, items);
            }
        }
        hir::StmtKind::Emit(_)
        | hir::StmtKind::Revert(_)
        | hir::StmtKind::Return(_)
        | hir::StmtKind::Break
        | hir::StmtKind::Continue
        | hir::StmtKind::Expr(_)
        | hir::StmtKind::Placeholder
        | hir::StmtKind::Err(_) => {}
    }
}

fn push_variable_item(gcx: Gcx<'_>, var_id: hir::VariableId, items: &mut Vec<SemaCompletionItem>) {
    let var = gcx.hir.variable(var_id);
    let Some(name) = var.name else {
        return;
    };
    if !var_kind_declares_in_scope(var.kind) {
        return;
    }
    items.push(SemaCompletionItem {
        label: name.to_string(),
        kind: SemaCompletionKind::Variable,
    });
}

fn var_declared_before_offset(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    var_id: hir::VariableId,
    offset: TextSize,
) -> bool {
    let var = gcx.hir.variable(var_id);
    let Some(range) = snapshot.span_to_text_range(var.span) else {
        return true;
    };
    range.start() <= offset
}

fn super_member_items(gcx: Gcx<'_>, contract_id: hir::ContractId) -> Vec<SemaCompletionItem> {
    let mut items = Vec::new();
    let contract = gcx.hir.contract(contract_id);
    if contract.linearized_bases.is_empty() {
        return items;
    }
    for &base_id in contract.linearized_bases.iter().skip(1) {
        let base = gcx.hir.contract(base_id);
        for &item_id in base.items {
            let item = gcx.hir.item(item_id);
            if !item.is_visible_in_derived_contracts() {
                continue;
            }
            let hir::ItemId::Function(function_id) = item_id else {
                continue;
            };
            let func = gcx.hir.function(function_id);
            if func.kind == FunctionKind::Modifier {
                continue;
            }
            if func.name.is_none() {
                continue;
            }
            items.push(SemaCompletionItem {
                label: gcx.item_name(function_id).to_string(),
                kind: SemaCompletionKind::Function,
            });
        }
    }
    items
}

fn member_items_for_type<'gcx>(
    gcx: Gcx<'gcx>,
    ty: Ty<'gcx>,
    source_id: hir::SourceId,
    contract_id: Option<hir::ContractId>,
) -> Vec<SemaCompletionItem> {
    let ty = default_memory_if_ref(gcx, ty);
    if let Some(target_contract_id) = contract_id_from_type(ty) {
        let base_accessible = contract_id
            .is_some_and(|current| contract_is_base_of(gcx, current, target_contract_id));
        return contract_type_items(gcx, target_contract_id, base_accessible);
    }

    completion_items_from_members(gcx, gcx.members_of(ty, source_id, contract_id))
}

fn completion_items_from_members<'gcx>(
    gcx: Gcx<'gcx>,
    members: &[Member<'gcx>],
) -> Vec<SemaCompletionItem> {
    let mut items = Vec::with_capacity(members.len());
    for member in members {
        let mut label = member.name.to_string();
        let kind = match member.res {
            Some(hir::Res::Item(hir::ItemId::Function(function_id))) => {
                let func = gcx.hir.function(function_id);
                if let Some(var_id) = func.gettee {
                    if let Some(name) = gcx.hir.variable(var_id).name {
                        label = name.to_string();
                    }
                    SemaCompletionKind::Variable
                } else {
                    SemaCompletionKind::Function
                }
            }
            Some(hir::Res::Item(hir::ItemId::Variable(var_id))) => {
                if let Some(name) = gcx.hir.variable(var_id).name {
                    label = name.to_string();
                }
                SemaCompletionKind::Variable
            }
            _ => match member.ty.kind {
                TyKind::FnPtr(_) => SemaCompletionKind::Function,
                _ => SemaCompletionKind::Variable,
            },
        };
        items.push(SemaCompletionItem { label, kind });
    }
    items
}

fn contract_type_items(
    gcx: Gcx<'_>,
    contract_id: hir::ContractId,
    base_accessible: bool,
) -> Vec<SemaCompletionItem> {
    contract_type_members(
        gcx,
        contract_id,
        base_accessible,
        ContractMemberAccess::Value,
    )
    .into_iter()
    .map(|member| {
        let kind = match member.ty.kind {
            TyKind::FnPtr(_) => SemaCompletionKind::Function,
            _ => SemaCompletionKind::Variable,
        };
        SemaCompletionItem {
            label: member.name.to_string(),
            kind,
        }
    })
    .collect()
}

fn contract_is_base_of(
    gcx: Gcx<'_>,
    current_contract: hir::ContractId,
    target_contract: hir::ContractId,
) -> bool {
    if current_contract == target_contract {
        return true;
    }
    let bases = gcx.hir.contract(current_contract).linearized_bases;
    if bases.is_empty() {
        return false;
    }
    bases.contains(&target_contract)
}

fn var_kind_declares_in_scope(kind: hir::VarKind) -> bool {
    !matches!(
        kind,
        hir::VarKind::Error | hir::VarKind::Event | hir::VarKind::Struct
    )
}

fn contract_id_for_symbol(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    symbol: &ResolvedSymbol,
) -> Option<hir::ContractId> {
    let source_id = snapshot.source_id_for_file(symbol.definition_file_id)?;
    let range = symbol.definition_range;
    for contract_id in gcx.hir.contract_ids() {
        let contract = gcx.hir.contract(contract_id);
        if contract.source != source_id {
            continue;
        }
        if ident_matches(snapshot, contract.name, range) {
            return Some(contract_id);
        }
    }
    None
}

fn variable_id_for_symbol(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    symbol: &ResolvedSymbol,
) -> Option<hir::VariableId> {
    let source_id = snapshot.source_id_for_file(symbol.definition_file_id)?;
    let range = symbol.definition_range;
    for var_id in gcx.hir.variable_ids() {
        let var = gcx.hir.variable(var_id);
        if var.source != source_id {
            continue;
        }
        let Some(name) = var.name else {
            continue;
        };
        if ident_matches(snapshot, name, range) {
            return Some(var_id);
        }
    }
    None
}

fn ident_matches(snapshot: &SemaSnapshot, ident: Ident, range: TextRange) -> bool {
    let Some(ident_range) = snapshot.span_to_text_range(ident.span) else {
        return false;
    };
    ident_range == range
}

fn range_len(range: TextRange) -> u32 {
    u32::from(range.len())
}
