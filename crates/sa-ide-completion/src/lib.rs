use std::collections::HashSet;

use sa_base_db::{FileId, ProjectId};
use sa_def::DefKind;
use sa_hir::{
    HirDatabase, contract_member_definitions_at_offset, local_scopes, lowered_program,
    visible_definitions,
};
use sa_paths::NormalizedPath;
use sa_sema::{SemaCompletionItem, SemaCompletionKind};
use sa_span::{TextRange, TextSize, is_ident_byte, range_contains};
use sa_syntax::parse_file;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionItemKind,
    pub replacement_range: TextRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionItemKind {
    Contract,
    Function,
    Struct,
    Enum,
    Event,
    Error,
    Modifier,
    Variable,
    Type,
    File,
}

pub fn completions(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    offset: TextSize,
) -> Vec<CompletionItem> {
    let text = db.file_input(file_id).text(db);
    let context = completion_context(text.as_ref(), offset);
    let parse_has_errors = matches!(context.kind, CompletionContextKind::Identifier)
        && !parse_file(text.as_ref()).errors().is_empty();

    let mut items = match &context.kind {
        CompletionContextKind::Identifier => {
            sema_identifier_items(db, project_id, file_id, offset, context.range)
                .unwrap_or_else(|| identifier_items(db, project_id, file_id, offset, context.range))
        }
        CompletionContextKind::Member {
            receiver,
            receiver_range,
        } => {
            if let Some(items) = sema_member_items(
                db,
                project_id,
                file_id,
                offset,
                receiver,
                *receiver_range,
                context.range,
            ) {
                items
            } else {
                let items = member_items(db, project_id, receiver, context.range);
                if items.is_empty() {
                    fallback_member_items(text.as_ref(), receiver, context.range)
                } else {
                    items
                }
            }
        }
        CompletionContextKind::Import => {
            import_items(db, project_id, &context.prefix, context.range)
        }
    };

    if parse_has_errors {
        items.extend(fallback_identifier_items(
            text.as_ref(),
            offset,
            context.range,
        ));
    }

    if !context.prefix.is_empty() {
        let prefix_lower = context.prefix.to_lowercase();
        items.retain(|item| item.label.to_lowercase().starts_with(&prefix_lower));
    }

    items.sort_by(|a, b| {
        (a.label.as_str(), completion_rank(a.kind))
            .cmp(&(b.label.as_str(), completion_rank(b.kind)))
    });
    items.dedup_by(|a, b| a.label == b.label && a.kind == b.kind);
    items
}

struct CompletionContext {
    kind: CompletionContextKind,
    prefix: String,
    range: TextRange,
}

enum CompletionContextKind {
    Identifier,
    Member {
        receiver: String,
        receiver_range: TextRange,
    },
    Import,
}

fn completion_context(text: &str, offset: TextSize) -> CompletionContext {
    if let Some(context) = import_context(text, offset) {
        return context;
    }

    if let Some(context) = member_context(text, offset) {
        return context;
    }

    let (prefix, range) = identifier_prefix(text, offset, 0);
    CompletionContext {
        kind: CompletionContextKind::Identifier,
        prefix,
        range,
    }
}

fn identifier_items(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    offset: TextSize,
    range: TextRange,
) -> Vec<CompletionItem> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();

    for def in visible_definitions(db, project_id, file_id) {
        push_completion_item(
            def.name(),
            completion_kind(def.kind()),
            range,
            &mut items,
            &mut seen,
        );
    }

    for def in contract_member_definitions_at_offset(db, project_id, file_id, offset) {
        push_completion_item(
            def.name(),
            completion_kind(def.kind()),
            range,
            &mut items,
            &mut seen,
        );
    }

    let locals = local_scopes(db, file_id);
    for local in locals.defs() {
        if local_def_in_scope(local, offset) {
            push_completion_item(
                local.name(),
                CompletionItemKind::Variable,
                range,
                &mut items,
                &mut seen,
            );
        }
    }

    items
}

fn fallback_identifier_items(
    text: &str,
    offset: TextSize,
    range: TextRange,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut known_types = HashSet::new();

    collect_fallback_imports(text, &mut known_types, &mut items, &mut seen, range);
    collect_fallback_type_defs(text, &mut known_types, &mut items, &mut seen, range);
    collect_fallback_locals(text, offset, &known_types, &mut items, &mut seen, range);

    items
}

fn collect_fallback_imports(
    text: &str,
    known_types: &mut HashSet<String>,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<(String, CompletionItemKind)>,
    range: TextRange,
) {
    let mut lexer = FallbackLexer::new(text);
    let mut in_import = false;
    let mut in_braces = false;
    let mut pending_name: Option<String> = None;
    let mut expect_alias = false;
    let mut saw_star = false;

    while let Some(token) = lexer.next_token() {
        match token {
            FallbackToken::Ident(ident) => {
                if !in_import {
                    if ident == "import" {
                        in_import = true;
                        in_braces = false;
                        pending_name = None;
                        expect_alias = false;
                        saw_star = false;
                    }
                    continue;
                }

                if ident == "as" {
                    expect_alias = true;
                    continue;
                }
                if ident == "from" {
                    continue;
                }

                if in_braces {
                    if expect_alias {
                        push_fallback_type(&ident, known_types, items, seen, range);
                        pending_name = None;
                        expect_alias = false;
                    } else {
                        if let Some(name) = pending_name.take() {
                            push_fallback_type(&name, known_types, items, seen, range);
                        }
                        pending_name = Some(ident);
                    }
                } else if expect_alias {
                    push_fallback_type(&ident, known_types, items, seen, range);
                    expect_alias = false;
                } else if saw_star {
                    continue;
                }
            }
            FallbackToken::Punct(punct) => match punct {
                '{' => {
                    if in_import {
                        in_braces = true;
                        pending_name = None;
                        expect_alias = false;
                    }
                }
                '}' => {
                    if in_braces {
                        if let Some(name) = pending_name.take() {
                            push_fallback_type(&name, known_types, items, seen, range);
                        }
                        in_braces = false;
                    }
                }
                ',' => {
                    if in_braces {
                        if let Some(name) = pending_name.take() {
                            push_fallback_type(&name, known_types, items, seen, range);
                        }
                        expect_alias = false;
                    }
                }
                '*' => {
                    if in_import {
                        saw_star = true;
                    }
                }
                ';' => {
                    if in_import {
                        if let Some(name) = pending_name.take() {
                            push_fallback_type(&name, known_types, items, seen, range);
                        }
                        in_import = false;
                        in_braces = false;
                        expect_alias = false;
                        saw_star = false;
                    }
                }
                _ => {}
            },
        }
    }
}

fn collect_fallback_type_defs(
    text: &str,
    known_types: &mut HashSet<String>,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<(String, CompletionItemKind)>,
    range: TextRange,
) {
    let mut lexer = FallbackLexer::new(text);
    let mut brace_depth = 0usize;
    let mut pending_kind: Option<CompletionItemKind> = None;

    while let Some(token) = lexer.next_token() {
        match token {
            FallbackToken::Ident(ident) => {
                if let Some(kind) = pending_kind.take() {
                    if is_type_def_kind(kind) {
                        known_types.insert(ident.clone());
                    }
                    push_completion_item(&ident, kind, range, items, seen);
                    continue;
                }

                if brace_depth <= 1
                    && let Some(kind) = type_def_keyword_kind(&ident)
                {
                    pending_kind = Some(kind);
                }
            }
            FallbackToken::Punct(punct) => match punct {
                '{' => brace_depth += 1,
                '}' => brace_depth = brace_depth.saturating_sub(1),
                _ => {}
            },
        }
    }
}

fn collect_fallback_locals(
    text: &str,
    offset: TextSize,
    known_types: &HashSet<String>,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<(String, CompletionItemKind)>,
    range: TextRange,
) {
    let limit = usize::from(offset).min(text.len());
    let prefix = &text[..limit];
    let mut lexer = FallbackLexer::new(prefix);

    let mut brace_depth = 0usize;
    let mut pending_function_body = false;
    let mut function_body_depth: Option<usize> = None;

    let mut params_parsed = false;
    let mut parsing_params = false;
    let mut param_depth = 0usize;
    let mut param_tokens: Vec<String> = Vec::new();
    let mut pending_params: Vec<String> = Vec::new();
    let mut current_params: Vec<String> = Vec::new();
    let mut current_locals: Vec<String> = Vec::new();

    let mut statement_idents: Vec<String> = Vec::new();
    let mut statement_type_start = false;
    let mut statement_paren_early = false;

    while let Some(token) = lexer.next_token() {
        match token {
            FallbackToken::Ident(ident) => {
                if brace_depth <= 1 && is_function_keyword(&ident) {
                    pending_function_body = true;
                    params_parsed = false;
                    parsing_params = false;
                    param_depth = 0;
                    param_tokens.clear();
                    pending_params.clear();
                    continue;
                }

                if parsing_params && param_depth == 1 {
                    param_tokens.push(ident);
                    continue;
                }

                if in_function_body(function_body_depth, brace_depth) {
                    if statement_idents.is_empty() {
                        statement_type_start = is_type_like(&ident, known_types);
                        statement_paren_early = false;
                    }
                    statement_idents.push(ident);
                }
            }
            FallbackToken::Punct(punct) => match punct {
                '(' => {
                    if pending_function_body && !params_parsed && !parsing_params {
                        parsing_params = true;
                        param_depth = 1;
                        param_tokens.clear();
                        continue;
                    }
                    if parsing_params {
                        param_depth += 1;
                    }
                    if in_function_body(function_body_depth, brace_depth)
                        && statement_type_start
                        && statement_idents.len() == 1
                    {
                        statement_paren_early = true;
                    }
                }
                ')' => {
                    if parsing_params {
                        param_depth = param_depth.saturating_sub(1);
                        if param_depth == 0 {
                            push_param_tokens(&mut param_tokens, &mut pending_params, known_types);
                            parsing_params = false;
                            params_parsed = true;
                        }
                    }
                }
                ',' => {
                    if parsing_params && param_depth == 1 {
                        push_param_tokens(&mut param_tokens, &mut pending_params, known_types);
                    }
                }
                '{' => {
                    brace_depth += 1;
                    if pending_function_body {
                        function_body_depth = Some(brace_depth);
                        pending_function_body = false;
                        current_locals.clear();
                        current_params = std::mem::take(&mut pending_params);
                        statement_idents.clear();
                        statement_type_start = false;
                        statement_paren_early = false;
                    }
                }
                '}' => {
                    if function_body_depth == Some(brace_depth) {
                        function_body_depth = None;
                        current_locals.clear();
                        current_params.clear();
                    }
                    brace_depth = brace_depth.saturating_sub(1);
                    statement_idents.clear();
                    statement_type_start = false;
                    statement_paren_early = false;
                }
                ';' => {
                    if pending_function_body {
                        pending_function_body = false;
                        params_parsed = false;
                        parsing_params = false;
                        param_depth = 0;
                        param_tokens.clear();
                        pending_params.clear();
                    }
                    if in_function_body(function_body_depth, brace_depth)
                        && let Some(name) = statement_var_name(
                            &statement_idents,
                            statement_type_start,
                            statement_paren_early,
                            known_types,
                        )
                    {
                        current_locals.push(name);
                    }
                    statement_idents.clear();
                    statement_type_start = false;
                    statement_paren_early = false;
                }
                _ => {}
            },
        }
    }

    if in_function_body(function_body_depth, brace_depth) {
        for name in current_params.into_iter().chain(current_locals) {
            push_completion_item(&name, CompletionItemKind::Variable, range, items, seen);
        }
    }
}

fn in_function_body(function_body_depth: Option<usize>, brace_depth: usize) -> bool {
    function_body_depth.is_some_and(|depth| brace_depth >= depth)
}

fn statement_var_name(
    idents: &[String],
    type_start: bool,
    paren_early: bool,
    known_types: &HashSet<String>,
) -> Option<String> {
    let first = idents.first()?;
    if !type_start || !is_type_like(first, known_types) {
        return None;
    }
    if paren_early && !type_allows_paren(first) {
        return None;
    }
    let name = idents.iter().rev().find(|ident| !is_decl_modifier(ident))?;
    if name == first {
        return None;
    }
    Some(name.clone())
}

fn push_param_tokens(
    tokens: &mut Vec<String>,
    pending_params: &mut Vec<String>,
    known_types: &HashSet<String>,
) {
    if tokens.is_empty() {
        return;
    }
    let first = &tokens[0];
    if !is_type_like(first, known_types) {
        tokens.clear();
        return;
    }
    let name = tokens.iter().rev().find(|ident| !is_decl_modifier(ident));
    if let Some(name) = name
        && name != first
    {
        pending_params.push(name.clone());
    }
    tokens.clear();
}

fn push_fallback_type(
    name: &str,
    known_types: &mut HashSet<String>,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<(String, CompletionItemKind)>,
    range: TextRange,
) {
    known_types.insert(name.to_string());
    push_completion_item(name, CompletionItemKind::Type, range, items, seen);
}

fn type_def_keyword_kind(ident: &str) -> Option<CompletionItemKind> {
    match ident {
        "contract" | "library" | "interface" => Some(CompletionItemKind::Contract),
        "struct" => Some(CompletionItemKind::Struct),
        "enum" => Some(CompletionItemKind::Enum),
        "type" => Some(CompletionItemKind::Type),
        _ => None,
    }
}

fn is_type_def_kind(kind: CompletionItemKind) -> bool {
    matches!(
        kind,
        CompletionItemKind::Contract
            | CompletionItemKind::Struct
            | CompletionItemKind::Enum
            | CompletionItemKind::Type
    )
}

fn is_function_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "function" | "constructor" | "fallback" | "receive" | "modifier"
    )
}

fn type_allows_paren(ident: &str) -> bool {
    matches!(ident, "mapping" | "function")
}

fn is_type_like(ident: &str, known_types: &HashSet<String>) -> bool {
    known_types.contains(ident) || is_builtin_type(ident)
}

fn is_builtin_type(ident: &str) -> bool {
    matches!(
        ident,
        "address" | "bool" | "string" | "byte" | "bytes" | "mapping" | "function"
    ) || is_sized_type(ident, "uint", 8, 256, 8)
        || is_sized_type(ident, "int", 8, 256, 8)
        || is_sized_type(ident, "bytes", 1, 32, 1)
}

fn is_sized_type(ident: &str, prefix: &str, min: u16, max: u16, step: u16) -> bool {
    let Some(rest) = ident.strip_prefix(prefix) else {
        return false;
    };
    if rest.is_empty() {
        return true;
    }
    let Ok(size) = rest.parse::<u16>() else {
        return false;
    };
    if size < min || size > max {
        return false;
    }
    size % step == 0
}

fn is_decl_modifier(ident: &str) -> bool {
    matches!(
        ident,
        "memory"
            | "calldata"
            | "storage"
            | "indexed"
            | "payable"
            | "public"
            | "private"
            | "internal"
            | "external"
            | "constant"
            | "immutable"
    )
}

fn push_completion_item(
    label: &str,
    kind: CompletionItemKind,
    range: TextRange,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<(String, CompletionItemKind)>,
) {
    if seen.insert((label.to_string(), kind)) {
        items.push(CompletionItem {
            label: label.to_string(),
            kind,
            replacement_range: range,
        });
    }
}

fn local_def_in_scope(local: &sa_hir::LocalDef, offset: TextSize) -> bool {
    local.range().start() <= offset
        && (range_contains(local.scope(), offset) || range_contains(local.range(), offset))
}

fn sema_identifier_items(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    offset: TextSize,
    range: TextRange,
) -> Option<Vec<CompletionItem>> {
    let project = db.project_input(project_id);
    let snapshot = sa_sema::sema_snapshot_for_project(db, project);
    let snapshot = snapshot.for_file(file_id)?;
    let items = snapshot.identifier_completions(file_id, offset)?;
    Some(
        items
            .into_iter()
            .map(|item| completion_from_sema(item, range))
            .collect(),
    )
}

fn member_items(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    receiver: &str,
    range: TextRange,
) -> Vec<CompletionItem> {
    let program = lowered_program(db, project_id);
    let mut items = Vec::new();

    for entry in program.def_map().entries() {
        if entry.container() != Some(receiver) {
            continue;
        }
        let label = entry.location().name().to_string();
        items.push(CompletionItem {
            label,
            kind: completion_kind(entry.kind()),
            replacement_range: range,
        });
    }

    items
}

fn sema_member_items(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    offset: TextSize,
    receiver: &str,
    receiver_range: TextRange,
    range: TextRange,
) -> Option<Vec<CompletionItem>> {
    let project = db.project_input(project_id);
    let snapshot = sa_sema::sema_snapshot_for_project(db, project);
    let snapshot = snapshot.for_file(file_id)?;
    let items = snapshot.member_completions(file_id, offset, receiver_range, receiver)?;
    Some(
        items
            .into_iter()
            .map(|item| completion_from_sema(item, range))
            .collect(),
    )
}

fn fallback_member_items(text: &str, receiver: &str, range: TextRange) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    let mut lexer = FallbackLexer::new(text);

    let mut awaiting_contract_name = false;
    let mut awaiting_contract_brace = false;
    let mut in_target = false;
    let mut brace_depth = 0usize;

    let mut pending_named_kind: Option<CompletionItemKind> = None;
    let mut statement_idents: Vec<String> = Vec::new();
    let mut statement_has_decl_keyword = false;
    let mut statement_skip_variable = false;

    while let Some(token) = lexer.next_token() {
        match token {
            FallbackToken::Ident(ident) => {
                if !in_target {
                    if awaiting_contract_name {
                        if ident == receiver {
                            awaiting_contract_brace = true;
                        }
                        awaiting_contract_name = false;
                    } else if matches!(ident.as_str(), "contract" | "interface" | "library") {
                        awaiting_contract_name = true;
                    }
                    continue;
                }

                if let Some(kind) = pending_named_kind.take() {
                    items.push(CompletionItem {
                        label: ident,
                        kind,
                        replacement_range: range,
                    });
                    continue;
                }

                if brace_depth == 1 {
                    match ident.as_str() {
                        "function" => {
                            pending_named_kind = Some(CompletionItemKind::Function);
                            statement_has_decl_keyword = true;
                        }
                        "event" => {
                            pending_named_kind = Some(CompletionItemKind::Event);
                            statement_has_decl_keyword = true;
                        }
                        "error" => {
                            pending_named_kind = Some(CompletionItemKind::Error);
                            statement_has_decl_keyword = true;
                        }
                        "modifier" => {
                            pending_named_kind = Some(CompletionItemKind::Modifier);
                            statement_has_decl_keyword = true;
                        }
                        "struct" => {
                            pending_named_kind = Some(CompletionItemKind::Struct);
                            statement_has_decl_keyword = true;
                        }
                        "enum" => {
                            pending_named_kind = Some(CompletionItemKind::Enum);
                            statement_has_decl_keyword = true;
                        }
                        "type" => {
                            pending_named_kind = Some(CompletionItemKind::Type);
                            statement_has_decl_keyword = true;
                        }
                        "using" | "pragma" | "import" => {
                            statement_skip_variable = true;
                        }
                        _ => {
                            statement_idents.push(ident);
                        }
                    }
                }
            }
            FallbackToken::Punct(punct) => match punct {
                '{' => {
                    if awaiting_contract_brace {
                        in_target = true;
                        brace_depth = 1;
                        awaiting_contract_brace = false;
                        statement_idents.clear();
                        statement_has_decl_keyword = false;
                        statement_skip_variable = false;
                        pending_named_kind = None;
                        continue;
                    }
                    if in_target {
                        if brace_depth == 1 {
                            statement_idents.clear();
                            statement_has_decl_keyword = false;
                            statement_skip_variable = false;
                            pending_named_kind = None;
                        }
                        brace_depth += 1;
                    }
                }
                '}' => {
                    if in_target {
                        brace_depth = brace_depth.saturating_sub(1);
                        if brace_depth == 0 {
                            in_target = false;
                        }
                    }
                }
                ';' => {
                    if in_target && brace_depth == 1 {
                        if !statement_has_decl_keyword
                            && !statement_skip_variable
                            && let Some(name) = statement_idents.last()
                        {
                            items.push(CompletionItem {
                                label: name.clone(),
                                kind: CompletionItemKind::Variable,
                                replacement_range: range,
                            });
                        }
                        statement_idents.clear();
                        statement_has_decl_keyword = false;
                        statement_skip_variable = false;
                    }
                }
                _ => {}
            },
        }
    }

    items
}

fn completion_from_sema(item: SemaCompletionItem, range: TextRange) -> CompletionItem {
    CompletionItem {
        label: item.label,
        kind: completion_kind_from_sema(item.kind),
        replacement_range: range,
    }
}

fn completion_kind_from_sema(kind: SemaCompletionKind) -> CompletionItemKind {
    match kind {
        SemaCompletionKind::Contract => CompletionItemKind::Contract,
        SemaCompletionKind::Function => CompletionItemKind::Function,
        SemaCompletionKind::Struct => CompletionItemKind::Struct,
        SemaCompletionKind::Enum => CompletionItemKind::Enum,
        SemaCompletionKind::Event => CompletionItemKind::Event,
        SemaCompletionKind::Error => CompletionItemKind::Error,
        SemaCompletionKind::Modifier => CompletionItemKind::Modifier,
        SemaCompletionKind::Variable => CompletionItemKind::Variable,
        SemaCompletionKind::Type => CompletionItemKind::Type,
    }
}

fn import_items(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    prefix: &str,
    range: TextRange,
) -> Vec<CompletionItem> {
    let workspace = db.project_input(project_id).workspace(db);
    let root = workspace.root();
    let root_str = root.as_str().trim_end_matches('/');

    let mut items = Vec::new();
    for file_id in db.file_ids() {
        let path = db.file_path(file_id);
        let path_str = path.as_str();
        if !path_str.ends_with(".sol") {
            continue;
        }
        let rel = make_relative(root_str, &path);
        if !rel.starts_with(prefix) {
            continue;
        }
        items.push(CompletionItem {
            label: rel,
            kind: CompletionItemKind::File,
            replacement_range: range,
        });
    }

    items
}

fn make_relative(root: &str, path: &NormalizedPath) -> String {
    let full = path.as_str();
    if let Some(stripped) = full.strip_prefix(root) {
        stripped.trim_start_matches('/').to_string()
    } else {
        full.to_string()
    }
}

fn completion_kind(kind: DefKind) -> CompletionItemKind {
    match kind {
        DefKind::Contract => CompletionItemKind::Contract,
        DefKind::Function => CompletionItemKind::Function,
        DefKind::Struct => CompletionItemKind::Struct,
        DefKind::Enum => CompletionItemKind::Enum,
        DefKind::Event => CompletionItemKind::Event,
        DefKind::Error => CompletionItemKind::Error,
        DefKind::Modifier => CompletionItemKind::Modifier,
        DefKind::Variable => CompletionItemKind::Variable,
        DefKind::Udvt => CompletionItemKind::Type,
    }
}

fn completion_rank(kind: CompletionItemKind) -> u8 {
    match kind {
        CompletionItemKind::Contract => 0,
        CompletionItemKind::Struct => 1,
        CompletionItemKind::Enum => 2,
        CompletionItemKind::Function => 3,
        CompletionItemKind::Variable => 4,
        CompletionItemKind::Event => 5,
        CompletionItemKind::Error => 6,
        CompletionItemKind::Modifier => 7,
        CompletionItemKind::Type => 8,
        CompletionItemKind::File => 9,
    }
}

fn import_context(text: &str, offset: TextSize) -> Option<CompletionContext> {
    let idx = usize::from(offset).min(text.len());
    let line_start = text[..idx].rfind('\n').map(|pos| pos + 1).unwrap_or(0);
    let line = &text[line_start..idx];
    let (quote_pos, _quote_char) = line
        .rfind('"')
        .map(|pos| (pos, '"'))
        .or_else(|| line.rfind('\'').map(|pos| (pos, '\'')))?;

    let before = &line[..quote_pos];
    if !before.trim_start().starts_with("import") {
        return None;
    }

    let prefix = line[quote_pos + 1..].to_string();
    let start = line_start + quote_pos + 1;
    Some(CompletionContext {
        kind: CompletionContextKind::Import,
        prefix,
        range: TextRange::new(TextSize::from(start as u32), TextSize::from(idx as u32)),
    })
}

fn member_context(text: &str, offset: TextSize) -> Option<CompletionContext> {
    let bytes = text.as_bytes();
    let mut idx = usize::from(offset).min(bytes.len());
    while idx > 0 && bytes[idx - 1].is_ascii_whitespace() {
        idx -= 1;
    }
    if idx == 0 {
        return None;
    }
    let mut prefix_start = idx;
    while prefix_start > 0 && is_ident_byte(bytes[prefix_start - 1]) {
        prefix_start -= 1;
    }
    if prefix_start == 0 || bytes[prefix_start - 1] != b'.' {
        return None;
    }

    let dot = prefix_start - 1;
    let (receiver_start, receiver_end) = ident_before(bytes, dot)?;
    let receiver = text
        .get(receiver_start..receiver_end)
        .unwrap_or_default()
        .to_string();
    let receiver_range = TextRange::new(
        TextSize::from(receiver_start as u32),
        TextSize::from(receiver_end as u32),
    );

    let (prefix, range) = identifier_prefix(text, offset, dot + 1);
    Some(CompletionContext {
        kind: CompletionContextKind::Member {
            receiver,
            receiver_range,
        },
        prefix,
        range,
    })
}

fn identifier_prefix(text: &str, offset: TextSize, min_start: usize) -> (String, TextRange) {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let idx = usize::from(offset).min(len);

    let mut start = idx;
    while start > min_start && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = idx;
    while end < len && is_ident_byte(bytes[end]) {
        end += 1;
    }

    let prefix = text.get(start..idx).unwrap_or_default().to_string();
    let range = TextRange::new(TextSize::from(start as u32), TextSize::from(end as u32));
    (prefix, range)
}

fn ident_before(bytes: &[u8], mut idx: usize) -> Option<(usize, usize)> {
    while idx > 0 && bytes[idx - 1].is_ascii_whitespace() {
        idx -= 1;
    }
    let end = idx;
    let mut start = idx;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    if start == end {
        None
    } else {
        Some((start, end))
    }
}

struct FallbackLexer<'a> {
    bytes: &'a [u8],
    idx: usize,
}

enum FallbackToken {
    Ident(String),
    Punct(char),
}

impl<'a> FallbackLexer<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            idx: 0,
        }
    }

    fn next_token(&mut self) -> Option<FallbackToken> {
        loop {
            self.skip_ws_and_comments();
            if self.idx >= self.bytes.len() {
                return None;
            }

            let byte = self.bytes[self.idx];
            if is_ident_start(byte) {
                let start = self.idx;
                self.idx += 1;
                while self.idx < self.bytes.len() && is_ident_byte(self.bytes[self.idx]) {
                    self.idx += 1;
                }
                let ident = std::str::from_utf8(&self.bytes[start..self.idx])
                    .unwrap_or_default()
                    .to_string();
                return Some(FallbackToken::Ident(ident));
            }

            if matches!(byte, b'{' | b'}' | b';' | b',' | b'(' | b')' | b'*') {
                self.idx += 1;
                return Some(FallbackToken::Punct(byte as char));
            }

            // Skip unrecognized byte and continue the loop
            self.idx += 1;
        }
    }

    fn skip_ws_and_comments(&mut self) {
        while self.idx < self.bytes.len() {
            let byte = self.bytes[self.idx];
            if byte.is_ascii_whitespace() {
                self.idx += 1;
                continue;
            }
            if byte == b'/' {
                if self.peek_byte(1) == Some(b'/') {
                    self.idx += 2;
                    while self.idx < self.bytes.len() && self.bytes[self.idx] != b'\n' {
                        self.idx += 1;
                    }
                    continue;
                }
                if self.peek_byte(1) == Some(b'*') {
                    self.idx += 2;
                    let mut found_terminator = false;
                    while self.idx + 1 < self.bytes.len() {
                        if self.bytes[self.idx] == b'*' && self.bytes[self.idx + 1] == b'/' {
                            self.idx += 2;
                            found_terminator = true;
                            break;
                        }
                        self.idx += 1;
                    }
                    // If no closing `*/` was found, set idx to EOF
                    if !found_terminator {
                        self.idx = self.bytes.len();
                    }
                    continue;
                }
            }
            if byte == b'\'' || byte == b'"' {
                self.skip_string(byte);
                continue;
            }
            break;
        }
    }

    fn skip_string(&mut self, quote: u8) {
        self.idx += 1;
        while self.idx < self.bytes.len() {
            let byte = self.bytes[self.idx];
            if byte == b'\\' {
                self.idx = (self.idx + 2).min(self.bytes.len());
                continue;
            }
            self.idx += 1;
            if byte == quote {
                break;
            }
        }
    }

    fn peek_byte(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.idx + offset).copied()
    }
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use sa_test_support::extract_offset;

    fn labels(items: &[CompletionItem]) -> HashSet<&str> {
        items.iter().map(|item| item.label.as_str()).collect()
    }

    #[test]
    fn import_context_extracts_prefix_and_range() {
        let (text, offset) = extract_offset("import \"lib/To/*caret*/ken.sol\";\n");
        let context = import_context(&text, offset).expect("import context");

        assert!(matches!(context.kind, CompletionContextKind::Import));
        assert_eq!(context.prefix, "lib/To");

        let start = text.find('"').expect("quote") + 1;
        assert_eq!(context.range.start(), TextSize::from(start as u32));
        assert_eq!(context.range.end(), offset);
    }

    #[test]
    fn member_context_detects_receiver_and_prefix() {
        let (text, offset) = extract_offset("foo.bar/*caret*/Baz");
        let context = member_context(&text, offset).expect("member context");

        let CompletionContextKind::Member {
            receiver,
            receiver_range,
        } = context.kind
        else {
            panic!("expected member context");
        };
        assert_eq!(receiver, "foo");
        assert_eq!(
            receiver_range,
            TextRange::new(TextSize::from(0), TextSize::from(3))
        );

        assert_eq!(context.prefix, "bar");
        let ident_start = text.find("barBaz").expect("barBaz");
        let ident_end = ident_start + "barBaz".len();
        assert_eq!(
            context.range,
            TextRange::new(
                TextSize::from(ident_start as u32),
                TextSize::from(ident_end as u32)
            )
        );
    }

    #[test]
    fn identifier_prefix_respects_min_start() {
        let (text, offset) = extract_offset("foo.bar/*caret*/Baz");
        let dot = text.find('.').expect("dot");
        let (prefix, range) = identifier_prefix(&text, offset, dot + 1);

        assert_eq!(prefix, "bar");
        let ident_start = text.find("barBaz").expect("barBaz");
        let ident_end = ident_start + "barBaz".len();
        assert_eq!(
            range,
            TextRange::new(
                TextSize::from(ident_start as u32),
                TextSize::from(ident_end as u32)
            )
        );
    }

    #[test]
    fn fallback_identifier_items_collect_imports_types_and_locals() {
        let (text, offset) = extract_offset(
            r#"
import {Foo, Bar as Baz} from "./Lib.sol";
import * as Glob from "./Lib.sol";
import "./Other.sol";

contract Sample {
    struct Inner { uint256 x; }
    enum Choice { A, B }
    type Alias is uint256;

    function doThing(uint256 param, address owner) public {
        uint256 local;
        mapping(address => uint256) balances;
        /*caret*/
    }
}
"#,
        );

        let items = fallback_identifier_items(&text, offset, TextRange::new(offset, offset));
        let labels = labels(&items);

        assert!(labels.contains("Foo"));
        assert!(labels.contains("Baz"));
        assert!(labels.contains("Glob"));
        assert!(labels.contains("Sample"));
        assert!(labels.contains("Inner"));
        assert!(labels.contains("Choice"));
        assert!(labels.contains("Alias"));
        assert!(labels.contains("param"));
        assert!(labels.contains("owner"));
        assert!(labels.contains("local"));
        assert!(labels.contains("balances"));
    }

    #[test]
    fn fallback_member_items_collects_contract_members() {
        let text = r#"
pragma solidity ^0.8.20;

contract Foo {
    uint256 value;
    function bar() public {}
    event Evt();
    error Boom();
    modifier onlyOwner() { _; }
    struct Data { uint256 x; }
    enum Choice { A, B }
    type Alias is uint256;
    using Lib for uint256;
}
"#;
        let items = fallback_member_items(
            text,
            "Foo",
            TextRange::new(TextSize::from(0), TextSize::from(0)),
        );
        let labels = labels(&items);

        assert!(labels.contains("value"));
        assert!(labels.contains("bar"));
        assert!(labels.contains("Evt"));
        assert!(labels.contains("Boom"));
        assert!(labels.contains("onlyOwner"));
        assert!(labels.contains("Data"));
        assert!(labels.contains("Choice"));
        assert!(labels.contains("Alias"));
        assert!(!labels.contains("Lib"));
    }

    #[test]
    fn sized_and_builtin_type_helpers_work() {
        assert!(is_sized_type("uint256", "uint", 8, 256, 8));
        assert!(!is_sized_type("uint7", "uint", 8, 256, 8));
        assert!(is_builtin_type("uint256"));
        assert!(is_builtin_type("bytes32"));
        assert!(!is_builtin_type("customtype"));
    }

    #[test]
    fn identifier_helpers_accept_valid_names() {
        assert!(is_ident_start(b'_'));
        assert!(is_ident_start(b'A'));
        assert!(is_ident_start(b'$'));
        assert!(!is_ident_start(b'1'));
    }
}
