use std::collections::HashSet;

use sa_base_db::FileId;
use sa_span::{TextRange, TextSize, is_ident_byte, range_contains};
use sa_syntax::Parse;
use sa_syntax::ast::{Item, ItemKind, Stmt, StmtKind, TypeKind as AstTypeKind, VariableDefinition};
use solar::ast::{FunctionKind, ImportItems, ItemKind as SolarItemKind};
use solar::interface::{Ident, Span};
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
    pub detail: Option<String>,
    pub origin: Option<String>,
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
            match identifier_completion_context(self, gcx, source_id, offset) {
                IdentifierCompletionContext::StructLiteralFields { struct_id } => {
                    struct_field_completion_items(gcx, struct_id)
                }
                IdentifierCompletionContext::Scope {
                    contract_id,
                    function_id,
                } => {
                    let mut items = Vec::new();
                    let mut seen = HashSet::new();

                    collect_source_items(gcx, source_id, &mut items, &mut seen);
                    collect_imported_items(gcx, source_id, &mut items, &mut seen);

                    if let Some(contract_id) = contract_id {
                        for item in contract_scope_items(gcx, contract_id) {
                            push_completion_item(&mut items, &mut seen, item);
                        }
                    }

                    if let Some(function_id) = function_id {
                        for item in local_items_for_function(self, gcx, function_id, offset) {
                            push_completion_item(&mut items, &mut seen, item);
                        }
                    }

                    items
                }
            }
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
                        member_items_for_variable(self, gcx, source_id, current_contract, var_id)
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
                    member_items_for_variable(self, gcx, source_id, current_contract, var_id)
                }
                Some(ReceiverResolution::Contract(contract_id)) => {
                    let ty = gcx.type_of_item(contract_id.into()).make_type_type(gcx);
                    member_items_for_type(gcx, ty, source_id, current_contract)
                }
                None => member_items_for_unresolved_receiver(
                    self,
                    gcx,
                    source_id,
                    offset,
                    receiver,
                    current_contract,
                )
                .unwrap_or_default(),
            }
        });

        Some(items)
    }
}

#[derive(Clone, Copy, Debug)]
enum IdentifierCompletionContext {
    StructLiteralFields {
        struct_id: hir::StructId,
    },
    Scope {
        contract_id: Option<hir::ContractId>,
        function_id: Option<hir::FunctionId>,
    },
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

    let detail = match item_id {
        hir::ItemId::Function(_)
        | hir::ItemId::Variable(_)
        | hir::ItemId::Event(_)
        | hir::ItemId::Error(_) => detail_for_item_id(gcx, item_id),
        _ => None,
    };

    Some(SemaCompletionItem {
        label: name.to_string(),
        kind,
        detail,
        origin: None,
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
        let SolarItemKind::Import(import) = &item.kind else {
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
                    detail: None,
                    origin: None,
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

fn identifier_completion_context(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    offset: TextSize,
) -> IdentifierCompletionContext {
    let contract_id = contract_at_offset(snapshot, gcx, source_id, offset);
    let function_id = function_at_offset(snapshot, gcx, source_id, offset);
    if let Some(function_id) = function_id {
        let func = gcx.hir.function(function_id);
        if let Some(body) = func.body {
            let mut finder = IdentifierCompletionContextFinder {
                snapshot,
                gcx,
                source_id,
                offset,
                context: None,
            };
            if let Some(context) = finder.find_in_body(&body) {
                return context;
            }
        }
    }
    IdentifierCompletionContext::Scope {
        contract_id,
        function_id,
    }
}

fn struct_field_completion_items(
    gcx: Gcx<'_>,
    struct_id: hir::StructId,
) -> Vec<SemaCompletionItem> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let strukt = gcx.hir.strukt(struct_id);
    for &field_id in strukt.fields {
        let var = gcx.hir.variable(field_id);
        let Some(name) = var.name else {
            continue;
        };
        let item = SemaCompletionItem {
            label: name.as_str().to_string(),
            kind: SemaCompletionKind::Variable,
            detail: None,
            origin: None,
        };
        push_completion_item(&mut items, &mut seen, item);
    }
    items
}

struct IdentifierCompletionContextFinder<'a, 'gcx> {
    snapshot: &'a SemaSnapshot,
    gcx: Gcx<'gcx>,
    source_id: hir::SourceId,
    offset: TextSize,
    context: Option<IdentifierCompletionContext>,
}

impl<'a, 'gcx> IdentifierCompletionContextFinder<'a, 'gcx> {
    fn find_in_body(&mut self, body: &hir::Block<'gcx>) -> Option<IdentifierCompletionContext> {
        for stmt in body.stmts {
            self.visit_stmt(stmt);
            if self.context.is_some() {
                break;
            }
        }
        self.context
    }

    fn visit_stmt(&mut self, stmt: &hir::Stmt<'gcx>) {
        if self.context.is_some() {
            return;
        }
        match &stmt.kind {
            hir::StmtKind::DeclSingle(var_id) => {
                let var = self.gcx.hir.variable(*var_id);
                self.visit_variable(var);
            }
            hir::StmtKind::DeclMulti(vars, expr) => {
                for var in vars.iter().flatten() {
                    let var = self.gcx.hir.variable(*var);
                    self.visit_variable(var);
                }
                self.visit_expr(expr);
            }
            hir::StmtKind::Block(block)
            | hir::StmtKind::UncheckedBlock(block)
            | hir::StmtKind::Loop(block, _) => {
                for stmt in block.stmts {
                    self.visit_stmt(stmt);
                }
            }
            hir::StmtKind::Emit(expr) | hir::StmtKind::Revert(expr) => {
                self.visit_expr(expr);
            }
            hir::StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.visit_expr(expr);
                }
            }
            hir::StmtKind::Break
            | hir::StmtKind::Continue
            | hir::StmtKind::Placeholder
            | hir::StmtKind::Err(_) => {}
            hir::StmtKind::If(cond, then_branch, else_branch) => {
                self.visit_expr(cond);
                self.visit_stmt(then_branch);
                if let Some(else_branch) = else_branch {
                    self.visit_stmt(else_branch);
                }
            }
            hir::StmtKind::Try(stmt_try) => {
                self.visit_expr(&stmt_try.expr);
                for clause in stmt_try.clauses {
                    for &arg in clause.args {
                        let var = self.gcx.hir.variable(arg);
                        self.visit_variable(var);
                    }
                    for stmt in clause.block.stmts {
                        self.visit_stmt(stmt);
                    }
                }
            }
            hir::StmtKind::Expr(expr) => self.visit_expr(expr),
        }
    }

    fn visit_variable(&mut self, var: &hir::Variable<'gcx>) {
        if self.context.is_some() {
            return;
        }
        if let Some(expr) = var.initializer {
            self.visit_expr(expr);
        }
    }

    fn visit_expr(&mut self, expr: &hir::Expr<'gcx>) {
        if self.context.is_some() {
            return;
        }
        if let hir::ExprKind::Call(callee, args, _) = &expr.kind
            && self.handle_named_arg_context(callee, args)
        {
            return;
        }
        let in_expr = self
            .snapshot
            .span_to_text_range(expr.span)
            .map(|range| range_contains(range, self.offset))
            .unwrap_or(true);
        if !in_expr {
            return;
        }

        match &expr.kind {
            hir::ExprKind::Call(callee, args, opts) => {
                self.visit_expr(callee);
                if let Some(opts) = opts {
                    for opt in *opts {
                        self.visit_expr(&opt.value);
                    }
                }
                for expr in args.kind.exprs() {
                    self.visit_expr(expr);
                }
            }
            hir::ExprKind::Ident(_) | hir::ExprKind::Lit(_) | hir::ExprKind::Err(_) => {}
            hir::ExprKind::Member(base, _) => self.visit_expr(base),
            hir::ExprKind::Array(exprs) => {
                for expr in exprs.iter() {
                    self.visit_expr(expr);
                }
            }
            hir::ExprKind::Assign(lhs, _, rhs) | hir::ExprKind::Binary(lhs, _, rhs) => {
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            hir::ExprKind::Delete(expr)
            | hir::ExprKind::Payable(expr)
            | hir::ExprKind::Unary(_, expr) => self.visit_expr(expr),
            hir::ExprKind::Index(expr, index) => {
                self.visit_expr(expr);
                if let Some(index) = index {
                    self.visit_expr(index);
                }
            }
            hir::ExprKind::Slice(expr, start, end) => {
                self.visit_expr(expr);
                if let Some(start) = start {
                    self.visit_expr(start);
                }
                if let Some(end) = end {
                    self.visit_expr(end);
                }
            }
            hir::ExprKind::Ternary(cond, then_branch, else_branch) => {
                self.visit_expr(cond);
                self.visit_expr(then_branch);
                self.visit_expr(else_branch);
            }
            hir::ExprKind::Tuple(exprs) => {
                for expr in exprs.iter().copied().flatten() {
                    self.visit_expr(expr);
                }
            }
            hir::ExprKind::New(ty) | hir::ExprKind::TypeCall(ty) | hir::ExprKind::Type(ty) => {
                self.visit_type(ty);
            }
        }
    }

    fn handle_named_arg_context(
        &mut self,
        callee: &hir::Expr<'gcx>,
        args: &hir::CallArgs<'gcx>,
    ) -> bool {
        let Some(struct_id) = self.struct_id_for_callee(callee) else {
            return false;
        };
        if let hir::CallArgsKind::Named(named_args) = args.kind {
            for named_arg in named_args {
                if let Some(range) = self.snapshot.span_to_text_range(named_arg.name.span)
                    && (range_contains(range, self.offset) || range.end() == self.offset)
                {
                    self.context =
                        Some(IdentifierCompletionContext::StructLiteralFields { struct_id });
                    return true;
                }
            }
        }

        if self.struct_literal_name_position(args) {
            self.context = Some(IdentifierCompletionContext::StructLiteralFields { struct_id });
            return true;
        }
        false
    }

    fn struct_literal_name_position(&self, args: &hir::CallArgs<'gcx>) -> bool {
        let Some(range) = self.snapshot.span_to_text_range(args.span) else {
            return false;
        };
        if !(range_contains(range, self.offset) || range.end() == self.offset) {
            return false;
        }
        let source = self.gcx.hir.source(self.source_id);
        let text = source.file.src.as_str();
        let bytes = text.as_bytes();
        let start = usize::from(range.start());
        let mut idx = usize::from(self.offset);
        if idx == 0 || idx > bytes.len() || bytes.is_empty() {
            return false;
        }
        idx = idx.saturating_sub(1);
        if idx >= bytes.len() {
            idx = bytes.len().saturating_sub(1);
        }

        let mut i = idx;
        loop {
            if i < start {
                break;
            }
            let b = bytes[i];
            if b == b':' {
                return false;
            }
            if b == b',' || b == b'{' {
                return true;
            }
            if is_ident_byte(b) || b.is_ascii_whitespace() {
                if i == 0 {
                    break;
                }
                i -= 1;
                continue;
            }
            return false;
        }
        false
    }

    fn visit_type(&mut self, ty: &hir::Type<'gcx>) {
        if self.context.is_some() {
            return;
        }
        match &ty.kind {
            hir::TypeKind::Array(array) => {
                self.visit_type(&array.element);
                if let Some(expr) = array.size {
                    self.visit_expr(expr);
                }
            }
            hir::TypeKind::Function(func) => {
                for &param in func.parameters {
                    let var = self.gcx.hir.variable(param);
                    self.visit_variable(var);
                }
                for &ret in func.returns {
                    let var = self.gcx.hir.variable(ret);
                    self.visit_variable(var);
                }
            }
            hir::TypeKind::Mapping(mapping) => {
                self.visit_type(&mapping.key);
                self.visit_type(&mapping.value);
            }
            _ => {}
        }
    }

    fn struct_id_for_callee(&self, callee: &hir::Expr<'gcx>) -> Option<hir::StructId> {
        match &callee.kind {
            hir::ExprKind::Ident(res) => self.struct_id_from_res(res),
            hir::ExprKind::Type(ty) | hir::ExprKind::TypeCall(ty) => match ty.kind {
                hir::TypeKind::Custom(hir::ItemId::Struct(id)) => Some(id),
                _ => None,
            },
            hir::ExprKind::Member(base, ident) => self.struct_id_from_member(base, ident),
            _ => None,
        }
    }

    fn struct_id_from_member(
        &self,
        base: &hir::Expr<'gcx>,
        ident: &solar::interface::Ident,
    ) -> Option<hir::StructId> {
        let hir::ExprKind::Ident(res) = &base.kind else {
            return None;
        };
        let mut contract_id = None;
        for res in res.iter() {
            if let hir::Res::Item(hir::ItemId::Contract(id)) = res {
                if contract_id.is_some() {
                    return None;
                }
                contract_id = Some(*id);
            }
        }
        let contract_id = contract_id?;
        let contract = self.gcx.hir.contract(contract_id);
        let mut found = None;
        for &item_id in contract.items {
            let hir::ItemId::Struct(struct_id) = item_id else {
                continue;
            };
            let Some(name) = self.gcx.item_name_opt(item_id) else {
                continue;
            };
            if name.name != ident.name {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(struct_id);
        }
        found
    }

    fn struct_id_from_res(&self, res: &[hir::Res]) -> Option<hir::StructId> {
        let mut found = None;
        for res in res {
            if let hir::Res::Item(hir::ItemId::Struct(id)) = res {
                if found.is_some() {
                    return None;
                }
                found = Some(*id);
            }
        }
        found
    }
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
    if let Some(resolution) = resolve_import_alias_receiver(gcx, source_id, receiver) {
        return Some(resolution);
    }
    find_contract_by_name(gcx, source_id, receiver).map(ReceiverResolution::Contract)
}

fn resolve_import_alias_receiver(
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    receiver: &str,
) -> Option<ReceiverResolution> {
    let item_id = resolve_import_alias_item(gcx, source_id, receiver)?;
    match item_id {
        hir::ItemId::Contract(contract_id) => Some(ReceiverResolution::Contract(contract_id)),
        _ => None,
    }
}

fn resolve_import_alias_item(
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    alias_name: &str,
) -> Option<hir::ItemId> {
    let source = gcx.sources.get(source_id)?;
    let ast = source.ast.as_ref()?;
    let hir_source = gcx.hir.source(source_id);
    let mut resolved = None;

    for (item_id, item) in ast.items.iter_enumerated() {
        let SolarItemKind::Import(import) = &item.kind else {
            continue;
        };
        let Some(import_source_id) = hir_source
            .imports
            .iter()
            .find_map(|(import_id, source_id)| (*import_id == item_id).then_some(*source_id))
        else {
            continue;
        };

        let ImportItems::Aliases(aliases) = &import.items else {
            continue;
        };
        for (original, alias) in aliases.iter() {
            let alias_ident = alias.as_ref().unwrap_or(original);
            if alias_ident.as_str() != alias_name {
                continue;
            }
            let Some(item_id) = exports::find_exported_item(gcx, import_source_id, original.name)
            else {
                continue;
            };
            if resolved.is_some() {
                return None;
            }
            resolved = Some(item_id);
        }
    }

    resolved
}

fn member_items_for_variable(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    contract_id: Option<hir::ContractId>,
    var_id: hir::VariableId,
) -> Vec<SemaCompletionItem> {
    let ty = gcx.type_of_item(var_id.into());
    let items = member_items_for_type(gcx, ty, source_id, contract_id);
    if !items.is_empty() {
        return items;
    }
    if matches!(ty.kind, TyKind::Err(_))
        && let Some(items) =
            member_items_for_variable_alias(snapshot, gcx, source_id, contract_id, var_id)
    {
        return items;
    }
    items
}

fn member_items_for_variable_alias(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    contract_id: Option<hir::ContractId>,
    var_id: hir::VariableId,
) -> Option<Vec<SemaCompletionItem>> {
    let var = gcx.hir.variable(var_id);
    let hir::TypeKind::Custom(_) = var.ty.kind else {
        return None;
    };
    let alias_name = type_name_from_span(snapshot, gcx, source_id, var.ty.span)?;
    let item_id = resolve_import_alias_item(gcx, source_id, alias_name.as_str())?;
    let hir::ItemId::Contract(_) = item_id else {
        return None;
    };
    let ty = gcx.type_of_item(item_id);
    Some(member_items_for_type(gcx, ty, source_id, contract_id))
}

fn type_name_from_span(
    snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    span: Span,
) -> Option<String> {
    let range = snapshot.span_to_text_range(span)?;
    let source = gcx.hir.source(source_id);
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    let snippet = source.file.src.get(start..end)?.trim();
    if snippet.is_empty() || snippet.contains('.') {
        return None;
    }
    Some(snippet.to_string())
}

fn member_items_for_unresolved_receiver(
    _snapshot: &SemaSnapshot,
    gcx: Gcx<'_>,
    source_id: hir::SourceId,
    offset: TextSize,
    receiver: &str,
    contract_id: Option<hir::ContractId>,
) -> Option<Vec<SemaCompletionItem>> {
    let source = gcx.hir.source(source_id);
    let parse = sa_syntax::parse_file(source.file.src.as_str());
    let var = find_local_var_definition(&parse, offset, receiver)?;
    let type_ident = match &var.ty.kind {
        AstTypeKind::Custom(path) => path.get_ident(),
        _ => None,
    }?;
    let item_id = exports::find_exported_item(gcx, source_id, type_ident.name).or_else(|| {
        find_contract_by_name(gcx, source_id, type_ident.as_str()).map(hir::ItemId::Contract)
    })?;
    let ty = gcx.type_of_item(item_id);
    let items = member_items_for_type(gcx, ty, source_id, contract_id);
    (!items.is_empty()).then_some(items)
}

fn find_local_var_definition<'a>(
    parse: &'a Parse,
    offset: TextSize,
    receiver: &str,
) -> Option<&'a VariableDefinition<'static>> {
    let mut best: Option<(TextRange, &VariableDefinition<'static>)> = None;
    for item in parse.tree().items.iter() {
        find_local_var_in_item(parse, item, offset, receiver, &mut best);
    }
    best.map(|(_, var)| var)
}

fn find_local_var_in_item<'a>(
    parse: &'a Parse,
    item: &'a Item<'static>,
    offset: TextSize,
    receiver: &str,
    best: &mut Option<(TextRange, &'a VariableDefinition<'static>)>,
) {
    let Some(item_range) = parse.span_to_text_range(item.span) else {
        return;
    };
    if !range_contains(item_range, offset) {
        return;
    }
    match &item.kind {
        ItemKind::Contract(contract) => {
            for item in contract.body.iter() {
                find_local_var_in_item(parse, item, offset, receiver, best);
            }
        }
        ItemKind::Function(function) => {
            let Some(body) = function.body.as_ref() else {
                return;
            };
            let Some(body_range) = parse.span_to_text_range(function.body_span) else {
                return;
            };
            if !range_contains(body_range, offset) {
                return;
            }
            for param in function.header.parameters.vars.iter() {
                consider_local_var(parse, param, receiver, offset, body_range, best);
            }
            if let Some(returns) = function.header.returns.as_ref() {
                for param in returns.vars.iter() {
                    consider_local_var(parse, param, receiver, offset, body_range, best);
                }
            }
            find_local_var_in_block(parse, body, offset, receiver, best);
        }
        _ => {}
    }
}

fn find_local_var_in_block<'a>(
    parse: &'a Parse,
    block: &'a sa_syntax::ast::Block<'static>,
    offset: TextSize,
    receiver: &str,
    best: &mut Option<(TextRange, &'a VariableDefinition<'static>)>,
) {
    let Some(block_range) = parse.span_to_text_range(block.span) else {
        return;
    };
    if !range_contains(block_range, offset) {
        return;
    }
    for stmt in block.stmts.iter() {
        find_local_var_in_stmt(parse, stmt, offset, receiver, block_range, best);
    }
}

fn find_local_var_in_stmt<'a>(
    parse: &'a Parse,
    stmt: &'a Stmt<'static>,
    offset: TextSize,
    receiver: &str,
    scope: TextRange,
    best: &mut Option<(TextRange, &'a VariableDefinition<'static>)>,
) {
    match &stmt.kind {
        StmtKind::DeclSingle(var) => {
            consider_local_var(parse, var, receiver, offset, scope, best);
        }
        StmtKind::DeclMulti(vars, _) => {
            for var in vars.iter() {
                if let sa_syntax::ast::interface::SpannedOption::Some(var) = var {
                    consider_local_var(parse, var, receiver, offset, scope, best);
                }
            }
        }
        StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
            find_local_var_in_block(parse, block, offset, receiver, best);
        }
        StmtKind::For { init, body, .. } => {
            let Some(body_range) = parse.span_to_text_range(body.span) else {
                return;
            };
            if !range_contains(body_range, offset) {
                return;
            }
            if let Some(init) = init.as_deref() {
                find_local_var_in_stmt(parse, init, offset, receiver, body_range, best);
            }
            find_local_var_in_stmt(parse, body, offset, receiver, body_range, best);
        }
        StmtKind::If(_, then_branch, else_branch) => {
            if let Some(then_range) = parse.span_to_text_range(then_branch.span)
                && range_contains(then_range, offset)
            {
                find_local_var_in_stmt(parse, then_branch, offset, receiver, then_range, best);
            }
            if let Some(else_branch) = else_branch.as_deref()
                && let Some(else_range) = parse.span_to_text_range(else_branch.span)
                && range_contains(else_range, offset)
            {
                find_local_var_in_stmt(parse, else_branch, offset, receiver, else_range, best);
            }
        }
        StmtKind::While(_, body) | StmtKind::DoWhile(body, _) => {
            let Some(body_range) = parse.span_to_text_range(body.span) else {
                return;
            };
            if !range_contains(body_range, offset) {
                return;
            }
            find_local_var_in_stmt(parse, body, offset, receiver, body_range, best);
        }
        StmtKind::Try(stmt_try) => {
            for clause in stmt_try.clauses.iter() {
                let Some(clause_range) = parse.span_to_text_range(clause.span) else {
                    continue;
                };
                if !range_contains(clause_range, offset) {
                    continue;
                }
                for param in clause.args.vars.iter() {
                    consider_local_var(parse, param, receiver, offset, clause_range, best);
                }
                find_local_var_in_block(parse, &clause.block, offset, receiver, best);
            }
        }
        _ => {}
    }
}

fn consider_local_var<'a>(
    parse: &'a Parse,
    var: &'a VariableDefinition<'static>,
    receiver: &str,
    offset: TextSize,
    scope: TextRange,
    best: &mut Option<(TextRange, &'a VariableDefinition<'static>)>,
) {
    let Some(name) = var.name else {
        return;
    };
    if name.as_str() != receiver {
        return;
    }
    let Some(range) = parse.span_to_text_range(name.span) else {
        return;
    };
    if range.start() > offset || !range_contains(scope, offset) {
        return;
    }
    let replace = best
        .as_ref()
        .map(|(best_scope, _)| scope.len() < best_scope.len())
        .unwrap_or(true);
    if replace {
        *best = Some((scope, var));
    }
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
    if let Some((_, id)) = best {
        return Some(id);
    }

    let source = gcx.hir.source(source_id);
    let name = contract_name_at_offset_fallback(source.file.src.as_str(), offset)?;
    gcx.hir.contract_ids().find(|id| {
        let contract = gcx.hir.contract(*id);
        contract.source == source_id && contract.name.as_str() == name
    })
}

fn contract_name_at_offset_fallback(text: &str, offset: TextSize) -> Option<String> {
    let offset = usize::from(offset).min(text.len());
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let mut scanner = FallbackScanner::new(text);
    let mut brace_depth = 0usize;

    while let Some(token) = scanner.next_token() {
        match token.kind {
            FallbackTokenKind::Punct('{') => brace_depth += 1,
            FallbackTokenKind::Punct('}') => brace_depth = brace_depth.saturating_sub(1),
            FallbackTokenKind::Ident(ref ident)
                if brace_depth == 0 && is_contract_keyword(ident) =>
            {
                if let Some(decl) = parse_contract_decl(&mut scanner, text, token.start) {
                    brace_depth = 0;
                    if offset >= decl.start && offset <= decl.close_brace {
                        return Some(decl.name);
                    }
                }
            }
            _ => {}
        }
    }

    None
}

fn is_contract_keyword(ident: &str) -> bool {
    matches!(ident, "contract" | "library" | "interface")
}

struct ContractDecl {
    start: usize,
    close_brace: usize,
    name: String,
}

fn parse_contract_decl(
    scanner: &mut FallbackScanner<'_>,
    text: &str,
    contract_start: usize,
) -> Option<ContractDecl> {
    let name_token = scanner.next_token()?;
    let name = match name_token.kind {
        FallbackTokenKind::Ident(ident) => ident,
        _ => return None,
    };

    let mut open_brace: Option<usize> = None;
    while let Some(token) = scanner.next_token() {
        if let FallbackTokenKind::Punct('{') = token.kind {
            open_brace = Some(token.start);
            break;
        }
    }
    let open_brace = open_brace?;
    let close_brace = matching_close_brace(text, open_brace).unwrap_or(text.len());
    scanner.idx = close_brace.saturating_add(1);

    Some(ContractDecl {
        start: contract_start,
        close_brace,
        name,
    })
}

fn matching_close_brace(text: &str, open_brace: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if open_brace >= bytes.len() {
        return None;
    }
    let mut depth = 0usize;
    let mut i = open_brace.saturating_add(1);
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'/' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'/' {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if next == b'*' {
                i += 2;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        if b == b'\'' || b == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            if depth == 0 {
                return Some(i);
            }
            depth = depth.saturating_sub(1);
        }
        i += 1;
    }
    None
}

fn skip_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start.saturating_add(1);
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i = i.saturating_add(2);
            continue;
        }
        if b == quote {
            return i.saturating_add(1);
        }
        i += 1;
    }
    bytes.len()
}

struct FallbackScanner<'a> {
    bytes: &'a [u8],
    idx: usize,
}

struct FallbackToken {
    kind: FallbackTokenKind,
    start: usize,
}

enum FallbackTokenKind {
    Ident(String),
    Punct(char),
}

impl<'a> FallbackScanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            idx: 0,
        }
    }

    fn next_token(&mut self) -> Option<FallbackToken> {
        self.skip_trivia();
        if self.idx >= self.bytes.len() {
            return None;
        }
        let start = self.idx;
        let b = self.bytes[self.idx];
        if is_ident_byte(b) {
            self.idx += 1;
            while self.idx < self.bytes.len() && is_ident_byte(self.bytes[self.idx]) {
                self.idx += 1;
            }
            let ident = std::str::from_utf8(&self.bytes[start..self.idx])
                .ok()?
                .to_string();
            Some(FallbackToken {
                kind: FallbackTokenKind::Ident(ident),
                start,
            })
        } else {
            self.idx += 1;
            Some(FallbackToken {
                kind: FallbackTokenKind::Punct(b as char),
                start,
            })
        }
    }

    fn skip_trivia(&mut self) {
        while self.idx < self.bytes.len() {
            let b = self.bytes[self.idx];
            if b.is_ascii_whitespace() {
                self.idx += 1;
                continue;
            }
            if b == b'/' && self.idx + 1 < self.bytes.len() {
                let next = self.bytes[self.idx + 1];
                if next == b'/' {
                    self.idx += 2;
                    while self.idx < self.bytes.len() && self.bytes[self.idx] != b'\n' {
                        self.idx += 1;
                    }
                    continue;
                }
                if next == b'*' {
                    self.idx += 2;
                    while self.idx + 1 < self.bytes.len() {
                        if self.bytes[self.idx] == b'*' && self.bytes[self.idx + 1] == b'/' {
                            self.idx += 2;
                            break;
                        }
                        self.idx += 1;
                    }
                    continue;
                }
            }
            if b == b'\'' || b == b'"' {
                self.idx = skip_string(self.bytes, self.idx);
                continue;
            }
            break;
        }
    }
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
        detail: None,
        origin: None,
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
                detail: detail_for_item_id(gcx, item_id),
                origin: Some(base.name.as_str().to_string()),
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
    let receiver_contract = receiver_contract_id(ty);
    if let Some(target_contract_id) = contract_id_from_type(ty) {
        let base_accessible = contract_id
            .is_some_and(|current| contract_is_base_of(gcx, current, target_contract_id));
        return contract_type_items(
            gcx,
            target_contract_id,
            base_accessible,
            Some(target_contract_id),
        );
    }

    completion_items_from_members(
        gcx,
        gcx.members_of(ty, source_id, contract_id),
        receiver_contract,
    )
}

fn completion_items_from_members<'gcx>(
    gcx: Gcx<'gcx>,
    members: &[Member<'gcx>],
    current_contract: Option<hir::ContractId>,
) -> Vec<SemaCompletionItem> {
    let mut items = Vec::with_capacity(members.len());
    for member in members {
        let mut label = member.name.to_string();
        let (kind, detail) = match member.res {
            Some(hir::Res::Item(hir::ItemId::Function(function_id))) => {
                let func = gcx.hir.function(function_id);
                if let Some(var_id) = func.gettee {
                    if let Some(name) = gcx.hir.variable(var_id).name {
                        label = name.to_string();
                    }
                    (
                        SemaCompletionKind::Variable,
                        detail_for_item_id(gcx, hir::ItemId::Variable(var_id)),
                    )
                } else {
                    (
                        SemaCompletionKind::Function,
                        detail_for_item_id(gcx, hir::ItemId::Function(function_id)),
                    )
                }
            }
            Some(hir::Res::Item(hir::ItemId::Variable(var_id))) => {
                if let Some(name) = gcx.hir.variable(var_id).name {
                    label = name.to_string();
                }
                (
                    SemaCompletionKind::Variable,
                    detail_for_item_id(gcx, hir::ItemId::Variable(var_id)),
                )
            }
            _ => match member.ty.kind {
                TyKind::FnPtr(_) => (SemaCompletionKind::Function, detail_for_ty(gcx, member.ty)),
                _ => (SemaCompletionKind::Variable, detail_for_ty(gcx, member.ty)),
            },
        };
        let origin = match member.res {
            Some(hir::Res::Item(item_id)) => origin_for_item_id(gcx, item_id, current_contract),
            _ => Some("builtin".to_string()),
        };
        items.push(SemaCompletionItem {
            label,
            kind,
            detail,
            origin,
        });
    }
    items
}

fn contract_type_items(
    gcx: Gcx<'_>,
    contract_id: hir::ContractId,
    base_accessible: bool,
    receiver_contract: Option<hir::ContractId>,
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
        let detail = detail_for_item_id(gcx, member.item_id);
        let origin = origin_for_item_id(gcx, member.item_id, receiver_contract);
        SemaCompletionItem {
            label: member.name.to_string(),
            kind,
            detail,
            origin,
        }
    })
    .collect()
}

fn detail_for_item_id(gcx: Gcx<'_>, item_id: hir::ItemId) -> Option<String> {
    let ty = gcx.type_of_item(item_id);
    detail_for_ty(gcx, ty)
}

fn detail_for_ty<'gcx>(gcx: Gcx<'gcx>, ty: Ty<'gcx>) -> Option<String> {
    match ty.kind {
        TyKind::FnPtr(_) => {
            let params = ty.parameters().unwrap_or_default();
            let returns = ty.returns().unwrap_or_default();
            Some(format_signature(gcx, params, returns))
        }
        TyKind::Event(tys, _) | TyKind::Error(tys, _) => Some(format_params(gcx, tys)),
        _ => Some(ty.display(gcx).to_string()),
    }
}

fn format_signature<'gcx>(gcx: Gcx<'gcx>, params: &[Ty<'gcx>], returns: &[Ty<'gcx>]) -> String {
    format!(
        "({}) -> ({})",
        format_type_list(gcx, params),
        format_type_list(gcx, returns)
    )
}

fn format_params<'gcx>(gcx: Gcx<'gcx>, params: &[Ty<'gcx>]) -> String {
    format!("({})", format_type_list(gcx, params))
}

fn format_type_list<'gcx>(gcx: Gcx<'gcx>, tys: &[Ty<'gcx>]) -> String {
    tys.iter()
        .map(|ty| ty.display(gcx).to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn origin_for_item_id(
    gcx: Gcx<'_>,
    item_id: hir::ItemId,
    receiver_contract: Option<hir::ContractId>,
) -> Option<String> {
    let item = gcx.hir.item(item_id);
    let contract_id = item.contract()?;
    if receiver_contract.is_some_and(|current| current == contract_id) {
        return None;
    }
    Some(gcx.hir.contract(contract_id).name.as_str().to_string())
}

fn receiver_contract_id(ty: Ty<'_>) -> Option<hir::ContractId> {
    match ty.kind {
        TyKind::Contract(contract_id) => Some(contract_id),
        TyKind::Type(inner) => match inner.kind {
            TyKind::Contract(contract_id) => Some(contract_id),
            _ => None,
        },
        _ => None,
    }
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
