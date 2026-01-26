use std::collections::HashMap;
use std::sync::Arc;

use sa_base_db::FileId;
use sa_span::{TextRange, TextSize};
use solar::ast::{DataLocation, ImportItems, ItemKind};
use solar::interface::{Span, source_map::SourceMap};
use solar::sema::ty::TyKind;
use solar::sema::{Gcx, Ty, builtins::Builtin, hir};

use crate::contract_members::{
    ContractMember, ContractMemberAccess, contract_id_from_type, contract_type_members,
};
use crate::exports;
use crate::ty_utils::default_memory_if_ref;
use crate::{ResolvedSymbol, SemaSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DefinitionKey {
    file_id: FileId,
    range: TextRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemaReference {
    file_id: FileId,
    range: TextRange,
}

impl SemaReference {
    pub fn file_id(&self) -> FileId {
        self.file_id
    }

    pub fn range(&self) -> TextRange {
        self.range
    }
}

pub(crate) struct SemaReferenceIndex {
    references: HashMap<DefinitionKey, Vec<SemaReference>>,
}

impl SemaReferenceIndex {
    pub(crate) fn new(snapshot: &SemaSnapshot) -> Self {
        let mut references = HashMap::new();
        let source_map = Arc::clone(&snapshot.source_map);
        let file_id_by_source = snapshot.file_id_by_source.clone();

        snapshot.with_gcx(|gcx| {
            for source_id in gcx.hir.source_ids() {
                let Some(file_id) = file_id_by_source.get(&source_id).copied() else {
                    continue;
                };
                let source = gcx.hir.source(source_id);
                let source_text = Arc::clone(&source.file.src);
                let mut collector = ReferenceCollector::new(
                    gcx,
                    Arc::clone(&source_map),
                    file_id_by_source.clone(),
                    source_id,
                    source_text,
                    file_id,
                    &mut references,
                );
                collector.collect_source(source);
            }
        });

        for refs in references.values_mut() {
            refs.sort_by(|left, right| {
                (left.file_id, left.range.start()).cmp(&(right.file_id, right.range.start()))
            });
            refs.dedup();
        }

        Self { references }
    }

    pub(crate) fn references_for(
        &self,
        definition_file_id: FileId,
        definition_range: TextRange,
    ) -> Option<&[SemaReference]> {
        let key = DefinitionKey {
            file_id: definition_file_id,
            range: definition_range,
        };
        self.references.get(&key).map(|refs| refs.as_slice())
    }
}

struct ReferenceCollector<'gcx, 'a> {
    gcx: Gcx<'gcx>,
    source_map: Arc<SourceMap>,
    file_id_by_source: HashMap<hir::SourceId, FileId>,
    source_id: hir::SourceId,
    source_text: Arc<String>,
    current_contract: Option<hir::ContractId>,
    import_name_counts: Option<HashMap<String, usize>>,
    current_file_id: FileId,
    references: &'a mut HashMap<DefinitionKey, Vec<SemaReference>>,
}

impl<'gcx, 'a> ReferenceCollector<'gcx, 'a> {
    fn new(
        gcx: Gcx<'gcx>,
        source_map: Arc<SourceMap>,
        file_id_by_source: HashMap<hir::SourceId, FileId>,
        source_id: hir::SourceId,
        source_text: Arc<String>,
        current_file_id: FileId,
        references: &'a mut HashMap<DefinitionKey, Vec<SemaReference>>,
    ) -> Self {
        Self {
            gcx,
            source_map,
            file_id_by_source,
            source_id,
            source_text,
            current_contract: None,
            import_name_counts: None,
            current_file_id,
            references,
        }
    }

    fn collect_source(&mut self, source: &hir::Source<'gcx>) {
        self.collect_import_aliases(source);
        for &item_id in source.items {
            self.visit_item(item_id);
        }
    }

    fn collect_import_aliases(&mut self, source: &hir::Source<'gcx>) {
        let Some(ast) = self
            .gcx
            .sources
            .get(self.source_id)
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

            if let ImportItems::Aliases(aliases) = &import.items {
                for (original, alias) in aliases.iter() {
                    let Some(item_id) = self.find_import_item(import_source_id, original.name)
                    else {
                        continue;
                    };
                    let alias = alias.as_ref().unwrap_or(original);
                    self.record_import_alias(item_id, alias.span);
                }
            }
        }
    }

    fn record_import_alias(&mut self, item_id: hir::ItemId, span: Span) {
        let Some(range) = self.span_to_text_range(span) else {
            return;
        };
        let Some(symbol) = self.symbol_for_item(item_id, range) else {
            return;
        };
        self.record_symbol_reference(&symbol, range);
    }

    fn ident_text_at_range(&self, range: TextRange) -> Option<String> {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        let snippet = self.source_text.get(start..end)?.trim();
        if snippet.is_empty() {
            return None;
        }
        Some(snippet.to_string())
    }

    fn is_import_name_ambiguous(&mut self, name: &str) -> bool {
        let source = self.gcx.hir.source(self.source_id);
        let counts = self.import_name_counts(source);
        counts.get(name).copied().unwrap_or(0) > 1
    }

    fn import_name_counts(&mut self, source: &hir::Source<'gcx>) -> &HashMap<String, usize> {
        if self.import_name_counts.is_none() {
            let counts = self.build_import_name_counts(source);
            self.import_name_counts = Some(counts);
        }
        self.import_name_counts.as_ref().expect("import counts")
    }

    fn build_import_name_counts(&self, source: &hir::Source<'gcx>) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        let Some(ast) = self
            .gcx
            .sources
            .get(self.source_id)
            .and_then(|source| source.ast.as_ref())
        else {
            return counts;
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
                ImportItems::Aliases(aliases) => {
                    for (original, alias) in aliases.iter() {
                        let alias = alias.as_ref().unwrap_or(original);
                        *counts.entry(alias.as_str().to_string()).or_insert(0) += 1;
                    }
                }
                ImportItems::Plain(Some(alias)) | ImportItems::Glob(alias) => {
                    *counts.entry(alias.as_str().to_string()).or_insert(0) += 1;
                }
                ImportItems::Plain(None) => {
                    for name in exports::exported_item_names(self.gcx, import_source_id) {
                        *counts.entry(name.as_str().to_string()).or_insert(0) += 1;
                    }
                }
            }
        }

        counts
    }

    fn find_import_item(
        &self,
        source_id: hir::SourceId,
        name: solar::interface::Symbol,
    ) -> Option<hir::ItemId> {
        exports::find_exported_item(self.gcx, source_id, name)
    }

    fn record_symbol_reference(&mut self, symbol: &ResolvedSymbol, range: TextRange) {
        let key = DefinitionKey {
            file_id: symbol.definition_file_id,
            range: symbol.definition_range,
        };
        self.references.entry(key).or_default().push(SemaReference {
            file_id: self.current_file_id,
            range,
        });
    }

    fn record_definition(&mut self, item_id: hir::ItemId) {
        let item = self.gcx.hir.item(item_id);
        let Some(name) = item.name() else {
            return;
        };
        let Some(range) = self.span_to_text_range(name.span) else {
            return;
        };
        let Some(symbol) = self.symbol_for_item(item_id, range) else {
            return;
        };
        self.record_symbol_reference(&symbol, symbol.definition_range);
    }

    fn visit_item(&mut self, item_id: hir::ItemId) {
        self.record_definition(item_id);
        match item_id {
            hir::ItemId::Contract(id) => {
                let contract = self.gcx.hir.contract(id);
                self.with_contract(Some(id), |this| {
                    for modifier in contract.bases_args {
                        this.visit_call_args(&modifier.args);
                    }
                    for &item_id in contract.items {
                        this.visit_item(item_id);
                    }
                    if let Some(ctor) = contract.ctor {
                        this.visit_function_id(ctor);
                    }
                    if let Some(fallback) = contract.fallback {
                        this.visit_function_id(fallback);
                    }
                    if let Some(receive) = contract.receive {
                        this.visit_function_id(receive);
                    }
                });
            }
            hir::ItemId::Function(id) => self.visit_function_id(id),
            hir::ItemId::Variable(id) => {
                let var = self.gcx.hir.variable(id);
                self.with_contract(var.contract, |this| this.visit_variable(var));
            }
            hir::ItemId::Struct(id) => {
                let strukt = self.gcx.hir.strukt(id);
                for &field in strukt.fields {
                    let var = self.gcx.hir.variable(field);
                    self.visit_variable(var);
                }
            }
            hir::ItemId::Enum(_id) => {}
            hir::ItemId::Udvt(id) => {
                let udvt = self.gcx.hir.udvt(id);
                self.visit_ty(&udvt.ty);
            }
            hir::ItemId::Event(id) => {
                let event = self.gcx.hir.event(id);
                for &param in event.parameters {
                    let var = self.gcx.hir.variable(param);
                    self.visit_variable(var);
                }
            }
            hir::ItemId::Error(id) => {
                let error = self.gcx.hir.error(id);
                for &param in error.parameters {
                    let var = self.gcx.hir.variable(param);
                    self.visit_variable(var);
                }
            }
        }
    }

    fn visit_function_id(&mut self, id: hir::FunctionId) {
        let func = self.gcx.hir.function(id);
        self.with_contract(func.contract, |this| this.visit_function(func));
    }

    fn visit_function(&mut self, func: &hir::Function<'gcx>) {
        for &param in func.parameters {
            let var = self.gcx.hir.variable(param);
            self.visit_variable(var);
        }
        for modifier in func.modifiers {
            self.visit_call_args(&modifier.args);
        }
        for &ret in func.returns {
            let var = self.gcx.hir.variable(ret);
            self.visit_variable(var);
        }
        if let Some(body) = func.body {
            for stmt in body.stmts {
                self.visit_stmt(stmt);
            }
        }
    }

    fn visit_variable(&mut self, var: &hir::Variable<'gcx>) {
        self.visit_ty(&var.ty);
        if let Some(expr) = var.initializer {
            self.visit_expr(expr);
        }
    }

    fn visit_stmt(&mut self, stmt: &hir::Stmt<'gcx>) {
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

    fn visit_expr(&mut self, expr: &hir::Expr<'gcx>) {
        match &expr.kind {
            hir::ExprKind::Call(callee, args, opts) => {
                match &callee.kind {
                    hir::ExprKind::Ident(res) => {
                        if let Some(range) = self.span_to_text_range(callee.span)
                            && let Some(symbol) = self.resolve_call_ident(res, args, range)
                        {
                            self.record_symbol_reference(&symbol, range);
                        }
                    }
                    hir::ExprKind::Member(base, ident) => {
                        if let Some(range) = self.span_to_text_range(ident.span)
                            && let Some(symbol) = if self.is_super_expr(base) {
                                self.resolve_super_member(ident, Some(args), range)
                            } else {
                                self.resolve_member_access(base, ident, Some(args), range)
                            }
                        {
                            self.record_symbol_reference(&symbol, range);
                        }
                        self.visit_expr(base);
                    }
                    _ => {
                        self.visit_expr(callee);
                    }
                }
                if let Some(opts) = opts {
                    for opt in *opts {
                        self.visit_expr(&opt.value);
                    }
                }
                self.visit_call_args(args);
            }
            hir::ExprKind::Ident(res) => {
                if let Some(range) = self.span_to_text_range(expr.span)
                    && let Some(symbol) = self.resolve_ident(res, range)
                {
                    self.record_symbol_reference(&symbol, range);
                }
            }
            hir::ExprKind::Member(base, ident) => {
                if let Some(range) = self.span_to_text_range(ident.span)
                    && let Some(symbol) = if self.is_super_expr(base) {
                        self.resolve_super_member(ident, None, range)
                    } else {
                        self.resolve_member_access(base, ident, None, range)
                    }
                {
                    self.record_symbol_reference(&symbol, range);
                }
                self.visit_expr(base);
            }
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
            hir::ExprKind::Lit(_) | hir::ExprKind::Err(_) => {}
            hir::ExprKind::New(ty) | hir::ExprKind::TypeCall(ty) | hir::ExprKind::Type(ty) => {
                self.visit_ty(ty);
            }
        }
    }

    fn visit_call_args(&mut self, args: &hir::CallArgs<'gcx>) {
        for expr in args.kind.exprs() {
            self.visit_expr(expr);
        }
    }

    fn visit_ty(&mut self, ty: &hir::Type<'gcx>) {
        match &ty.kind {
            hir::TypeKind::Custom(item_id) => {
                if let Some(range) = self.span_to_text_range(ty.span) {
                    if self.alias_is_ambiguous(ty.span) {
                        return;
                    }
                    if let Some(symbol) = self.resolve_item(*item_id, range) {
                        self.record_symbol_reference(&symbol, range);
                    }
                }
            }
            hir::TypeKind::Err(_) => {}
            hir::TypeKind::Array(array) => {
                self.visit_ty(&array.element);
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
                self.visit_ty(&mapping.key);
                self.visit_ty(&mapping.value);
            }
            hir::TypeKind::Elementary(_) => {}
        }
    }

    fn resolve_call_ident(
        &mut self,
        res: &[hir::Res],
        args: &hir::CallArgs<'gcx>,
        range: TextRange,
    ) -> Option<ResolvedSymbol> {
        let items = res
            .iter()
            .filter_map(|res| match res {
                hir::Res::Item(item_id) => Some(*item_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        let item_id = self.resolve_call_overloads(&items, args)?;
        self.resolve_item(item_id, range)
    }

    fn resolve_ident(&mut self, res: &[hir::Res], range: TextRange) -> Option<ResolvedSymbol> {
        let items = res
            .iter()
            .filter_map(|res| match res {
                hir::Res::Item(item_id) => Some(*item_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        if items.len() != 1 {
            return None;
        }
        let item_id = items[0];
        let item = self.gcx.hir.item(item_id);
        if item.source() != self.source_id
            && self
                .ident_text_at_range(range)
                .as_deref()
                .is_some_and(|name| self.is_import_name_ambiguous(name))
        {
            return None;
        }
        self.resolve_item(item_id, range)
    }

    fn resolve_item(&self, item_id: hir::ItemId, range: TextRange) -> Option<ResolvedSymbol> {
        self.symbol_for_item(item_id, range)
    }

    fn resolve_member_access(
        &mut self,
        base: &hir::Expr<'gcx>,
        ident: &solar::interface::Ident,
        args: Option<&hir::CallArgs<'gcx>>,
        range: TextRange,
    ) -> Option<ResolvedSymbol> {
        let ty = default_memory_if_ref(self.gcx, self.receiver_ty(base)?);
        let access = if args.is_some() {
            ContractMemberAccess::Call
        } else {
            ContractMemberAccess::Value
        };
        let items = if let Some(members) = self.contract_type_members(ty, access) {
            members
                .iter()
                .filter(|member| member.name == ident.name)
                .map(|member| member.item_id)
                .collect::<Vec<_>>()
        } else {
            let members = self
                .gcx
                .members_of(ty, self.source_id, self.current_contract);
            members
                .iter()
                .filter(|member| member.name == ident.name)
                .filter_map(|member| match member.res {
                    Some(hir::Res::Item(item_id)) => Some(item_id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        let item_id = match args {
            Some(args) => self.resolve_call_overloads(&items, args),
            None => {
                if items.len() == 1 {
                    Some(items[0])
                } else {
                    // Avoid arbitrary overload selection without expected type context.
                    None
                }
            }
        }?;

        self.resolve_item(item_id, range)
    }

    fn resolve_super_member(
        &mut self,
        ident: &solar::interface::Ident,
        args: Option<&hir::CallArgs<'gcx>>,
        range: TextRange,
    ) -> Option<ResolvedSymbol> {
        let item_id = self.super_member_item(ident, args)?;
        self.resolve_item(item_id, range)
    }

    fn receiver_ty(&mut self, expr: &hir::Expr<'gcx>) -> Option<Ty<'gcx>> {
        match &expr.kind {
            hir::ExprKind::Ident(res) => self.receiver_ty_from_res(res),
            hir::ExprKind::Member(base, ident) => self.member_ty(base, ident),
            hir::ExprKind::Call(callee, args, _opts) => self.call_result_ty(callee, args),
            hir::ExprKind::Index(base, _) => self.index_result_ty(base),
            hir::ExprKind::Tuple(exprs) => self.tuple_receiver_ty(exprs),
            hir::ExprKind::Type(ty) => Some(self.gcx.type_of_hir_ty(ty).make_type_type(self.gcx)),
            hir::ExprKind::TypeCall(ty) => Some(self.gcx.type_of_hir_ty(ty).make_meta(self.gcx)),
            hir::ExprKind::Payable(expr)
            | hir::ExprKind::Delete(expr)
            | hir::ExprKind::Unary(_, expr) => self.receiver_ty(expr),
            hir::ExprKind::Ternary(_, then_expr, else_expr) => {
                let then_ty = self.receiver_ty(then_expr)?;
                let else_ty = self.receiver_ty(else_expr)?;
                if then_ty == else_ty {
                    Some(then_ty)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn receiver_ty_from_res(&self, res: &[hir::Res]) -> Option<Ty<'gcx>> {
        res.iter().find_map(|res| match res {
            hir::Res::Builtin(Builtin::This | Builtin::Super) => self
                .current_contract
                .map(|contract| self.gcx.type_of_item(contract.into())),
            hir::Res::Err(_) => None,
            _ => Some(self.gcx.type_of_res(*res)),
        })
    }

    fn tuple_receiver_ty(&mut self, exprs: &[Option<&hir::Expr<'gcx>>]) -> Option<Ty<'gcx>> {
        if exprs.len() == 1 {
            return exprs[0].and_then(|expr| self.receiver_ty(expr));
        }
        None
    }

    fn member_ty(
        &mut self,
        base: &hir::Expr<'gcx>,
        ident: &solar::interface::Ident,
    ) -> Option<Ty<'gcx>> {
        let base_ty = default_memory_if_ref(self.gcx, self.receiver_ty(base)?);
        if let Some(members) = self.contract_type_members(base_ty, ContractMemberAccess::Value) {
            let mut matches = members.iter().filter(|member| member.name == ident.name);
            let member = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            return Some(member.ty);
        }

        let mut matches = self
            .gcx
            .members_of(base_ty, self.source_id, self.current_contract)
            .iter()
            .filter(|member| member.name == ident.name);
        let member = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(member.ty)
    }

    fn index_result_ty(&mut self, base: &hir::Expr<'gcx>) -> Option<Ty<'gcx>> {
        let base_ty = self.receiver_ty(base)?;
        let loc = base_ty.loc();
        match base_ty.peel_refs().kind {
            TyKind::Array(element, _) | TyKind::DynArray(element) => {
                Some(element.with_loc_if_ref_opt(self.gcx, loc))
            }
            TyKind::Mapping(_, value) => {
                let loc = loc.or(Some(DataLocation::Storage));
                Some(value.with_loc_if_ref_opt(self.gcx, loc))
            }
            TyKind::Slice(element) => Some(element.with_loc_if_ref_opt(self.gcx, loc)),
            _ => None,
        }
    }

    fn call_result_ty(
        &mut self,
        callee: &hir::Expr<'gcx>,
        args: &hir::CallArgs<'gcx>,
    ) -> Option<Ty<'gcx>> {
        match &callee.kind {
            hir::ExprKind::Ident(res) => self.call_result_ty_from_res(res, args),
            hir::ExprKind::Member(base, ident) => {
                self.call_result_ty_from_member(base, ident, args)
            }
            hir::ExprKind::New(ty) => {
                Some(default_memory_if_ref(self.gcx, self.gcx.type_of_hir_ty(ty)))
            }
            _ => {
                let callee_ty = self.receiver_ty(callee)?;
                self.call_result_ty_from_ty(callee_ty)
            }
        }
    }

    fn call_result_ty_from_res(
        &mut self,
        res: &[hir::Res],
        args: &hir::CallArgs<'gcx>,
    ) -> Option<Ty<'gcx>> {
        let items = res
            .iter()
            .filter_map(|res| match res {
                hir::Res::Item(item_id) => Some(*item_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        if let Some(item_id) = self.resolve_call_overloads(&items, args) {
            return self.call_return_ty_from_item(item_id);
        }
        if items.len() == 1
            && let Some(ty) = self.call_return_ty_from_item(items[0])
        {
            return Some(ty);
        }
        if let Some(common) = self.common_return_ty(&items) {
            return Some(common);
        }
        self.receiver_ty_from_res(res)
            .and_then(|ty| self.call_result_ty_from_ty(ty))
    }

    fn call_result_ty_from_member(
        &mut self,
        base: &hir::Expr<'gcx>,
        ident: &solar::interface::Ident,
        args: &hir::CallArgs<'gcx>,
    ) -> Option<Ty<'gcx>> {
        let base_ty = default_memory_if_ref(self.gcx, self.receiver_ty(base)?);
        if let Some(members) = self.contract_type_members(base_ty, ContractMemberAccess::Call) {
            let named_members = members
                .iter()
                .filter(|member| member.name == ident.name)
                .collect::<Vec<_>>();
            if named_members.is_empty() {
                return None;
            }
            let items = named_members
                .iter()
                .map(|member| member.item_id)
                .collect::<Vec<_>>();
            if let Some(item_id) = self.resolve_call_overloads(&items, args) {
                return self.call_return_ty_from_item(item_id);
            }
            if items.len() == 1
                && let Some(ty) = self.call_return_ty_from_item(items[0])
            {
                return Some(ty);
            }
            if let Some(common) = self.common_return_ty(&items) {
                return Some(common);
            }
            if named_members.len() == 1 {
                return self.call_result_ty_from_ty(named_members[0].ty);
            }
            return None;
        }

        let members = self
            .gcx
            .members_of(base_ty, self.source_id, self.current_contract);
        let named_members = members
            .iter()
            .filter(|member| member.name == ident.name)
            .collect::<Vec<_>>();
        if named_members.is_empty() {
            return None;
        }
        let items = named_members
            .iter()
            .filter_map(|member| match member.res {
                Some(hir::Res::Item(item_id)) => Some(item_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        if let Some(item_id) = self.resolve_call_overloads(&items, args) {
            return self.call_return_ty_from_item(item_id);
        }
        if items.len() == 1
            && let Some(ty) = self.call_return_ty_from_item(items[0])
        {
            return Some(ty);
        }
        if let Some(common) = self.common_return_ty(&items) {
            return Some(common);
        }
        if named_members.len() == 1 {
            return self.call_result_ty_from_ty(named_members[0].ty);
        }
        None
    }

    fn call_result_ty_from_ty(&self, ty: Ty<'gcx>) -> Option<Ty<'gcx>> {
        match ty.kind {
            TyKind::FnPtr(f) => Some(self.fn_call_return_ty(f.returns)),
            TyKind::Type(inner) => Some(default_memory_if_ref(self.gcx, inner)),
            TyKind::Event(..) | TyKind::Error(..) => Some(self.gcx.types.unit),
            TyKind::Ref(inner, _) => self.call_result_ty_from_ty(inner),
            _ => None,
        }
    }

    fn call_return_ty_from_item(&self, item_id: hir::ItemId) -> Option<Ty<'gcx>> {
        self.call_result_ty_from_ty(self.gcx.type_of_item(item_id))
    }

    fn common_return_ty(&self, items: &[hir::ItemId]) -> Option<Ty<'gcx>> {
        let mut iter = items.iter().copied();
        let first = iter.next()?;
        let mut ty = self.call_return_ty_from_item(first)?;
        for item_id in iter {
            let next = self.call_return_ty_from_item(item_id)?;
            if next != ty {
                return None;
            }
            ty = next;
        }
        Some(ty)
    }

    fn fn_call_return_ty(&self, returns: &'gcx [Ty<'gcx>]) -> Ty<'gcx> {
        match returns {
            [] => self.gcx.types.unit,
            [ty] => *ty,
            tys => self.gcx.mk_ty_tuple(tys),
        }
    }

    fn super_member_item(
        &mut self,
        ident: &solar::interface::Ident,
        args: Option<&hir::CallArgs<'gcx>>,
    ) -> Option<hir::ItemId> {
        let contract_id = self.current_contract?;
        let bases = self.linearized_bases(contract_id)?;
        let start = bases.iter().position(|&id| id == contract_id)?;
        for &base_id in bases.iter().skip(start + 1) {
            let candidates = self.contract_items_named(base_id, ident.name);
            if candidates.is_empty() {
                continue;
            }
            if let Some(args) = args {
                if let Some(item_id) = self.resolve_call_overloads(&candidates, args) {
                    return Some(item_id);
                }
                continue;
            }
            return candidates.first().copied();
        }
        None
    }

    fn contract_items_named(
        &self,
        contract_id: hir::ContractId,
        name: solar::interface::Symbol,
    ) -> Vec<hir::ItemId> {
        let contract = self.gcx.hir.contract(contract_id);
        contract
            .items
            .iter()
            .copied()
            .filter(|item_id| {
                self.gcx
                    .hir
                    .item(*item_id)
                    .name()
                    .is_some_and(|ident| ident.name == name)
            })
            .collect()
    }

    fn resolve_call_overloads(
        &mut self,
        items: &[hir::ItemId],
        args: &hir::CallArgs<'gcx>,
    ) -> Option<hir::ItemId> {
        if items.is_empty() {
            return None;
        }
        if items.len() == 1 {
            let item_id = items[0];
            let ty = self.gcx.type_of_item(item_id);
            let params = ty.parameters()?;
            return (params.len() == args.len()).then_some(item_id);
        }

        let arg_count = args.len();
        let candidates = items
            .iter()
            .copied()
            .filter(|item_id| {
                let ty = self.gcx.type_of_item(*item_id);
                ty.parameters()
                    .is_some_and(|params| params.len() == arg_count)
            })
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            return Some(candidates[0]);
        }
        if candidates.is_empty() {
            return None;
        }

        let arg_tys = self.arg_types(args);
        if let Some(arg_tys) = arg_tys {
            let mut matches = Vec::new();
            for item_id in candidates {
                let ty = self.gcx.type_of_item(item_id);
                let Some(params) = ty.parameters() else {
                    continue;
                };
                if params.len() != arg_tys.len() {
                    continue;
                }
                if arg_tys
                    .iter()
                    .copied()
                    .zip(params.iter().copied())
                    .all(|(arg, param)| arg.convert_implicit_to(param, self.gcx))
                {
                    matches.push(item_id);
                }
            }
            return self.select_from_matches(&matches);
        }

        if self.signatures_match(&candidates) {
            return self
                .select_by_c3_order(&candidates)
                .or_else(|| candidates.first().copied());
        }
        None
    }

    fn select_from_matches(&mut self, matches: &[hir::ItemId]) -> Option<hir::ItemId> {
        match matches {
            [] => None,
            [single] => Some(*single),
            _ => {
                if self.signatures_match(matches) {
                    self.select_by_c3_order(matches)
                        .or_else(|| matches.first().copied())
                } else {
                    None
                }
            }
        }
    }

    fn select_by_c3_order(&mut self, items: &[hir::ItemId]) -> Option<hir::ItemId> {
        let contract_id = self.current_contract?;
        let bases = self.linearized_bases(contract_id)?;
        let mut best = None;
        let mut best_idx = usize::MAX;

        for item_id in items {
            let item_contract = self.gcx.hir.item(*item_id).contract()?;
            let Some(position) = bases.iter().position(|&id| id == item_contract) else {
                continue;
            };
            if position < best_idx {
                best_idx = position;
                best = Some(*item_id);
            }
        }

        best
    }

    fn signatures_match(&self, items: &[hir::ItemId]) -> bool {
        let Some(first) = items.first().copied() else {
            return false;
        };
        let first_params = self.gcx.type_of_item(first).parameters();
        items.iter().all(|item_id| {
            let params = self.gcx.type_of_item(*item_id).parameters();
            params == first_params
        })
    }

    fn arg_types(&self, args: &hir::CallArgs<'gcx>) -> Option<Vec<Ty<'gcx>>> {
        match &args.kind {
            hir::CallArgsKind::Unnamed(exprs) => {
                let mut types = Vec::with_capacity(exprs.len());
                for expr in exprs.iter() {
                    match &expr.kind {
                        hir::ExprKind::Lit(lit) => types.push(self.gcx.type_of_lit(lit)),
                        _ => return None,
                    }
                }
                Some(types)
            }
            hir::CallArgsKind::Named(_) => None,
        }
    }

    fn leading_qualifier(&self, span: Span) -> Option<&str> {
        let range = self.span_to_text_range(span)?;
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        let snippet = self.source_text.get(start..end)?;
        let snippet = snippet.trim();
        let mut parts = snippet.split('.');
        let first = parts.next()?.trim();
        if parts.next().is_some() && !first.is_empty() {
            Some(first)
        } else {
            None
        }
    }

    fn alias_is_ambiguous(&self, span: Span) -> bool {
        let Some(qualifier) = self.leading_qualifier(span) else {
            return false;
        };
        let Some(source) = self.gcx.sources.get(self.source_id) else {
            return false;
        };
        let Some(ast) = source.ast.as_ref() else {
            return false;
        };

        let mut matches = 0;
        for (_, import) in ast.imports() {
            let Some(alias) = import.source_alias() else {
                continue;
            };
            if alias.as_str() == qualifier {
                matches += 1;
                if matches > 1 {
                    return true;
                }
            }
        }

        false
    }

    fn linearized_bases(
        &self,
        contract_id: hir::ContractId,
    ) -> Option<&'gcx [hir::ContractId]> {
        let contract = self.gcx.hir.contract(contract_id);
        if contract.linearization_failed() {
            return None;
        }
        Some(contract.linearized_bases)
    }


    fn contract_type_members(
        &mut self,
        ty: Ty<'gcx>,
        access: ContractMemberAccess,
    ) -> Option<Vec<ContractMember<'gcx>>> {
        let contract_id = contract_id_from_type(ty)?;
        let base_accessible = self.base_accessible_contract(contract_id);
        Some(contract_type_members(
            self.gcx,
            contract_id,
            base_accessible,
            access,
        ))
    }

    fn base_accessible_contract(&mut self, contract_id: hir::ContractId) -> bool {
        let Some(current_contract) = self.current_contract else {
            return false;
        };
        if current_contract == contract_id {
            return true;
        }
        let Some(bases) = self.linearized_bases(current_contract) else {
            return false;
        };
        bases.contains(&contract_id)
    }

    fn symbol_for_item(
        &self,
        item_id: hir::ItemId,
        origin_range: TextRange,
    ) -> Option<ResolvedSymbol> {
        let item = self.gcx.hir.item(item_id);
        let name = item.name()?;
        let name_str = name.as_str().to_string();
        let container = item
            .contract()
            .map(|contract_id| self.gcx.hir.contract(contract_id).name.as_str().to_string());
        let definition_range = self.span_to_text_range(name.span)?;
        let definition_file_id = *self.file_id_by_source.get(&item.source())?;
        let kind = match item_id {
            hir::ItemId::Contract(_) => crate::ResolvedSymbolKind::Contract,
            hir::ItemId::Function(id) => {
                let func = self.gcx.hir.function(id);
                if func.kind == solar::ast::FunctionKind::Modifier {
                    crate::ResolvedSymbolKind::Modifier
                } else {
                    crate::ResolvedSymbolKind::Function
                }
            }
            hir::ItemId::Struct(_) => crate::ResolvedSymbolKind::Struct,
            hir::ItemId::Enum(_) => crate::ResolvedSymbolKind::Enum,
            hir::ItemId::Event(_) => crate::ResolvedSymbolKind::Event,
            hir::ItemId::Error(_) => crate::ResolvedSymbolKind::Error,
            hir::ItemId::Udvt(_) => crate::ResolvedSymbolKind::Udvt,
            hir::ItemId::Variable(id) => {
                let var = self.gcx.hir.variable(id);
                if !matches!(var.kind, hir::VarKind::Global | hir::VarKind::State) {
                    return None;
                }
                crate::ResolvedSymbolKind::Variable
            }
        };
        Some(ResolvedSymbol {
            kind,
            name: name_str,
            container,
            definition_file_id,
            definition_range,
            origin_range,
        })
    }

    fn is_super_expr(&self, expr: &hir::Expr<'gcx>) -> bool {
        matches!(&expr.kind, hir::ExprKind::Ident(res) if res.iter().any(|res| matches!(res, hir::Res::Builtin(Builtin::Super))))
    }

    fn span_to_text_range(&self, span: Span) -> Option<TextRange> {
        let range = self.source_map.span_to_range(span).ok()?;
        let start = TextSize::try_from(range.start).ok()?;
        let end = TextSize::try_from(range.end).ok()?;
        Some(TextRange::new(start, end))
    }

    fn with_contract(&mut self, contract: Option<hir::ContractId>, f: impl FnOnce(&mut Self)) {
        let prev = self.current_contract;
        self.current_contract = contract;
        f(self);
        self.current_contract = prev;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sa_paths::NormalizedPath;
    use sa_span::{TextRange, TextSize};
    use sa_test_support::{extract_offsets, find_range};
    use sa_test_utils::{Fixture, FixtureBuilder};

    use crate::SemaSnapshot;

    fn snapshot_for_fixture(fixture: &Fixture) -> SemaSnapshot {
        let path_to_file_id = fixture
            .vfs_snapshot()
            .iter()
            .map(|(file_id, path)| (path.clone(), file_id))
            .collect::<HashMap<NormalizedPath, _>>();
        SemaSnapshot::new(
            fixture.config(),
            fixture.vfs_snapshot(),
            &path_to_file_id,
            None,
            true,
        )
        .expect("sema snapshot")
    }

    #[test]
    fn references_include_import_aliases() {
        let foo_text = r#"
pragma solidity ^0.8.20;

contract Foo {}
"#
        .trim();
        let main_text = r#"
pragma solidity ^0.8.20;

import {Foo as Bar} from "./Foo.sol";

contract Main {}
"#
        .trim();

        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file("src/Foo.sol", foo_text)
            .file("src/Main.sol", main_text)
            .build()
            .expect("fixture");

        let snapshot = snapshot_for_fixture(&fixture);
        let foo_file_id = fixture.file_id("src/Foo.sol").expect("foo file id");
        let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
        let foo_def_range = find_range(foo_text, "Foo");
        let alias_range = find_range(main_text, "Bar");

        let refs = snapshot
            .references_for_definition(foo_file_id, foo_def_range)
            .expect("foo references");
        assert!(
            refs.iter()
                .any(|reference| reference.file_id() == main_file_id
                    && reference.range() == alias_range),
            "expected import alias to be recorded as a reference"
        );
    }

    #[test]
    fn references_resolve_super_member_calls() {
        let text = r#"
pragma solidity ^0.8.20;

contract Base {
    function basePing() public virtual {}
}

contract Child is Base {
    function basePing() public override {}

    function call() public {
        super.basePing();
    }
}
"#
        .trim();

        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file("src/Main.sol", text)
            .build()
            .expect("fixture");

        let snapshot = snapshot_for_fixture(&fixture);
        let file_id = fixture.file_id("src/Main.sol").expect("main file id");
        let def_range = find_range(text, "basePing");
        let call_site = "super.basePing";
        let call_pos = text.find(call_site).expect("super call");
        let start = call_pos + "super.".len();
        let end = start + "basePing".len();
        let call_range = TextRange::new(TextSize::from(start as u32), TextSize::from(end as u32));

        let refs = snapshot
            .references_for_definition(file_id, def_range)
            .expect("basePing references");
        assert!(
            refs.iter()
                .any(|reference| reference.file_id() == file_id && reference.range() == call_range),
            "expected super member call to resolve to base definition"
        );
    }

    #[test]
    fn references_resolve_overloaded_calls_by_literal_args() {
        let (text, offsets) = extract_offsets(
            r#"
pragma solidity ^0.8.20;

contract Overloads {
    function /*def_u_start*/foo/*def_u_end*/(uint256 value) public {}
    function /*def_b_start*/foo/*def_b_end*/(bool value) public {}

    function call() public {
        /*call_u_start*/foo/*call_u_end*/(1);
        /*call_b_start*/foo/*call_b_end*/(true);
    }
}
"#
            .trim(),
            &[
                "/*def_u_start*/",
                "/*def_u_end*/",
                "/*def_b_start*/",
                "/*def_b_end*/",
                "/*call_u_start*/",
                "/*call_u_end*/",
                "/*call_b_start*/",
                "/*call_b_end*/",
            ],
        );

        let def_u_range = TextRange::new(offsets[0], offsets[1]);
        let def_b_range = TextRange::new(offsets[2], offsets[3]);
        let call_u_range = TextRange::new(offsets[4], offsets[5]);
        let call_b_range = TextRange::new(offsets[6], offsets[7]);

        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file("src/Main.sol", text)
            .build()
            .expect("fixture");

        let snapshot = snapshot_for_fixture(&fixture);
        let file_id = fixture.file_id("src/Main.sol").expect("main file id");

        let uint_refs = snapshot
            .references_for_definition(file_id, def_u_range)
            .expect("uint overload refs");
        assert!(
            uint_refs
                .iter()
                .any(|reference| reference.range() == call_u_range),
            "expected uint overload to capture literal call"
        );
        assert!(
            !uint_refs
                .iter()
                .any(|reference| reference.range() == call_b_range),
            "expected bool literal call to map to bool overload"
        );

        let bool_refs = snapshot
            .references_for_definition(file_id, def_b_range)
            .expect("bool overload refs");
        assert!(
            bool_refs
                .iter()
                .any(|reference| reference.range() == call_b_range),
            "expected bool overload to capture literal call"
        );
    }
}
