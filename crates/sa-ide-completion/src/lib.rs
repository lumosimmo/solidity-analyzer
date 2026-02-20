use std::collections::{HashMap, HashSet};

use sa_base_db::{FileId, ProjectId};
use sa_def::DefKind;
use sa_hir::{
    HirDatabase, contract_member_definitions_at_offset, local_scopes, lowered_program,
    visible_definitions,
};
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryResolver, resolve_import_path_with_resolver};
use sa_sema::{SemaCompletionItem, SemaCompletionKind};
use sa_span::{TextRange, TextSize, is_ident_byte, range_contains};
use sa_syntax::ast::{
    ContractKind, DataLocation, ElementaryType, Item, ItemKind, Stmt, StmtKind, TypeKind,
    VariableDefinition, Visibility, interface::SpannedOption,
};
use sa_syntax::{Parse, parse_file};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionItemKind,
    pub replacement_range: TextRange,
    pub detail: Option<String>,
    pub origin: Option<String>,
    pub insert_text: Option<String>,
    pub insert_text_format: CompletionInsertTextFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionInsertTextFormat {
    Plain,
    Snippet,
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

    let mut restricted_handled = false;
    let mut items = match &context.kind {
        CompletionContextKind::Identifier => {
            if let Some(items) = restricted_identifier_items(
                db,
                project_id,
                file_id,
                text.as_ref(),
                offset,
                context.range,
            ) {
                restricted_handled = true;
                items
            } else {
                sema_identifier_items(db, project_id, file_id, offset, context.range)
                    .unwrap_or_else(|| {
                        identifier_items(db, project_id, file_id, offset, context.range)
                    })
            }
        }
        CompletionContextKind::Member {
            receiver,
            receiver_range,
        } => {
            let parse_has_errors = !parse_file(text.as_ref()).errors().is_empty();
            if parse_has_errors {
                if let Some(items) = member_items_from_local_decl(
                    db,
                    project_id,
                    file_id,
                    offset,
                    receiver,
                    context.range,
                ) {
                    items
                } else {
                    let contract_items = member_items_for_named_contract(
                        db,
                        project_id,
                        file_id,
                        receiver,
                        context.range,
                    );
                    let sema_items = sema_member_items(
                        db,
                        project_id,
                        file_id,
                        offset,
                        receiver,
                        *receiver_range,
                        context.range,
                    );
                    if !contract_items.is_empty() {
                        if let Some(mut sema_items) = sema_items
                            && !sema_items.is_empty()
                        {
                            let mut merged = contract_items;
                            merged.append(&mut sema_items);
                            merged
                        } else {
                            contract_items
                        }
                    } else {
                        let fallback_items =
                            fallback_member_items(text.as_ref(), receiver, context.range);
                        if let Some(mut sema_items) = sema_items
                            && !sema_items.is_empty()
                        {
                            if fallback_items.is_empty() {
                                sema_items
                            } else {
                                let mut merged = fallback_items;
                                merged.append(&mut sema_items);
                                merged
                            }
                        } else {
                            fallback_items
                        }
                    }
                }
            } else {
                let sema_items = sema_member_items(
                    db,
                    project_id,
                    file_id,
                    offset,
                    receiver,
                    *receiver_range,
                    context.range,
                );
                if let Some(items) = sema_items
                    && !items.is_empty()
                {
                    items
                } else {
                    member_items_for_named_contract(
                        db,
                        project_id,
                        file_id,
                        receiver,
                        context.range,
                    )
                }
            }
        }
        CompletionContextKind::Import => {
            import_items(db, project_id, &context.prefix, context.range)
        }
    };

    if parse_has_errors && !restricted_handled {
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
    let mut deduped: Vec<CompletionItem> = Vec::with_capacity(items.len());
    for item in items {
        if let Some(last) = deduped.last_mut()
            && last.label == item.label
            && last.kind == item.kind
        {
            if last.detail.is_none() && item.detail.is_some() {
                *last = item;
            }
            continue;
        }
        deduped.push(item);
    }
    deduped
}

fn restricted_identifier_items(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    text: &str,
    offset: TextSize,
    range: TextRange,
) -> Option<Vec<CompletionItem>> {
    if let Some(fields) = struct_literal_field_items(text, offset) {
        return Some(completion_items_from_names(
            fields,
            CompletionItemKind::Variable,
            range,
        ));
    }
    if let Some(options) = call_options_items(text, offset) {
        return Some(completion_items_from_names(
            options,
            CompletionItemKind::Variable,
            range,
        ));
    }
    if let Some(names) = named_args_items(text, offset) {
        return Some(completion_items_from_names(
            names,
            CompletionItemKind::Variable,
            range,
        ));
    }
    if let Some(bases) = override_list_items(text, offset) {
        return Some(completion_items_from_names(
            bases,
            CompletionItemKind::Contract,
            range,
        ));
    }
    if let Some(types) = returns_list_items(text, offset) {
        return Some(completion_items_from_names(
            types,
            CompletionItemKind::Type,
            range,
        ));
    }
    if using_brace_context(text, offset) {
        return Some(Vec::new());
    }

    let _ = (db, project_id, file_id);
    None
}

fn completion_items_from_names(
    names: Vec<String>,
    kind: CompletionItemKind,
    range: TextRange,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for name in names {
        push_completion_item(&name, kind, range, &mut items, &mut seen);
    }
    items
}

fn struct_literal_field_items(text: &str, offset: TextSize) -> Option<Vec<String>> {
    let mut struct_fields = HashMap::new();
    collect_fallback_struct_fields(text, &mut struct_fields);
    struct_literal_fields_at_offset(text, offset, &struct_fields)
}

fn call_options_items(text: &str, offset: TextSize) -> Option<Vec<String>> {
    let open_brace = open_brace_at_offset(text, offset)?;
    if brace_preceded_by_open_paren(text, open_brace) {
        return None;
    }
    let close_brace = matching_close_brace(text, open_brace)?;
    let after = next_non_ws_byte(text, close_brace + 1)?;
    if after != b'(' {
        return None;
    }
    let mut options = vec!["gas".to_string(), "value".to_string()];
    if call_options_has_new_keyword(text, open_brace) {
        options.push("salt".to_string());
    }
    let used = named_fields_before_offset(text, offset, open_brace);
    options.retain(|opt| !used.contains(opt));
    Some(options)
}

fn named_args_items(text: &str, offset: TextSize) -> Option<Vec<String>> {
    let open_brace = open_brace_at_offset(text, offset)?;
    if !brace_preceded_by_open_paren(text, open_brace) {
        return None;
    }
    if let Some((struct_name, _)) = struct_literal_name_at_offset(text, offset) {
        let mut struct_fields = HashMap::new();
        collect_fallback_struct_fields(text, &mut struct_fields);
        if struct_fields.contains_key(&struct_name) {
            return None;
        }
    }

    let names = named_arg_candidates(text, offset, open_brace).unwrap_or_default();
    let used = named_fields_before_offset(text, offset, open_brace);
    let remaining = names
        .into_iter()
        .filter(|name| !used.contains(name))
        .collect::<Vec<_>>();
    Some(remaining)
}

fn override_list_items(text: &str, offset: TextSize) -> Option<Vec<String>> {
    let open_paren = keyword_paren_at_offset(text, offset, "override")?;
    let parse = parse_file(text);
    let Some(contract_name) = contract_name_at_offset(text, &parse, offset) else {
        return Some(Vec::new());
    };
    let mut bases = contract_bases_in_parse(&parse, &contract_name)
        .into_iter()
        .filter(|segments| !segments.is_empty())
        .map(|segments| segments.join("."))
        .collect::<Vec<_>>();
    if bases.is_empty() {
        bases = contract_bases_fallback(text, &contract_name);
    }
    let used = ident_list_before_offset(text, offset, open_paren);
    let remaining = bases
        .into_iter()
        .filter(|name| !used.contains(name))
        .collect::<Vec<_>>();
    Some(remaining)
}

fn returns_list_items(text: &str, offset: TextSize) -> Option<Vec<String>> {
    let _open_paren = keyword_paren_at_offset(text, offset, "returns")?;
    let mut known_types = HashSet::new();
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    collect_fallback_imports(
        text,
        &mut known_types,
        &mut items,
        &mut seen,
        TextRange::new(offset, offset),
    );
    collect_fallback_type_defs(
        text,
        &mut known_types,
        &mut items,
        &mut seen,
        TextRange::new(offset, offset),
    );
    let mut types = known_types.into_iter().collect::<Vec<_>>();
    types.extend(builtin_type_candidates());
    types.sort();
    types.dedup();
    Some(types)
}

fn using_brace_context(text: &str, offset: TextSize) -> bool {
    let Some(open_brace) = open_brace_at_offset(text, offset) else {
        return false;
    };
    let Some(keyword) = keyword_before_index(text, open_brace) else {
        return false;
    };
    keyword == "using"
}

fn open_brace_at_offset(text: &str, offset: TextSize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut idx = usize::from(offset).min(bytes.len());
    if idx == 0 {
        return None;
    }
    idx = idx.saturating_sub(1);

    let mut balance = 0i32;
    let mut i = idx;
    loop {
        let b = bytes[i];
        if b == b'}' {
            balance += 1;
        } else if b == b'{' {
            if balance == 0 {
                return Some(i);
            }
            balance -= 1;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

fn matching_close_brace(text: &str, open_brace: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    for (i, b) in bytes.iter().enumerate().skip(open_brace.saturating_add(1)) {
        match *b {
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn brace_preceded_by_open_paren(text: &str, open_brace: usize) -> bool {
    let bytes = text.as_bytes();
    let mut idx = open_brace;
    while idx > 0 && bytes[idx - 1].is_ascii_whitespace() {
        idx -= 1;
    }
    idx > 0 && bytes[idx - 1] == b'('
}

fn next_non_ws_byte(text: &str, idx: usize) -> Option<u8> {
    let bytes = text.as_bytes();
    let mut i = idx;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < bytes.len() {
        Some(bytes[i])
    } else {
        None
    }
}

fn keyword_before_index(text: &str, idx: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let (start, end) = ident_before(bytes, idx)?;
    Some(text[start..end].to_string())
}

fn keyword_paren_at_offset(text: &str, offset: TextSize, keyword: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut idx = usize::from(offset).min(bytes.len());
    if idx == 0 {
        return None;
    }
    idx = idx.saturating_sub(1);

    let mut balance = 0i32;
    let mut i = idx;
    loop {
        let b = bytes[i];
        match b {
            b')' => balance += 1,
            b'(' => {
                if balance == 0 {
                    if let Some((start, end)) = ident_before(bytes, i)
                        && text.get(start..end) == Some(keyword)
                    {
                        return Some(i);
                    }
                    return None;
                }
                balance -= 1;
            }
            _ => {}
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

fn ident_list_before_offset(text: &str, offset: TextSize, open_paren: usize) -> HashSet<String> {
    let bytes = text.as_bytes();
    let end = usize::from(offset).min(bytes.len());
    if open_paren + 1 >= end {
        return HashSet::new();
    }

    let mut used = HashSet::new();
    let mut current = String::new();
    let mut i = open_paren + 1;
    while i < end {
        let b = bytes[i];
        if is_ident_byte(b) {
            let start = i;
            let mut end_ident = i + 1;
            while end_ident < end && is_ident_byte(bytes[end_ident]) {
                end_ident += 1;
            }
            if let Ok(name) = std::str::from_utf8(&bytes[start..end_ident]) {
                current.push_str(name);
            }
            i = end_ident;
            continue;
        }
        if b == b'.' {
            if !current.is_empty() && !current.ends_with('.') {
                current.push('.');
            }
            i += 1;
            continue;
        }
        if b == b',' || b == b')' {
            if !current.is_empty() && !current.ends_with('.') {
                used.insert(current.clone());
            }
            current.clear();
            if b == b')' {
                break;
            }
            i += 1;
            continue;
        }
        if !b.is_ascii_whitespace() {
            if !current.is_empty() && !current.ends_with('.') {
                used.insert(current.clone());
            }
            current.clear();
        }
        i += 1;
    }
    if !current.is_empty() && !current.ends_with('.') {
        used.insert(current);
    }
    used
}

fn call_options_has_new_keyword(text: &str, open_brace: usize) -> bool {
    let bytes = text.as_bytes();
    if open_brace == 0 {
        return false;
    }
    let mut idx = open_brace;
    while idx > 0 && bytes[idx - 1].is_ascii_whitespace() {
        idx -= 1;
    }
    while idx > 0 {
        let b = bytes[idx - 1];
        if is_ident_byte(b) {
            while idx > 0 && is_ident_byte(bytes[idx - 1]) {
                idx -= 1;
            }
            continue;
        }
        if b == b'.' {
            idx -= 1;
            continue;
        }
        break;
    }
    while idx > 0 && bytes[idx - 1].is_ascii_whitespace() {
        idx -= 1;
    }
    let end = idx;
    while idx > 0 && is_ident_byte(bytes[idx - 1]) {
        idx -= 1;
    }
    if end > idx {
        text.get(idx..end) == Some("new")
    } else {
        false
    }
}

fn named_arg_candidates(text: &str, offset: TextSize, open_brace: usize) -> Option<Vec<String>> {
    let bytes = text.as_bytes();
    let mut idx = open_brace;
    while idx > 0 && bytes[idx - 1].is_ascii_whitespace() {
        idx -= 1;
    }
    if idx == 0 || bytes[idx - 1] != b'(' {
        return Some(Vec::new());
    }
    let open_paren = idx - 1;
    let Some((start, end)) = ident_before(bytes, open_paren) else {
        return Some(Vec::new());
    };
    let mut j = start;
    while j > 0 && bytes[j - 1].is_ascii_whitespace() {
        j -= 1;
    }
    if j > 0 && bytes[j - 1] == b'.' {
        return Some(Vec::new());
    }
    let func_name = text.get(start..end)?.to_string();
    let parse = parse_file(text);
    let mut params = if let Some(contract_name) = contract_name_at_offset(text, &parse, offset) {
        function_params_in_parse(&parse, &contract_name, &func_name)
    } else {
        Vec::new()
    };
    if params.is_empty() {
        params = function_params_by_name(&parse, &func_name);
    }
    params.sort();
    params.dedup();
    Some(params)
}

fn function_params_in_parse(parse: &Parse, contract_name: &str, func_name: &str) -> Vec<String> {
    let mut params = Vec::new();
    parse.with_session(|| {
        for item in parse.tree().items.iter() {
            let ItemKind::Contract(contract) = &item.kind else {
                continue;
            };
            if contract.name.as_str() != contract_name {
                continue;
            }
            for member in contract.body.iter() {
                let ItemKind::Function(func) = &member.kind else {
                    continue;
                };
                let Some(name) = func.header.name else {
                    continue;
                };
                if name.as_str() != func_name {
                    continue;
                }
                collect_param_names(&mut params, func.header.parameters.vars);
            }
        }
    });
    params
}

fn function_params_by_name(parse: &Parse, func_name: &str) -> Vec<String> {
    let mut params = Vec::new();
    parse.with_session(|| {
        for item in parse.tree().items.iter() {
            let ItemKind::Contract(contract) = &item.kind else {
                continue;
            };
            for member in contract.body.iter() {
                let ItemKind::Function(func) = &member.kind else {
                    continue;
                };
                let Some(name) = func.header.name else {
                    continue;
                };
                if name.as_str() != func_name {
                    continue;
                }
                collect_param_names(&mut params, func.header.parameters.vars);
            }
        }
    });
    params
}

fn collect_param_names(params: &mut Vec<String>, vars: &[VariableDefinition<'static>]) {
    for var in vars {
        if let Some(name) = var.name {
            params.push(name.as_str().to_string());
        }
    }
}

fn contract_name_at_offset(text: &str, parse: &Parse, offset: TextSize) -> Option<String> {
    let mut containing: Option<String> = None;
    let mut last_before: Option<(TextSize, String)> = None;

    parse.with_session(|| {
        for item in parse.tree().items.iter() {
            let ItemKind::Contract(contract) = &item.kind else {
                continue;
            };
            let Some(range) = parse.span_to_text_range(item.span) else {
                continue;
            };
            let name = contract.name.as_str().to_string();
            if range_contains(range, offset) {
                containing = Some(name);
                break;
            }
            if range.start() <= offset {
                last_before = Some((range.start(), name));
            }
        }
    });

    if let Some(name) = containing.or_else(|| last_before.map(|(_, name)| name)) {
        return Some(name);
    }

    let limit = usize::from(offset).min(text.len());
    let prefix = &text[..limit];
    let mut lexer = FallbackLexer::new(prefix);
    let mut brace_depth = 0usize;
    let mut expect_contract_name = false;
    let mut pending_contract: Option<String> = None;
    let mut contract_body_depth: Option<usize> = None;
    let mut current_contract: Option<String> = None;

    while let Some(token) = lexer.next_token() {
        match token {
            FallbackToken::Ident(ident) => {
                if expect_contract_name {
                    pending_contract = Some(ident);
                    expect_contract_name = false;
                    continue;
                }
                if brace_depth == 0
                    && matches!(ident.as_str(), "contract" | "interface" | "library")
                {
                    expect_contract_name = true;
                }
            }
            FallbackToken::Punct(punct) => match punct {
                '{' => {
                    brace_depth += 1;
                    if let Some(name) = pending_contract.take() {
                        contract_body_depth = Some(brace_depth);
                        current_contract = Some(name);
                    }
                }
                '}' => {
                    if let Some(depth) = contract_body_depth
                        && brace_depth == depth
                    {
                        contract_body_depth = None;
                        current_contract = None;
                    }
                    brace_depth = brace_depth.saturating_sub(1);
                }
                _ => {}
            },
        }
    }

    current_contract
}

fn contract_bases_fallback(text: &str, target_name: &str) -> Vec<String> {
    let mut lexer = FallbackLexer::new(text);
    let mut bases = Vec::new();
    let mut expect_name = false;
    let mut in_target = false;
    let mut collecting_bases = false;
    let mut current = String::new();
    let mut paren_depth = 0usize;

    while let Some(token) = lexer.next_token() {
        match token {
            FallbackToken::Ident(ident) => {
                if expect_name {
                    in_target = ident == target_name;
                    expect_name = false;
                    collecting_bases = false;
                    current.clear();
                    paren_depth = 0;
                    continue;
                }

                if matches!(ident.as_str(), "contract" | "interface" | "library") {
                    expect_name = true;
                    continue;
                }

                if in_target {
                    if ident == "is" {
                        collecting_bases = true;
                        continue;
                    }
                    if collecting_bases
                        && paren_depth == 0
                        && (current.is_empty() || current.ends_with('.'))
                    {
                        current.push_str(&ident);
                    }
                }
            }
            FallbackToken::Punct(punct) => {
                if !in_target {
                    continue;
                }
                match punct {
                    '{' => {
                        if !current.is_empty() && !current.ends_with('.') {
                            bases.push(current.clone());
                        }
                        break;
                    }
                    ',' => {
                        if collecting_bases && paren_depth == 0 {
                            if !current.is_empty() && !current.ends_with('.') {
                                bases.push(current.clone());
                            }
                            current.clear();
                        }
                    }
                    '.' => {
                        if collecting_bases
                            && paren_depth == 0
                            && !current.is_empty()
                            && !current.ends_with('.')
                        {
                            current.push('.');
                        }
                    }
                    '(' => {
                        if collecting_bases {
                            paren_depth += 1;
                            if !current.is_empty() && !current.ends_with('.') {
                                bases.push(current.clone());
                            }
                            current.clear();
                        }
                    }
                    ')' => {
                        if collecting_bases && paren_depth > 0 {
                            paren_depth -= 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    bases
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

fn collect_fallback_struct_fields(text: &str, structs: &mut HashMap<String, Vec<String>>) {
    let mut lexer = FallbackLexer::new(text);
    let mut brace_depth = 0usize;
    let mut expect_struct_name = false;
    let mut pending_struct: Option<String> = None;
    let mut struct_body_depth: Option<usize> = None;
    let mut current_struct: Option<String> = None;
    let mut statement_idents: Vec<String> = Vec::new();

    while let Some(token) = lexer.next_token() {
        match token {
            FallbackToken::Ident(ident) => {
                if expect_struct_name {
                    pending_struct = Some(ident);
                    expect_struct_name = false;
                    continue;
                }
                if ident == "struct" && struct_body_depth.is_none() {
                    expect_struct_name = true;
                    continue;
                }
                if struct_body_depth.is_some() && brace_depth == struct_body_depth.unwrap() {
                    statement_idents.push(ident);
                }
            }
            FallbackToken::Punct(punct) => match punct {
                '{' => {
                    brace_depth += 1;
                    if let Some(name) = pending_struct.take() {
                        struct_body_depth = Some(brace_depth);
                        structs.entry(name.clone()).or_default();
                        current_struct = Some(name);
                        statement_idents.clear();
                    }
                }
                '}' => {
                    if struct_body_depth.is_some() && brace_depth == struct_body_depth.unwrap() {
                        struct_body_depth = None;
                        current_struct = None;
                        statement_idents.clear();
                    }
                    brace_depth = brace_depth.saturating_sub(1);
                }
                ';' => {
                    if let Some(body_depth) = struct_body_depth
                        && brace_depth == body_depth
                        && let Some(name) = statement_idents.last()
                        && let Some(struct_name) = current_struct.as_ref()
                        && let Some(fields) = structs.get_mut(struct_name)
                    {
                        fields.push(name.clone());
                    }
                    statement_idents.clear();
                }
                _ => {}
            },
        }
    }
}

fn struct_literal_fields_at_offset(
    text: &str,
    offset: TextSize,
    struct_fields: &HashMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    let (struct_name, open_brace) = struct_literal_name_at_offset(text, offset)?;
    let fields = struct_fields.get(&struct_name)?;
    let used = named_fields_before_offset(text, offset, open_brace);
    let remaining = fields
        .iter()
        .filter(|name| !used.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Some(remaining)
}

fn struct_literal_name_at_offset(text: &str, offset: TextSize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut idx = usize::from(offset).min(bytes.len());
    if idx == 0 {
        return None;
    }
    idx = idx.saturating_sub(1);

    let mut balance = 0i32;
    let mut open_brace = None;
    let mut i = idx;
    loop {
        let b = bytes[i];
        if b == b'}' {
            balance += 1;
        } else if b == b'{' {
            if balance == 0 {
                open_brace = Some(i);
                break;
            }
            balance -= 1;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    let open_brace = open_brace?;
    if !struct_literal_name_position(text, offset, open_brace) {
        return None;
    }

    let mut j = open_brace;
    while j > 0 && bytes[j - 1].is_ascii_whitespace() {
        j -= 1;
    }
    if j == 0 || bytes[j - 1] != b'(' {
        return None;
    }
    j -= 1;
    while j > 0 && bytes[j - 1].is_ascii_whitespace() {
        j -= 1;
    }
    if j == 0 {
        return None;
    }
    let mut end = j;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end == 0 || !is_ident_byte(bytes[end - 1]) {
        return None;
    }
    let mut start = end;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    if start == end {
        return None;
    }
    Some((text[start..end].to_string(), open_brace))
}

fn struct_literal_name_position(text: &str, offset: TextSize, open_brace: usize) -> bool {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut idx = usize::from(offset).min(bytes.len());
    if idx == 0 {
        return false;
    }
    idx = idx.saturating_sub(1);
    let mut i = idx;
    loop {
        if i < open_brace {
            break;
        }
        let b = bytes[i];
        if b == b':' {
            return false;
        }
        if b == b',' || i == open_brace {
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

fn named_fields_before_offset(text: &str, offset: TextSize, open_brace: usize) -> HashSet<String> {
    let bytes = text.as_bytes();
    let mut used = HashSet::new();
    let end = usize::from(offset).min(bytes.len());
    if open_brace + 1 >= end {
        return used;
    }

    let mut brace_depth = 0i32;
    let mut paren_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut i = open_brace + 1;
    let mut last_ident: Option<String> = None;

    while i < end {
        let b = bytes[i];
        match b {
            b'{' => brace_depth += 1,
            b'}' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                }
            }
            b'(' => paren_depth += 1,
            b')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
            }
            b'[' => bracket_depth += 1,
            b']' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                }
            }
            b':' => {
                if brace_depth == 0
                    && paren_depth == 0
                    && bracket_depth == 0
                    && let Some(name) = last_ident.take()
                {
                    used.insert(name);
                }
            }
            b',' => {
                if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 {
                    last_ident = None;
                }
            }
            b'"' | b'\'' => {
                let quote = b;
                i += 1;
                while i < end {
                    let c = bytes[i];
                    if c == b'\\' {
                        i = i.saturating_add(2);
                        continue;
                    }
                    if c == quote {
                        break;
                    }
                    i += 1;
                }
            }
            _ => {
                if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 && is_ident_byte(b) {
                    let start = i;
                    let mut end_ident = i + 1;
                    while end_ident < end && is_ident_byte(bytes[end_ident]) {
                        end_ident += 1;
                    }
                    if let Ok(name) = std::str::from_utf8(&bytes[start..end_ident]) {
                        last_ident = Some(name.to_string());
                    }
                    i = end_ident.saturating_sub(1);
                }
            }
        }
        i += 1;
    }

    used
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

fn builtin_type_candidates() -> Vec<String> {
    let mut types = vec![
        "address".to_string(),
        "bool".to_string(),
        "string".to_string(),
        "byte".to_string(),
        "bytes".to_string(),
        "mapping".to_string(),
        "function".to_string(),
        "uint".to_string(),
        "int".to_string(),
    ];
    for size in (8..=256).step_by(8) {
        types.push(format!("uint{size}"));
        types.push(format!("int{size}"));
    }
    for size in 1..=32 {
        types.push(format!("bytes{size}"));
    }
    types
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

fn apply_callable_format(
    label: &str,
    kind: CompletionItemKind,
    detail: Option<&str>,
) -> (String, Option<String>, CompletionInsertTextFormat) {
    let base = label.strip_suffix("()").unwrap_or(label);
    match kind {
        CompletionItemKind::Function => {
            let display = format!("{base}()");
            let insert = Some(format!("{base}($0)"));
            (display, insert, CompletionInsertTextFormat::Snippet)
        }
        CompletionItemKind::Modifier => {
            let has_args = detail.map(|d| !d.starts_with("()")).unwrap_or(true);
            if has_args {
                let display = format!("{base}()");
                let insert = Some(format!("{base}($0)"));
                (display, insert, CompletionInsertTextFormat::Snippet)
            } else {
                (base.to_string(), None, CompletionInsertTextFormat::Plain)
            }
        }
        _ => (label.to_string(), None, CompletionInsertTextFormat::Plain),
    }
}

fn push_completion_item(
    label: &str,
    kind: CompletionItemKind,
    range: TextRange,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<(String, CompletionItemKind)>,
) {
    let (label, insert_text, insert_text_format) = apply_callable_format(label, kind, None);
    if seen.insert((label.to_string(), kind)) {
        items.push(CompletionItem {
            label,
            kind,
            replacement_range: range,
            detail: None,
            origin: None,
            insert_text,
            insert_text_format,
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

#[derive(Clone, Copy, Debug)]
enum MemberAccessKind {
    Instance,
    Type,
}

fn member_items_for_contract_def(
    db: &dyn HirDatabase,
    program: &sa_hir::HirProgram,
    contract_def: sa_def::DefId,
    range: TextRange,
    access: MemberAccessKind,
) -> Vec<CompletionItem> {
    let Some(entry) = program.def_map().entry(contract_def) else {
        return Vec::new();
    };
    let file_id = entry.location().file_id();
    let name = entry.location().name();

    contract_members_with_inheritance(db, program, file_id, name, access, range)
}

fn member_items_for_named_contract(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    receiver: &str,
    range: TextRange,
) -> Vec<CompletionItem> {
    let program = lowered_program(db, project_id);
    let contract_def = program
        .resolve_contract(file_id, receiver)
        .or_else(|| unique_contract_def(&program, receiver));
    let Some(contract_def) = contract_def else {
        return Vec::new();
    };
    let access = MemberAccessKind::Type;
    member_items_for_contract_def(db, &program, contract_def, range, access)
}

fn member_items_from_local_decl(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    offset: TextSize,
    receiver: &str,
    range: TextRange,
) -> Option<Vec<CompletionItem>> {
    let text = db.file_input(file_id).text(db);
    let mut parse = parse_file(text.as_ref());
    if !parse.errors().is_empty() && member_access_needs_patch(text.as_ref(), offset) {
        let mut patched = text.to_string();
        let insert_at = usize::from(offset).min(patched.len());
        patched.insert_str(insert_at, "__sa_dummy();");
        let patched_parse = parse_file(&patched);
        if patched_parse.errors().is_empty() {
            parse = patched_parse;
        }
    }
    if !parse.errors().is_empty()
        && let Some(prefix_parse) = parse_prefix_for_member_access(text.as_ref(), offset)
    {
        parse = prefix_parse;
    }
    let var = find_local_var_definition(&parse, offset, receiver)?;
    let (segments, type_ident) = match &var.ty.kind {
        TypeKind::Custom(path) => {
            let segments = parse.with_session(|| {
                path.segments()
                    .iter()
                    .map(|segment| segment.as_str().to_string())
                    .collect::<Vec<_>>()
            });
            (segments, path.get_ident())
        }
        _ => {
            let items = builtin_member_items(&var.ty.kind, var.data_location, range);
            return Some(items);
        }
    };
    let program = lowered_program(db, project_id);
    let contract_def = if segments.len() > 1 {
        let qualifier = segments.first()?;
        let name = segments.last()?;
        resolve_contract_def_from_qualified_path(db, project_id, file_id, &parse, qualifier, name)
    } else {
        let Some(type_ident) = type_ident else {
            return Some(Vec::new());
        };
        let type_name = parse.with_session(|| type_ident.as_str().to_string());
        let lookup_name =
            resolve_import_alias_name(&parse, type_name.as_str()).unwrap_or(type_name);
        program
            .resolve_contract(file_id, lookup_name.as_str())
            .or_else(|| unique_contract_def(&program, lookup_name.as_str()))
    };
    let Some(contract_def) = contract_def else {
        return Some(Vec::new());
    };
    let access = MemberAccessKind::Instance;
    let items = member_items_for_contract_def(db, &program, contract_def, range, access);
    Some(items)
}

fn builtin_member_items(
    ty: &TypeKind,
    data_location: Option<DataLocation>,
    range: TextRange,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    match ty {
        TypeKind::Elementary(elementary) => match elementary {
            ElementaryType::Address(payable) => {
                push_builtin_member(&mut items, "balance", CompletionItemKind::Variable, range);
                push_builtin_member(&mut items, "code", CompletionItemKind::Variable, range);
                push_builtin_member(&mut items, "codehash", CompletionItemKind::Variable, range);
                push_builtin_member(&mut items, "call", CompletionItemKind::Function, range);
                push_builtin_member(
                    &mut items,
                    "delegatecall",
                    CompletionItemKind::Function,
                    range,
                );
                push_builtin_member(
                    &mut items,
                    "staticcall",
                    CompletionItemKind::Function,
                    range,
                );
                if *payable {
                    push_builtin_member(
                        &mut items,
                        "transfer",
                        CompletionItemKind::Function,
                        range,
                    );
                    push_builtin_member(&mut items, "send", CompletionItemKind::Function, range);
                }
            }
            ElementaryType::Bytes | ElementaryType::String | ElementaryType::FixedBytes(_) => {
                push_builtin_member(&mut items, "length", CompletionItemKind::Variable, range);
                if allow_storage_mutation(data_location)
                    && matches!(elementary, ElementaryType::Bytes)
                {
                    push_builtin_member(&mut items, "push", CompletionItemKind::Function, range);
                    push_builtin_member(&mut items, "pop", CompletionItemKind::Function, range);
                }
            }
            _ => {}
        },
        TypeKind::Array(array) => {
            push_builtin_member(&mut items, "length", CompletionItemKind::Variable, range);
            if array.size.is_none() && allow_storage_mutation(data_location) {
                push_builtin_member(&mut items, "push", CompletionItemKind::Function, range);
                push_builtin_member(&mut items, "pop", CompletionItemKind::Function, range);
            }
        }
        _ => {}
    }
    items
}

fn push_builtin_member(
    items: &mut Vec<CompletionItem>,
    label: &str,
    kind: CompletionItemKind,
    range: TextRange,
) {
    let detail = builtin_member_detail(label, kind);
    let (label, insert_text, insert_text_format) =
        apply_callable_format(label, kind, detail.as_deref());
    items.push(CompletionItem {
        label,
        kind,
        replacement_range: range,
        detail,
        origin: Some("builtin".to_string()),
        insert_text,
        insert_text_format,
    });
}

fn builtin_member_detail(label: &str, kind: CompletionItemKind) -> Option<String> {
    match (label, kind) {
        ("balance", CompletionItemKind::Variable) => Some("uint256".to_string()),
        ("code", CompletionItemKind::Variable) => Some("bytes".to_string()),
        ("codehash", CompletionItemKind::Variable) => Some("bytes32".to_string()),
        ("length", CompletionItemKind::Variable) => Some("uint256".to_string()),
        ("transfer", CompletionItemKind::Function) => Some("(uint256) -> ()".to_string()),
        ("send", CompletionItemKind::Function) => Some("(uint256) -> (bool)".to_string()),
        ("call", CompletionItemKind::Function)
        | ("delegatecall", CompletionItemKind::Function)
        | ("staticcall", CompletionItemKind::Function) => {
            Some("(bytes) -> (bool,bytes)".to_string())
        }
        ("push", CompletionItemKind::Function) | ("pop", CompletionItemKind::Function) => {
            Some("() -> ()".to_string())
        }
        _ => None,
    }
}

fn allow_storage_mutation(data_location: Option<DataLocation>) -> bool {
    matches!(
        data_location,
        None | Some(DataLocation::Storage) | Some(DataLocation::Transient)
    )
}

struct ContractMemberAstContext<'a> {
    db: &'a dyn HirDatabase,
    file_id: FileId,
    contract_name: &'a str,
    origin: Option<String>,
    access: MemberAccessKind,
    base_accessible: bool,
    range: TextRange,
}

fn contract_members_with_inheritance(
    db: &dyn HirDatabase,
    program: &sa_hir::HirProgram,
    file_id: FileId,
    contract_name: &str,
    access: MemberAccessKind,
    range: TextRange,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let base_accessible = matches!(access, MemberAccessKind::Type);
    let context = ContractMemberAstContext {
        db,
        file_id,
        contract_name,
        origin: None,
        access,
        base_accessible,
        range,
    };
    push_contract_members_from_ast(&context, &mut items, &mut seen);

    let mut visited = HashSet::new();
    let mut pending = Vec::new();
    for base_path in contract_bases_in_file(db, file_id, contract_name) {
        if let Some(base_id) = resolve_contract_path(program, file_id, &base_path) {
            pending.push(base_id);
        }
    }

    while let Some(base_id) = pending.pop() {
        if !visited.insert(base_id) {
            continue;
        }
        let Some(entry) = program.def_map().entry(base_id) else {
            continue;
        };
        let base_file_id = entry.location().file_id();
        let base_name = entry.location().name();

        let context = ContractMemberAstContext {
            db,
            file_id: base_file_id,
            contract_name: base_name,
            origin: Some(base_name.to_string()),
            access,
            base_accessible,
            range,
        };
        push_contract_members_from_ast(&context, &mut items, &mut seen);

        let base_paths = contract_bases_in_file(db, base_file_id, base_name);
        for base_path in base_paths {
            if let Some(next_id) = resolve_contract_path(program, base_file_id, &base_path) {
                pending.push(next_id);
            }
        }
    }

    items
}

fn push_contract_members_from_ast(
    context: &ContractMemberAstContext<'_>,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<(String, CompletionItemKind)>,
) {
    let text = context.db.file_input(context.file_id).text(context.db);
    let parse = parse_file(text.as_ref());
    parse.with_session(|| {
        for item in parse.tree().items.iter() {
            let ItemKind::Contract(contract) = &item.kind else {
                continue;
            };
            if contract.name.as_str() != context.contract_name {
                continue;
            }
            let is_library = contract.kind == ContractKind::Library;
            let default_vis = if contract.kind == ContractKind::Interface {
                Visibility::External
            } else {
                Visibility::Internal
            };

            for member in contract.body.iter() {
                match &member.kind {
                    ItemKind::Function(func) => {
                        if !func.kind.is_ordinary() {
                            continue;
                        }
                        let Some(name) = func.header.name else {
                            continue;
                        };
                        let visibility = func.header.visibility().unwrap_or(default_vis);
                        if !allow_function_visibility(
                            visibility,
                            context.access,
                            is_library,
                            context.base_accessible,
                        ) {
                            continue;
                        }
                        let label = name.as_str().to_string();
                        let detail = ast_function_detail(&parse, text.as_ref(), func);
                        push_member_item(
                            label,
                            CompletionItemKind::Function,
                            detail,
                            context.origin.clone(),
                            context.range,
                            items,
                            seen,
                        );
                    }
                    ItemKind::Variable(var) => {
                        let Some(name) = var.name else {
                            continue;
                        };
                        let visibility = var.visibility.unwrap_or(default_vis);
                        let is_constant = var
                            .mutability
                            .map(|mutability| mutability.is_constant())
                            .unwrap_or(false);
                        if !allow_variable_visibility(
                            visibility,
                            is_constant,
                            context.access,
                            is_library,
                            context.base_accessible,
                        ) {
                            continue;
                        }
                        let label = name.as_str().to_string();
                        let detail = ast_variable_detail(&parse, text.as_ref(), var);
                        push_member_item(
                            label,
                            CompletionItemKind::Variable,
                            detail,
                            context.origin.clone(),
                            context.range,
                            items,
                            seen,
                        );
                    }
                    _ => {}
                }
            }
        }
    });
}

fn ast_function_detail(
    parse: &Parse,
    text: &str,
    func: &sa_syntax::ast::ItemFunction<'_>,
) -> Option<String> {
    let params = ast_param_types(parse, text, func.header.parameters.vars);
    let returns = func
        .header
        .returns
        .as_ref()
        .map(|returns| ast_param_types(parse, text, returns.vars))
        .unwrap_or_default();
    Some(format!("({}) -> ({})", params.join(","), returns.join(",")))
}

fn ast_variable_detail(parse: &Parse, text: &str, var: &VariableDefinition<'_>) -> Option<String> {
    let mut ty = ast_type_text(parse, text, &var.ty)?;
    if let Some(location) = var.data_location {
        ty.push(' ');
        ty.push_str(location.to_str());
    }
    Some(ty)
}

fn ast_param_types(parse: &Parse, text: &str, vars: &[VariableDefinition<'_>]) -> Vec<String> {
    vars.iter()
        .map(|var| ast_type_text(parse, text, &var.ty).unwrap_or_else(|| "unknown".to_string()))
        .collect()
}

fn ast_type_text(parse: &Parse, text: &str, ty: &sa_syntax::ast::Type<'_>) -> Option<String> {
    let range = parse.span_to_text_range(ty.span)?;
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    text.get(start..end).map(|slice| slice.trim().to_string())
}

fn push_member_item(
    label: String,
    kind: CompletionItemKind,
    detail: Option<String>,
    origin: Option<String>,
    range: TextRange,
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<(String, CompletionItemKind)>,
) {
    let (label, insert_text, insert_text_format) =
        apply_callable_format(&label, kind, detail.as_deref());
    if seen.insert((label.clone(), kind)) {
        items.push(CompletionItem {
            label,
            kind,
            replacement_range: range,
            detail,
            origin,
            insert_text,
            insert_text_format,
        });
    }
}

fn allow_function_visibility(
    visibility: Visibility,
    access: MemberAccessKind,
    is_library: bool,
    base_accessible: bool,
) -> bool {
    if is_library {
        return visibility >= Visibility::Internal;
    }
    match access {
        MemberAccessKind::Instance => {
            matches!(visibility, Visibility::Public | Visibility::External)
        }
        MemberAccessKind::Type => {
            matches!(visibility, Visibility::Public | Visibility::External)
                || (base_accessible && visibility == Visibility::Internal)
        }
    }
}

fn allow_variable_visibility(
    visibility: Visibility,
    is_constant: bool,
    access: MemberAccessKind,
    is_library: bool,
    base_accessible: bool,
) -> bool {
    if is_library {
        return visibility >= Visibility::Internal;
    }
    match access {
        MemberAccessKind::Instance => !is_constant && visibility == Visibility::Public,
        MemberAccessKind::Type => {
            matches!(visibility, Visibility::Public)
                || (base_accessible && visibility == Visibility::Internal)
        }
    }
}

fn resolve_contract_path(
    program: &sa_hir::HirProgram,
    file_id: FileId,
    path: &[String],
) -> Option<sa_def::DefId> {
    let name = path.last()?.as_str();
    let def_id = if path.len() == 1 {
        program.resolve_contract(file_id, name)
    } else {
        let qualifier = path.first()?.as_str();
        program.resolve_qualified_symbol(file_id, qualifier, name)
    };
    match def_id {
        Some(def_id @ sa_def::DefId::Contract(_)) => Some(def_id),
        _ => None,
    }
}

fn contract_bases_in_file(
    db: &dyn HirDatabase,
    file_id: FileId,
    contract_name: &str,
) -> Vec<Vec<String>> {
    let text = db.file_input(file_id).text(db);
    let parse = parse_file(text.as_ref());
    contract_bases_in_parse(&parse, contract_name)
}

fn contract_bases_in_parse(parse: &Parse, contract_name: &str) -> Vec<Vec<String>> {
    parse.with_session(|| {
        for item in parse.tree().items.iter() {
            if let ItemKind::Contract(contract) = &item.kind
                && contract.name.as_str() == contract_name
            {
                let bases = contract
                    .bases
                    .iter()
                    .filter_map(|base| {
                        let segments: Vec<String> = base
                            .name
                            .segments()
                            .iter()
                            .map(|segment| segment.as_str().to_string())
                            .collect();
                        (!segments.is_empty()).then_some(segments)
                    })
                    .collect();
                return bases;
            }
        }
        Vec::new()
    })
}

fn member_access_needs_patch(text: &str, offset: TextSize) -> bool {
    let bytes = text.as_bytes();
    let mut idx = usize::from(offset).min(bytes.len());
    while idx > 0 && bytes[idx - 1].is_ascii_whitespace() {
        idx -= 1;
    }
    idx > 0 && bytes[idx - 1] == b'.'
}

fn parse_prefix_for_member_access(text: &str, offset: TextSize) -> Option<Parse> {
    let end = usize::from(offset).min(text.len());
    let mut prefix = text[..end].to_string();
    let prefix_offset = TextSize::from(prefix.len() as u32);
    if member_access_needs_patch(&prefix, prefix_offset) {
        prefix.push_str("__sa_dummy();");
    }
    let mut brace_depth = 0usize;
    let mut lexer = FallbackLexer::new(&prefix);
    while let Some(token) = lexer.next_token() {
        if let FallbackToken::Punct(punct) = token {
            match punct {
                '{' => brace_depth += 1,
                '}' => brace_depth = brace_depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    for _ in 0..brace_depth {
        prefix.push('}');
    }
    let parse = parse_file(&prefix);
    parse.errors().is_empty().then_some(parse)
}

fn resolve_import_alias_name(parse: &Parse, local_name: &str) -> Option<String> {
    parse.with_session(|| {
        for (_, directive) in parse.tree().imports() {
            if let sa_syntax::ast::ImportItems::Aliases(aliases) = &directive.items {
                for (original, alias) in aliases.iter() {
                    let alias_ident = alias.as_ref().unwrap_or(original);
                    if alias_ident.as_str() == local_name {
                        return Some(original.as_str().to_string());
                    }
                }
            }
        }
        None
    })
}

fn resolve_source_alias_path(parse: &Parse, qualifier: &str) -> Option<String> {
    parse.with_session(|| {
        for (_, directive) in parse.tree().imports() {
            match &directive.items {
                sa_syntax::ast::ImportItems::Plain(Some(alias)) => {
                    if alias.as_str() == qualifier {
                        return Some(directive.path.value.as_str().to_string());
                    }
                }
                sa_syntax::ast::ImportItems::Glob(alias) => {
                    if alias.as_str() == qualifier {
                        return Some(directive.path.value.as_str().to_string());
                    }
                }
                _ => {}
            }
        }
        None
    })
}

fn resolve_contract_def_from_qualified_path(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    parse: &Parse,
    qualifier: &str,
    name: &str,
) -> Option<sa_def::DefId> {
    let program = lowered_program(db, project_id);
    if let Some(def_id) = program.resolve_qualified_symbol(file_id, qualifier, name) {
        let entry = program.def_map().entry(def_id)?;
        if entry.kind() == DefKind::Contract {
            return Some(def_id);
        }
    }

    let import_path = resolve_source_alias_path(parse, qualifier)?;
    let project = db.project_input(project_id);
    let workspace = project.workspace(db);
    let remappings = project.config(db).active_profile().remappings();
    let resolver = FoundryResolver::new(workspace.as_ref(), remappings).ok();
    let current_path = db.file_path(file_id);
    let resolved = resolve_import_path_with_resolver(
        workspace.as_ref(),
        remappings,
        current_path.as_ref(),
        &import_path,
        resolver.as_ref(),
    );
    let remap_fallback = resolve_import_path_with_remappings_fallback(
        workspace.as_ref(),
        remappings,
        current_path.as_ref(),
        &import_path,
    );
    let resolved = if let Some(remap) = remap_fallback.as_ref()
        && remap.used_context
    {
        Some(remap.path.clone())
    } else {
        resolved
            .or_else(|| resolve_relative_import_fallback(current_path.as_ref(), &import_path))
            .or_else(|| remap_fallback.map(|fallback| fallback.path))
    }?;
    let target_file_id = file_id_for_path(db, &resolved)?;
    contract_def_in_file(&program, target_file_id, name)
}

fn contract_def_in_file(
    program: &sa_hir::HirProgram,
    file_id: FileId,
    name: &str,
) -> Option<sa_def::DefId> {
    program
        .def_map()
        .entries_by_name_in_file(file_id, name)
        .into_iter()
        .find(|entry| entry.kind() == DefKind::Contract)
        .map(|entry| entry.id())
}

fn resolve_relative_import_fallback(
    current_path: &NormalizedPath,
    import_path: &str,
) -> Option<NormalizedPath> {
    let import_path = import_path.replace('\\', "/");
    if !import_path.starts_with("./") && !import_path.starts_with("../") {
        return None;
    }
    let current = std::path::Path::new(current_path.as_str());
    let base = current
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let combined = base.join(import_path);
    Some(NormalizedPath::new(combined.to_string_lossy()))
}

struct RemapFallback {
    path: NormalizedPath,
    used_context: bool,
}

fn resolve_import_path_with_remappings_fallback(
    workspace: &sa_project_model::FoundryWorkspace,
    remappings: &[sa_project_model::Remapping],
    current_path: &NormalizedPath,
    import_path: &str,
) -> Option<RemapFallback> {
    let root = workspace.root().as_str();
    let rel_current = current_path
        .as_str()
        .strip_prefix(root)
        .unwrap_or(current_path.as_str())
        .trim_start_matches('/');

    let mut context_matches = Vec::new();
    let mut plain_matches = Vec::new();
    for remap in remappings {
        if !import_path.starts_with(remap.from()) {
            continue;
        }
        if let Some(context) = remap.context() {
            let context = context.trim_end_matches('/');
            if rel_current == context || rel_current.starts_with(&format!("{context}/")) {
                context_matches.push(remap);
            }
        } else {
            plain_matches.push(remap);
        }
    }

    let (candidates, used_context) = if !context_matches.is_empty() {
        (context_matches, true)
    } else {
        (plain_matches, false)
    };
    let remap = candidates
        .into_iter()
        .max_by_key(|remap| remap.from().len())?;

    let remainder = import_path.strip_prefix(remap.from()).unwrap_or("");
    let mut resolved = std::path::PathBuf::from(root);
    resolved.push(remap.to());
    resolved.push(remainder);
    Some(RemapFallback {
        path: NormalizedPath::new(resolved.to_string_lossy()),
        used_context,
    })
}

fn file_id_for_path(db: &dyn HirDatabase, path: &NormalizedPath) -> Option<FileId> {
    db.file_ids()
        .into_iter()
        .find(|file_id| db.file_path(*file_id).as_ref() == path)
}

fn unique_contract_def(program: &sa_hir::HirProgram, name: &str) -> Option<sa_def::DefId> {
    let entries = program.def_map().entries_by_name(DefKind::Contract, name)?;
    if entries.len() == 1 {
        Some(entries[0].id())
    } else {
        None
    }
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
                if let SpannedOption::Some(var) = var {
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
    let matches = parse.with_session(|| name.as_str() == receiver);
    if !matches {
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
                    let detail = None;
                    let (label, insert_text, insert_text_format) =
                        apply_callable_format(&ident, kind, detail.as_deref());
                    items.push(CompletionItem {
                        label,
                        kind,
                        replacement_range: range,
                        detail,
                        origin: None,
                        insert_text,
                        insert_text_format,
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
                            let detail = None;
                            let (label, insert_text, insert_text_format) = apply_callable_format(
                                name,
                                CompletionItemKind::Variable,
                                detail.as_deref(),
                            );
                            items.push(CompletionItem {
                                label,
                                kind: CompletionItemKind::Variable,
                                replacement_range: range,
                                detail,
                                origin: None,
                                insert_text,
                                insert_text_format,
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
    let kind = completion_kind_from_sema(item.kind);
    let detail = item.detail;
    let (label, insert_text, insert_text_format) =
        apply_callable_format(&item.label, kind, detail.as_deref());
    CompletionItem {
        label,
        kind,
        replacement_range: range,
        detail,
        origin: item.origin,
        insert_text,
        insert_text_format,
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
            detail: None,
            origin: None,
            insert_text: None,
            insert_text_format: CompletionInsertTextFormat::Plain,
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

    use sa_paths::NormalizedPath;
    use sa_project_model::Remapping;
    use sa_test_support::{extract_offset, setup_db};

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
        assert!(labels.contains("bar()"));
        assert!(labels.contains("Evt"));
        assert!(labels.contains("Boom"));
        assert!(labels.contains("onlyOwner()"));
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

    #[test]
    fn completions_resolve_member_access_on_incomplete_local_decl() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import {A} from "./A.sol";

contract X {
    function f() public returns (uint256) {
        A a = new A();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (
                NormalizedPath::new("/workspace/src/Main.sol"),
                main_text.clone(),
            ),
            (
                NormalizedPath::new("/workspace/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
        ];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("ping()"));
    }

    #[test]
    fn completions_recover_builtin_address_members_on_parse_errors() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

contract X {
    function f() public {
        address payable target = payable(address(this));
        target./*caret*/
    }
}

contract Broken
"#,
        );

        let files = vec![(NormalizedPath::new("/external/Main.sol"), main_text)];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/external/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("balance"));
        assert!(labels.contains("transfer()"));
    }

    #[test]
    fn completions_recover_builtin_array_members_on_parse_errors() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

contract X {
    function f() public {
        uint256[] memory values;
        values./*caret*/
    }
}

contract Broken
"#,
        );

        let files = vec![(NormalizedPath::new("/external/Main.sol"), main_text)];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/external/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("length"));
    }

    #[test]
    fn completions_recover_builtin_address_nonpayable_excludes_transfer() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

contract X {
    function f() public {
        address target = address(this);
        target./*caret*/
    }
}

contract Broken
"#,
        );

        let files = vec![(NormalizedPath::new("/external/Main.sol"), main_text)];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/external/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("balance"));
        assert!(!labels.contains("transfer()"));
        assert!(!labels.contains("send"));
    }

    #[test]
    fn completions_recover_builtin_array_storage_includes_push_pop() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

contract X {
    uint256[] store;

    function f() public {
        uint256[] storage values = store;
        values./*caret*/
    }
}

contract Broken
"#,
        );

        let files = vec![(NormalizedPath::new("/external/Main.sol"), main_text)];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/external/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("length"));
        assert!(labels.contains("push()"));
        assert!(labels.contains("pop()"));
    }

    #[test]
    fn completions_recover_builtin_array_memory_excludes_push_pop() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

contract X {
    function f() public {
        uint256[] memory values;
        values./*caret*/
    }
}

contract Broken
"#,
        );

        let files = vec![(NormalizedPath::new("/external/Main.sol"), main_text)];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/external/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("length"));
        assert!(!labels.contains("push()"));
        assert!(!labels.contains("pop()"));
    }

    #[test]
    fn completions_recover_builtin_bytes_storage_includes_push_pop() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

contract X {
    bytes store;

    function f() public {
        bytes storage data = store;
        data./*caret*/
    }
}

contract Broken
"#,
        );

        let files = vec![(NormalizedPath::new("/external/Main.sol"), main_text)];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/external/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("length"));
        assert!(labels.contains("push()"));
        assert!(labels.contains("pop()"));
    }

    #[test]
    fn completions_recover_builtin_bytes_memory_excludes_push_pop() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

contract X {
    function f() public {
        bytes memory data = new bytes(4);
        data./*caret*/
    }
}

contract Broken
"#,
        );

        let files = vec![(NormalizedPath::new("/external/Main.sol"), main_text)];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/external/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("length"));
        assert!(!labels.contains("push()"));
        assert!(!labels.contains("pop()"));
    }

    #[test]
    fn completions_recover_builtin_string_length() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

contract X {
    function f() public {
        string memory name = "hi";
        name./*caret*/
    }
}

contract Broken
"#,
        );

        let files = vec![(NormalizedPath::new("/external/Main.sol"), main_text)];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/external/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("length"));
    }

    #[test]
    fn completions_do_not_use_contract_members_for_builtin_locals() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import {A} from "./A.sol";

contract X {
    function f() public {
        uint256 A = 0;
        A./*caret*/
    }
}
"#,
        );

        let files = vec![
            (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
            (
                NormalizedPath::new("/workspace/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
        ];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(!labels.contains("ping()"));
    }

    #[test]
    fn completions_filter_non_public_instance_members() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import {A} from "./A.sol";

contract X {
    function f() public returns (uint256) {
        A a = new A();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
            (
                NormalizedPath::new("/workspace/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function pubFn() public {}
    function extFn() external {}
    function intFn() internal {}
    function privFn() private {}
    uint256 public value;
    uint256 internal hidden;
    uint256 public constant CONST = 1;
}
"#
                .to_string(),
            ),
        ];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("pubFn()"));
        assert!(labels.contains("extFn()"));
        assert!(labels.contains("value"));
        assert!(!labels.contains("intFn()"));
        assert!(!labels.contains("privFn()"));
        assert!(!labels.contains("hidden"));
        assert!(!labels.contains("CONST"));
    }

    #[test]
    fn completions_include_inherited_members_for_recovery() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import {A} from "./A.sol";

contract X {
    function f() public returns (uint256) {
        A a0 = new A();
        a0./*caret*/
    }
}
"#,
        );

        let files = vec![
            (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
            (
                NormalizedPath::new("/workspace/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract B {
    function b() public {}
}

contract A is B {
    function a() public {}
}
"#
                .to_string(),
            ),
        ];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("a()"));
        assert!(labels.contains("b()"));
    }

    #[test]
    fn completions_resolve_member_access_on_import_alias_locals() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import {A as NotA} from "./A.sol";

contract X {
    function f() public returns (uint256) {
        NotA a = new NotA();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (
                NormalizedPath::new("/workspace/src/Main.sol"),
                main_text.clone(),
            ),
            (
                NormalizedPath::new("/workspace/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
        ];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("ping()"));
    }

    #[test]
    fn completions_prefer_import_alias_over_global_contract_name() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import {A as B} from "./A.sol";

contract X {
    function f() public returns (uint256) {
        B a = new B();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
            (
                NormalizedPath::new("/workspace/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
            (
                NormalizedPath::new("/workspace/src/B.sol"),
                r#"
pragma solidity ^0.8.20;

contract B {
    function wrong() public {}
}
"#
                .to_string(),
            ),
        ];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("ping()"));
        assert!(!labels.contains("wrong"));
    }

    #[test]
    fn completions_ignore_local_decl_after_caret() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import {A} from "./A.sol";

contract X {
    function f() public returns (uint256) {
        a./*caret*/
        A a = new A();
    }
}
"#,
        );

        let files = vec![
            (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
            (
                NormalizedPath::new("/workspace/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
        ];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(!labels.contains("ping()"));
    }

    #[test]
    fn completions_resolve_member_access_on_source_alias_locals() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import "./A.sol" as Lib;

contract X {
    function f() public returns (uint256) {
        Lib.A a = new Lib.A();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (
                NormalizedPath::new("/workspace/src/Main.sol"),
                main_text.clone(),
            ),
            (
                NormalizedPath::new("/workspace/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
            (
                NormalizedPath::new("/workspace/src/B.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function wrong() public {}
}
"#
                .to_string(),
            ),
        ];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("ping()"));
        assert!(!labels.contains("wrong"));
    }

    #[test]
    fn completions_resolve_member_access_on_glob_alias_locals() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import * as Lib from "./A.sol";

contract X {
    function f() public returns (uint256) {
        Lib.A a = new Lib.A();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
            (
                NormalizedPath::new("/workspace/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
            (
                NormalizedPath::new("/workspace/src/B.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function wrong() public {}
}
"#
                .to_string(),
            ),
        ];
        let (db, project_id, snapshot) = setup_db(files, vec![]);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("ping()"));
        assert!(!labels.contains("wrong"));
    }

    #[test]
    fn completions_resolve_member_access_on_source_alias_with_remapping() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import "lib/A.sol" as Lib;

contract X {
    function f() public returns (uint256) {
        Lib.A a = new Lib.A();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
            (
                NormalizedPath::new("/workspace/lib/forge-std/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
            (
                NormalizedPath::new("/workspace/src/B.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function wrong() public {}
}
"#
                .to_string(),
            ),
        ];
        let remappings = vec![Remapping::new("lib/", "lib/forge-std/")];
        let (db, project_id, snapshot) = setup_db(files, remappings);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("ping()"));
        assert!(!labels.contains("wrong"));
    }

    #[test]
    fn completions_resolve_member_access_on_glob_alias_with_remapping() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import * as Lib from "lib/A.sol";

contract X {
    function f() public returns (uint256) {
        Lib.A a = new Lib.A();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
            (
                NormalizedPath::new("/workspace/lib/forge-std/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
            (
                NormalizedPath::new("/workspace/src/B.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function wrong() public {}
}
"#
                .to_string(),
            ),
        ];
        let remappings = vec![Remapping::new("lib/", "lib/forge-std/")];
        let (db, project_id, snapshot) = setup_db(files, remappings);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("ping()"));
        assert!(!labels.contains("wrong"));
    }

    #[test]
    fn completions_resolve_member_access_on_context_specific_remap() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import "dep/A.sol" as Lib;

contract X {
    function f() public returns (uint256) {
        Lib.A a = new Lib.A();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (
                NormalizedPath::new("/workspace/src/Main.sol"),
                main_text.clone(),
            ),
            (
                NormalizedPath::new("/workspace/lib/src/dep/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
            (
                NormalizedPath::new("/workspace/lib/default/dep/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function wrong() public {}
}
"#
                .to_string(),
            ),
        ];
        let remappings = vec![
            Remapping::new("dep/", "lib/default/dep/"),
            Remapping::new("dep/", "lib/src/dep/").with_context("src"),
        ];
        let (db, project_id, snapshot) = setup_db(files, remappings);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("ping()"));
        assert!(!labels.contains("wrong"));
    }

    #[test]
    fn completions_resolve_member_access_on_nested_lib_remap() {
        let (main_text, offset) = extract_offset(
            r#"
pragma solidity ^0.8.20;

import * as Lib from "bar/A.sol";

contract X {
    function f() public returns (uint256) {
        Lib.A a = new Lib.A();
        a./*caret*/
    }
}
"#,
        );

        let files = vec![
            (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
            (
                NormalizedPath::new("/workspace/lib/foo/lib/bar/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function ping() public {}
}
"#
                .to_string(),
            ),
            (
                NormalizedPath::new("/workspace/lib/bar/src/A.sol"),
                r#"
pragma solidity ^0.8.20;

contract A {
    function wrong() public {}
}
"#
                .to_string(),
            ),
        ];
        let remappings = vec![Remapping::new("bar/", "lib/foo/lib/bar/src/")];
        let (db, project_id, snapshot) = setup_db(files, remappings);
        let main_id = snapshot
            .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
            .expect("main file id");

        let items = completions(&db, project_id, main_id, offset);
        let labels = labels(&items);
        assert!(labels.contains("ping()"));
        assert!(!labels.contains("wrong"));
    }
}
