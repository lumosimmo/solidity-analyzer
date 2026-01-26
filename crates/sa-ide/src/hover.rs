use sa_base_db::{FileId, ProjectId};
use sa_def::{DefEntry, DefKind};
use sa_hir::{Definition, HirDatabase, LocalDef, LocalDefKind, Semantics, lowered_program};
use sa_span::{TextRange, TextSize};
use sa_syntax::{
    Parse,
    ast::{Item, ItemKind, VariableDefinition},
    tokens::ident_range_at_offset,
};

use crate::syntax_utils::{
    docs_for_item_with_inheritdoc, find_item_by_name_range, format_function_signature,
    format_param, sema_function_signature_for_entry, sema_variable_label_for_entry, type_text,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverResult {
    pub range: TextRange,
    pub contents: String,
}

pub fn hover(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    offset: TextSize,
) -> Option<HoverResult> {
    let hover_text = db.file_input(file_id).text(db);
    let hover_range = ident_range_at_offset(hover_text.as_ref(), offset);
    let semantics = Semantics::new(db, project_id);
    let definition = semantics.resolve_definition(file_id, offset)?;
    match definition {
        Definition::Global(def_id) => {
            let program = lowered_program(db, project_id);
            let entry = program.def_map().entry(def_id)?;

            // Parse the definition file once for both label and docs
            let def_file_id = entry.location().file_id();
            let text = db.file_input(def_file_id).text(db);
            let parse = sa_syntax::parse_file(text.as_ref());

            let label = build_label(db, project_id, &parse, text.as_ref(), entry);
            let docs = docs_for_entry_with_parse(db, project_id, def_file_id, &parse, entry);
            let contents = format_hover_contents(&label, docs.as_deref());

            Some(HoverResult {
                range: hover_range.unwrap_or_else(|| entry.location().range()),
                contents,
            })
        }
        Definition::Local(local) => {
            let parse = sa_syntax::parse_file(hover_text.as_ref());
            let label = local_label(&parse, hover_text.as_ref(), &local);
            Some(HoverResult {
                range: hover_range.unwrap_or_else(|| local.range()),
                contents: format_hover_contents(&label, None),
            })
        }
    }
}

fn format_hover_contents(label: &str, docs: Option<&str>) -> String {
    let code = format!("```solidity\n{label}\n```");
    match docs {
        Some(doc) if !doc.is_empty() => format!("{code}\n\n{doc}"),
        _ => code,
    }
}

fn build_label(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    parse: &Parse,
    text: &str,
    entry: &DefEntry,
) -> String {
    match entry.kind() {
        DefKind::Function | DefKind::Modifier => {
            if let Some(signature) = sema_function_signature_for_entry(db, project_id, entry) {
                return signature.label;
            }
        }
        DefKind::Variable => {
            if let Some(label) = sema_variable_label_for_entry(db, project_id, entry) {
                return label;
            }
        }
        _ => {}
    }

    build_label_with_parse(parse, text, entry)
}

fn build_label_with_parse(parse: &Parse, text: &str, entry: &DefEntry) -> String {
    let name = entry.location().name();

    if let Some(item) = find_item_by_name_range(parse, entry.container(), entry.location().range())
    {
        match &item.kind {
            ItemKind::Function(function) => {
                return format_function_signature(parse, text, function);
            }
            ItemKind::Variable(variable) => {
                let ty =
                    type_text(parse, text, &variable.ty).unwrap_or_else(|| "unknown".to_string());
                return format!("{ty} {name}");
            }
            ItemKind::Contract(contract) => {
                return format!("{} {name}", contract.kind.to_str());
            }
            _ => {}
        }
    }

    format!("{} {name}", def_kind_label(entry.kind()))
}

fn docs_for_entry_with_parse(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    def_file_id: FileId,
    parse: &Parse,
    entry: &DefEntry,
) -> Option<String> {
    let item = find_item_by_name_range(parse, entry.container(), entry.location().range())?;
    docs_for_item_with_inheritdoc(db, project_id, def_file_id, parse, item, entry.container())
}

fn def_kind_label(kind: DefKind) -> &'static str {
    match kind {
        DefKind::Contract => "contract",
        DefKind::Function => "function",
        DefKind::Struct => "struct",
        DefKind::Enum => "enum",
        DefKind::Event => "event",
        DefKind::Error => "error",
        DefKind::Modifier => "modifier",
        DefKind::Variable => "variable",
        DefKind::Udvt => "type",
    }
}

fn local_label(parse: &Parse, text: &str, local: &LocalDef) -> String {
    let label = match local.kind() {
        LocalDefKind::Parameter => find_param_definition(parse, local, false)
            .map(|param| format_param(parse, text, param))
            .unwrap_or_else(|| local.name().to_string()),
        LocalDefKind::NamedReturn => find_param_definition(parse, local, true)
            .map(|param| format_param(parse, text, param))
            .unwrap_or_else(|| local.name().to_string()),
        LocalDefKind::Local => find_local_definition(parse, local)
            .map(|param| format_param(parse, text, param))
            .unwrap_or_else(|| local.name().to_string()),
    };

    match local.kind() {
        LocalDefKind::Parameter => format!("parameter {label}"),
        LocalDefKind::NamedReturn => format!("return {label}"),
        LocalDefKind::Local => format!("local {label}"),
    }
}

fn find_param_definition<'a>(
    parse: &'a Parse,
    local: &LocalDef,
    in_returns: bool,
) -> Option<&'a VariableDefinition<'static>> {
    for item in parse.tree().items.iter() {
        let found = find_param_in_item(parse, item, local, in_returns);
        if found.is_some() {
            return found;
        }
    }
    None
}

fn find_param_in_item<'a>(
    parse: &'a Parse,
    item: &'a Item<'static>,
    local: &LocalDef,
    in_returns: bool,
) -> Option<&'a VariableDefinition<'static>> {
    match &item.kind {
        ItemKind::Contract(contract) => contract
            .body
            .iter()
            .find_map(|item| find_param_in_item(parse, item, local, in_returns)),
        ItemKind::Function(function) => {
            let params = if in_returns {
                function
                    .header
                    .returns
                    .as_ref()
                    .map(|returns| returns.vars.iter())
            } else {
                Some(function.header.parameters.vars.iter())
            };
            params
                .into_iter()
                .flatten()
                .find(|param| matches_local_def(parse, local, param))
        }
        _ => None,
    }
}

fn find_local_definition<'a>(
    parse: &'a Parse,
    local: &LocalDef,
) -> Option<&'a VariableDefinition<'static>> {
    for item in parse.tree().items.iter() {
        let found = find_local_in_item(parse, item, local);
        if found.is_some() {
            return found;
        }
    }
    None
}

fn find_local_in_item<'a>(
    parse: &'a Parse,
    item: &'a Item<'static>,
    local: &LocalDef,
) -> Option<&'a VariableDefinition<'static>> {
    match &item.kind {
        ItemKind::Contract(contract) => contract
            .body
            .iter()
            .find_map(|item| find_local_in_item(parse, item, local)),
        ItemKind::Function(function) => function
            .body
            .as_ref()
            .and_then(|body| find_local_in_block(parse, body, local)),
        _ => None,
    }
}

fn find_local_in_block<'a>(
    parse: &'a Parse,
    block: &'a sa_syntax::ast::Block<'static>,
    local: &LocalDef,
) -> Option<&'a VariableDefinition<'static>> {
    for stmt in block.stmts.iter() {
        if let Some(found) = find_local_in_stmt(parse, stmt, local) {
            return Some(found);
        }
    }
    None
}

fn find_local_in_stmt<'a>(
    parse: &'a Parse,
    stmt: &'a sa_syntax::ast::Stmt<'static>,
    local: &LocalDef,
) -> Option<&'a VariableDefinition<'static>> {
    match &stmt.kind {
        sa_syntax::ast::StmtKind::DeclSingle(var) => {
            matches_local_def(parse, local, var).then_some(var)
        }
        sa_syntax::ast::StmtKind::DeclMulti(vars, _) => vars.iter().find_map(|var| {
            if let sa_syntax::ast::interface::SpannedOption::Some(var) = var {
                matches_local_def(parse, local, var).then_some(var)
            } else {
                None
            }
        }),
        sa_syntax::ast::StmtKind::Block(block)
        | sa_syntax::ast::StmtKind::UncheckedBlock(block) => {
            find_local_in_block(parse, block, local)
        }
        sa_syntax::ast::StmtKind::For { init, body, .. } => {
            if let Some(init) = init.as_deref()
                && let Some(found) = find_local_in_stmt(parse, init, local)
            {
                return Some(found);
            }
            find_local_in_stmt(parse, body, local)
        }
        sa_syntax::ast::StmtKind::If(_, then_branch, else_branch) => {
            find_local_in_stmt(parse, then_branch, local).or_else(|| {
                else_branch
                    .as_deref()
                    .and_then(|stmt| find_local_in_stmt(parse, stmt, local))
            })
        }
        sa_syntax::ast::StmtKind::While(_, body) | sa_syntax::ast::StmtKind::DoWhile(body, _) => {
            find_local_in_stmt(parse, body, local)
        }
        sa_syntax::ast::StmtKind::Try(stmt_try) => stmt_try.clauses.iter().find_map(|clause| {
            clause
                .args
                .vars
                .iter()
                .find(|param| matches_local_def(parse, local, param))
                .or_else(|| find_local_in_block(parse, &clause.block, local))
        }),
        _ => None,
    }
}

fn matches_local_def(parse: &Parse, local: &LocalDef, var: &VariableDefinition<'_>) -> bool {
    let Some(name) = var.name else {
        return false;
    };
    let Some(range) = parse.span_to_text_range(name.span) else {
        return false;
    };
    range == local.range()
}
