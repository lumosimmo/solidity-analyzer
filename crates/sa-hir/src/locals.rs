use sa_base_db::FileInput;
use sa_span::{TextRange, TextSize, range_contains};
use sa_syntax::Parse;
use sa_syntax::ast::{
    Block, CallArgs, Expr, ExprKind, IndexKind, Item, ItemFunction, ItemKind, Stmt, StmtKind,
    StmtTry, TryCatchClause, VariableDefinition, interface::SpannedOption,
};

use crate::HirDatabase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDefKind {
    Parameter,
    NamedReturn,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDef {
    name: String,
    kind: LocalDefKind,
    range: TextRange,
    scope: TextRange,
}

impl LocalDef {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> LocalDefKind {
        self.kind
    }

    pub fn range(&self) -> TextRange {
        self.range
    }

    pub fn scope(&self) -> TextRange {
        self.scope
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalScopes {
    defs: Vec<LocalDef>,
}

impl LocalScopes {
    pub fn resolve(&self, name: &str, offset: TextSize) -> Option<LocalDef> {
        self.defs
            .iter()
            .filter(|def| {
                def.name == name
                    && (range_contains(def.scope, offset) || range_contains(def.range, offset))
                    && def.range.start() <= offset
            })
            .min_by_key(|def| u32::from(def.scope.len()))
            .cloned()
    }

    pub fn defs(&self) -> &[LocalDef] {
        &self.defs
    }
}

unsafe impl salsa::Update for LocalScopes {
    unsafe fn maybe_update(old_pointer: *mut Self, new_value: Self) -> bool {
        let old = unsafe { &mut *old_pointer };
        if *old == new_value {
            false
        } else {
            *old = new_value;
            true
        }
    }
}

#[salsa::tracked(returns(ref))]
pub fn local_scopes_for_file(db: &dyn HirDatabase, file: FileInput) -> LocalScopes {
    let text = file.text(db);
    let parse = sa_syntax::parse_file(text.as_ref());
    LocalScopeCollector::new(&parse).collect()
}

pub fn local_scopes(db: &dyn HirDatabase, file_id: sa_base_db::FileId) -> LocalScopes {
    local_scopes_for_file(db, db.file_input(file_id)).clone()
}

pub fn local_references(
    db: &dyn HirDatabase,
    file_id: sa_base_db::FileId,
    local: &LocalDef,
) -> Vec<TextRange> {
    let text = db.file_input(file_id).text(db);
    let parse = sa_syntax::parse_file(text.as_ref());
    let locals = local_scopes(db, file_id);
    let mut collector = LocalReferenceCollector::new(&parse, &locals, local);
    collector.collect();
    collector.ranges.push(local.range());
    collector
        .ranges
        .sort_by_key(|range| (u32::from(range.start()), u32::from(range.end())));
    collector.ranges.dedup();
    collector.ranges
}

struct LocalScopeCollector<'a> {
    parse: &'a Parse,
    defs: Vec<LocalDef>,
    scopes: Vec<TextRange>,
}

impl<'a> LocalScopeCollector<'a> {
    fn new(parse: &'a Parse) -> Self {
        Self {
            parse,
            defs: Vec::new(),
            scopes: Vec::new(),
        }
    }

    fn collect(mut self) -> LocalScopes {
        for item in self.parse.tree().items.iter() {
            self.collect_item(item);
        }
        LocalScopes { defs: self.defs }
    }

    fn collect_item(&mut self, item: &Item<'_>) {
        match &item.kind {
            ItemKind::Contract(contract) => {
                for item in contract.body.iter() {
                    self.collect_item(item);
                }
            }
            ItemKind::Function(function) => self.collect_function(function),
            _ => {}
        }
    }

    fn collect_function(&mut self, function: &ItemFunction<'_>) {
        let Some(body) = function.body.as_ref() else {
            return;
        };
        let Some(body_range) = self.parse.span_to_text_range(body.span) else {
            return;
        };
        let header_range = self
            .parse
            .span_to_text_range(function.header.span)
            .unwrap_or(body_range);
        let params_range = self
            .parse
            .span_to_text_range(function.header.parameters.span);
        let returns_range = function
            .header
            .returns
            .as_ref()
            .and_then(|returns| self.parse.span_to_text_range(returns.span));
        let header_param_scope = match (params_range, returns_range) {
            (Some(params), Some(returns)) if params.end() <= returns.start() => {
                Some(TextRange::new(params.end(), returns.start()))
            }
            (Some(params), None) if params.end() <= header_range.end() => {
                Some(TextRange::new(params.end(), header_range.end()))
            }
            _ => None,
        };

        if let Some(scope) = header_param_scope.filter(|range| !range.is_empty()) {
            self.push_scope(scope);
            for param in function.header.parameters.vars.iter() {
                self.add_param(param, LocalDefKind::Parameter);
            }
            self.pop_scope();
        }

        self.push_scope(body_range);
        for param in function.header.parameters.vars.iter() {
            self.add_param(param, LocalDefKind::Parameter);
        }
        if let Some(returns) = function.header.returns.as_ref() {
            for param in returns.vars.iter() {
                self.add_param(param, LocalDefKind::NamedReturn);
            }
        }
        self.collect_block_stmts(body);
        self.pop_scope();
    }

    fn add_param(&mut self, param: &VariableDefinition<'_>, kind: LocalDefKind) {
        let Some(name) = param.name else {
            return;
        };
        let Some(range) = self.parse.span_to_text_range(name.span) else {
            return;
        };
        let Some(scope) = self.scopes.last().copied() else {
            return;
        };
        let name = self.parse.with_session(|| name.to_string());
        self.defs.push(LocalDef {
            name,
            kind,
            range,
            scope,
        });
    }

    fn collect_block_stmts(&mut self, block: &Block<'_>) {
        for stmt in block.stmts.iter() {
            self.collect_stmt(stmt);
        }
    }

    fn collect_stmt(&mut self, stmt: &Stmt<'_>) {
        match &stmt.kind {
            StmtKind::DeclSingle(var) => {
                self.add_local(var);
            }
            StmtKind::DeclMulti(vars, _) => {
                for var in vars.iter() {
                    if let SpannedOption::Some(var) = var {
                        self.add_local(var);
                    }
                }
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.collect_block(block);
            }
            StmtKind::For { init, body, .. } => {
                self.collect_for(stmt, init.as_deref(), body);
            }
            StmtKind::If(_, then_branch, else_branch) => {
                self.collect_stmt_with_scope(then_branch);
                if let Some(else_branch) = else_branch.as_deref() {
                    self.collect_stmt_with_scope(else_branch);
                }
            }
            StmtKind::While(_, body) | StmtKind::DoWhile(body, _) => {
                self.collect_stmt_with_scope(body);
            }
            StmtKind::Try(stmt_try) => {
                self.collect_try(stmt_try);
            }
            _ => {}
        }
    }

    fn collect_block(&mut self, block: &Block<'_>) {
        let Some(scope_range) = self.parse.span_to_text_range(block.span) else {
            return;
        };
        self.push_scope(scope_range);
        self.collect_block_stmts(block);
        self.pop_scope();
    }

    fn collect_for(&mut self, stmt: &Stmt<'_>, init: Option<&Stmt<'_>>, body: &Stmt<'_>) {
        let Some(scope_range) = self.parse.span_to_text_range(stmt.span) else {
            return;
        };
        self.push_scope(scope_range);
        if let Some(init) = init {
            self.collect_stmt(init);
        }
        self.collect_stmt(body);
        self.pop_scope();
    }

    fn collect_try(&mut self, stmt_try: &StmtTry<'_>) {
        for clause in stmt_try.clauses.iter() {
            self.collect_try_clause(clause);
        }
    }

    fn collect_try_clause(&mut self, clause: &TryCatchClause<'_>) {
        let Some(scope_range) = self.parse.span_to_text_range(clause.block.span) else {
            return;
        };
        self.push_scope(scope_range);
        for param in clause.args.vars.iter() {
            self.add_param(param, LocalDefKind::Parameter);
        }
        self.collect_block_stmts(&clause.block);
        self.pop_scope();
    }

    fn collect_stmt_with_scope(&mut self, stmt: &Stmt<'_>) {
        let Some(scope_range) = self.parse.span_to_text_range(stmt.span) else {
            return;
        };
        self.push_scope(scope_range);
        self.collect_stmt(stmt);
        self.pop_scope();
    }

    fn add_local(&mut self, var: &VariableDefinition<'_>) {
        let Some(name) = var.name else {
            return;
        };
        let Some(range) = self.parse.span_to_text_range(name.span) else {
            return;
        };
        let Some(scope) = self.scopes.last().copied() else {
            return;
        };
        let name = self.parse.with_session(|| name.to_string());
        self.defs.push(LocalDef {
            name,
            kind: LocalDefKind::Local,
            range,
            scope,
        });
    }

    fn push_scope(&mut self, range: TextRange) {
        self.scopes.push(range);
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
}

struct LocalReferenceCollector<'a> {
    parse: &'a Parse,
    locals: &'a LocalScopes,
    target: &'a LocalDef,
    ranges: Vec<TextRange>,
}

impl<'a> LocalReferenceCollector<'a> {
    fn new(parse: &'a Parse, locals: &'a LocalScopes, target: &'a LocalDef) -> Self {
        Self {
            parse,
            locals,
            target,
            ranges: Vec::new(),
        }
    }

    fn collect(&mut self) {
        for item in self.parse.tree().items.iter() {
            self.collect_item(item);
        }
    }

    fn collect_item(&mut self, item: &Item<'_>) {
        match &item.kind {
            ItemKind::Contract(contract) => {
                for item in contract.body.iter() {
                    self.collect_item(item);
                }
            }
            ItemKind::Function(function) => {
                for modifier in function.header.modifiers.iter() {
                    self.collect_call_args(&modifier.arguments);
                }
                if let Some(body) = function.body.as_ref() {
                    self.collect_block(body);
                }
            }
            _ => {}
        }
    }

    fn collect_block(&mut self, block: &Block<'_>) {
        for stmt in block.stmts.iter() {
            self.collect_stmt(stmt);
        }
    }

    fn collect_stmt(&mut self, stmt: &Stmt<'_>) {
        match &stmt.kind {
            StmtKind::DeclSingle(var) => {
                if let Some(expr) = var.initializer.as_deref() {
                    self.collect_expr(expr);
                }
            }
            StmtKind::DeclMulti(_, expr) => {
                self.collect_expr(expr);
            }
            StmtKind::Block(block) | StmtKind::UncheckedBlock(block) => {
                self.collect_block(block);
            }
            StmtKind::For {
                init,
                cond,
                next,
                body,
            } => {
                if let Some(init) = init.as_deref() {
                    self.collect_stmt(init);
                }
                if let Some(cond) = cond.as_deref() {
                    self.collect_expr(cond);
                }
                if let Some(next) = next.as_deref() {
                    self.collect_expr(next);
                }
                self.collect_stmt(body);
            }
            StmtKind::If(cond, then_branch, else_branch) => {
                self.collect_expr(cond);
                self.collect_stmt(then_branch);
                if let Some(else_branch) = else_branch.as_deref() {
                    self.collect_stmt(else_branch);
                }
            }
            StmtKind::While(cond, body) => {
                self.collect_expr(cond);
                self.collect_stmt(body);
            }
            StmtKind::DoWhile(body, cond) => {
                self.collect_stmt(body);
                self.collect_expr(cond);
            }
            StmtKind::Try(stmt_try) => {
                self.collect_expr(stmt_try.expr.as_ref());
                for clause in stmt_try.clauses.iter() {
                    self.collect_block(&clause.block);
                }
            }
            StmtKind::Emit(_, args) | StmtKind::Revert(_, args) => {
                self.collect_call_args(args);
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr.as_deref() {
                    self.collect_expr(expr);
                }
            }
            StmtKind::Expr(expr) => {
                self.collect_expr(expr);
            }
            _ => {}
        }
    }

    fn collect_expr(&mut self, expr: &Expr<'_>) {
        match &expr.kind {
            ExprKind::Ident(ident) => {
                self.record_ident(*ident);
            }
            ExprKind::Array(items) => {
                for item in items.iter() {
                    self.collect_expr(item);
                }
            }
            ExprKind::Assign(lhs, _, rhs) | ExprKind::Binary(lhs, _, rhs) => {
                self.collect_expr(lhs);
                self.collect_expr(rhs);
            }
            ExprKind::Call(callee, args) => {
                self.collect_expr(callee);
                self.collect_call_args(args);
            }
            ExprKind::CallOptions(callee, args) => {
                self.collect_expr(callee);
                for arg in args.iter() {
                    self.collect_expr(arg.value.as_ref());
                }
            }
            ExprKind::Delete(expr) => {
                self.collect_expr(expr);
            }
            ExprKind::Index(expr, index) => {
                self.collect_expr(expr);
                self.collect_index(index);
            }
            ExprKind::Member(expr, _) => {
                self.collect_expr(expr);
            }
            ExprKind::Payable(args) => {
                self.collect_call_args(args);
            }
            ExprKind::Ternary(cond, then_expr, else_expr) => {
                self.collect_expr(cond);
                self.collect_expr(then_expr);
                self.collect_expr(else_expr);
            }
            ExprKind::Tuple(items) => {
                for item in items.iter() {
                    if let SpannedOption::Some(expr) = item {
                        self.collect_expr(expr);
                    }
                }
            }
            ExprKind::Unary(_, expr) => {
                self.collect_expr(expr);
            }
            ExprKind::Lit(_, _) | ExprKind::New(_) | ExprKind::Type(_) | ExprKind::TypeCall(_) => {}
        }
    }

    fn collect_call_args(&mut self, args: &CallArgs<'_>) {
        for expr in args.exprs() {
            self.collect_expr(expr);
        }
    }

    fn collect_index(&mut self, index: &IndexKind<'_>) {
        match index {
            IndexKind::Index(expr) => {
                if let Some(expr) = expr.as_deref() {
                    self.collect_expr(expr);
                }
            }
            IndexKind::Range(start, end) => {
                if let Some(expr) = start.as_deref() {
                    self.collect_expr(expr);
                }
                if let Some(expr) = end.as_deref() {
                    self.collect_expr(expr);
                }
            }
        }
    }

    fn record_ident(&mut self, ident: sa_syntax::ast::interface::Ident) {
        let Some(range) = self.parse.span_to_text_range(ident.span) else {
            return;
        };
        let name = self.parse.with_session(|| ident.to_string());
        let Some(resolved) = self.locals.resolve(&name, range.start()) else {
            return;
        };
        if resolved.range() == self.target.range() {
            self.ranges.push(range);
        }
    }
}
