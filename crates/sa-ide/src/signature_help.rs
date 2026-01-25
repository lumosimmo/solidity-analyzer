use sa_base_db::{FileId, ProjectId};
use sa_def::{DefEntry, DefKind};
use sa_hir::{HirDatabase, Semantics, lowered_program};
use sa_span::{TextSize, is_ident_byte};
use sa_syntax::{
    Parse,
    ast::{Item, ItemKind},
};

use crate::syntax_utils::{
    docs_for_item_with_inheritdoc, find_item_by_name_range, format_function_signature, format_param,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    pub signatures: Vec<SignatureInformation>,
    pub active_signature: Option<usize>,
    pub active_parameter: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInformation {
    pub label: String,
    pub documentation: Option<String>,
    pub parameters: Vec<ParameterInformation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterInformation {
    pub label: String,
}

pub fn signature_help(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    offset: TextSize,
) -> Option<SignatureHelp> {
    let text = db.file_input(file_id).text(db);
    let call = call_at_offset(text.as_ref(), offset)?;
    let program = lowered_program(db, project_id);

    // Find the container contract at the call site for scoped resolution
    let parse = sa_syntax::parse_file(text.as_ref());
    let container = find_container_at_offset(&parse, offset);

    // Try to resolve using source_to_def, then validate it matches the expected scope
    let entry = resolve_function_entry_via_semantics(
        db,
        project_id,
        file_id,
        &call,
        &program,
        container.as_deref(),
    )
    .or_else(|| {
        resolve_function_entry_by_name(&program, file_id, &call.name, container.as_deref())
    })?;

    let def_file_id = entry.location().file_id();
    let def_text = db.file_input(def_file_id).text(db);
    let parse = sa_syntax::parse_file(def_text.as_ref());
    let function_item = find_function_item(&parse, entry)?;
    let ItemKind::Function(function) = &function_item.kind else {
        return None;
    };

    let label = format_function_signature(&parse, def_text.as_ref(), function);
    let parameters = function
        .header
        .parameters
        .vars
        .iter()
        .map(|param| ParameterInformation {
            label: format_param(&parse, def_text.as_ref(), param),
        })
        .collect::<Vec<_>>();
    let documentation = docs_for_item_with_inheritdoc(
        db,
        project_id,
        def_file_id,
        &parse,
        function_item,
        entry.container(),
    );

    // Clamp active_parameter to valid range, or None if no parameters
    let active_parameter = if parameters.is_empty() {
        None
    } else {
        Some(call.active_parameter.min(parameters.len() - 1))
    };

    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation,
            parameters,
        }],
        active_signature: Some(0),
        active_parameter,
    })
}

struct CallContext {
    name: String,
    /// Offset pointing into the function name identifier (for source_to_def resolution)
    name_offset: TextSize,
    active_parameter: usize,
}

fn call_at_offset(text: &str, offset: TextSize) -> Option<CallContext> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let idx = usize::from(offset).min(bytes.len());
    let open_paren = find_open_paren(bytes, idx)?;
    let (start, end) = ident_before(bytes, open_paren)?;
    let name = std::str::from_utf8(&bytes[start..end]).ok()?.to_string();
    let active_parameter = count_commas(bytes, open_paren + 1, idx);
    // Use the start of the identifier for source_to_def resolution
    let name_offset = TextSize::from(start as u32);
    Some(CallContext {
        name,
        name_offset,
        active_parameter,
    })
}

/// Finds the position of the opening parenthesis for a function call.
///
/// # Known Limitation
///
/// This function scans bytes for `(` and `)` without distinguishing whether
/// they appear inside string literals or comments. This could lead to incorrect
/// matching in edge cases like: `foo("some (text)", bar(`. For typical signature
/// help scenarios (where the cursor is actively positioned in a function call),
/// this is unlikely to cause problems. However, if signature help were invoked
/// at arbitrary cursor positions within string literals, incorrect parenthesis
/// matching could occur.
fn find_open_paren(bytes: &[u8], mut idx: usize) -> Option<usize> {
    let mut depth = 0usize;
    while idx > 0 {
        idx -= 1;
        match bytes[idx] {
            b')' => depth += 1,
            b'(' => {
                if depth == 0 {
                    return Some(idx);
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    None
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

/// Counts commas at the top level (depth 0) of a byte slice.
///
/// Tracks nesting of `()`, `[]`, and `{}` to correctly handle arrays, struct
/// literals, and nested function calls within arguments.
///
/// # Known Limitation
///
/// This function does not skip string literals or comments. It assumes the
/// provided byte slice does not contain commas inside strings or comments that
/// would be incorrectly counted. The caller (e.g., `find_open_paren` followed
/// by `call_at_offset`) is responsible for ensuring the slice represents a
/// valid function argument context where this simplification is acceptable.
fn count_commas(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut count = 0usize;
    for &byte in &bytes[start..end] {
        match byte {
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'{' => brace_depth += 1,
            b'}' => brace_depth = brace_depth.saturating_sub(1),
            b',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

/// Resolves the function at the call site using Semantics::source_to_def.
///
/// This provides accurate resolution for cross-contract calls and handles
/// cases where multiple functions with the same name exist in different contracts.
/// If a container is specified, validates that the resolved function belongs to it.
fn resolve_function_entry_via_semantics<'a>(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    call: &CallContext,
    program: &'a sa_hir::HirProgram,
    expected_container: Option<&str>,
) -> Option<&'a DefEntry> {
    let semantics = Semantics::new(db, project_id);
    let def_id = semantics.source_to_def(file_id, call.name_offset)?;
    let entry = program.def_map().entry(def_id)?;

    // Only return if it's a function (not a contract or other definition)
    if entry.kind() != DefKind::Function {
        return None;
    }

    // If we have an expected container, validate the function belongs to it
    // (for unqualified calls within a contract, the function should be in the same contract)
    if expected_container.is_some() && entry.container() != expected_container {
        return None;
    }

    Some(entry)
}

/// Fallback resolution by name when source_to_def doesn't resolve.
///
/// Prefers functions in the same container (contract), then same file, then any match.
fn resolve_function_entry_by_name<'a>(
    program: &'a sa_hir::HirProgram,
    file_id: FileId,
    name: &str,
    container: Option<&str>,
) -> Option<&'a DefEntry> {
    let entries = program.def_map().entries_by_name(DefKind::Function, name)?;

    // First, try to find a function in the same container (contract)
    if let Some(entry) = container.and_then(|container_name| {
        entries
            .iter()
            .find(|e| e.location().file_id() == file_id && e.container() == Some(container_name))
            .copied()
    }) {
        return Some(entry);
    }

    // Then, prefer any function in the same file
    if let Some(entry) = entries.iter().find(|e| e.location().file_id() == file_id) {
        return Some(*entry);
    }

    // Finally, fall back to any matching function (deterministic by min location)
    entries
        .into_iter()
        .min_by_key(|e| (e.location().file_id(), e.location().range().start()))
}

/// Finds the name of the contract containing the given offset.
fn find_container_at_offset(parse: &Parse, offset: TextSize) -> Option<String> {
    parse.with_session(|| {
        for item in parse.tree().items.iter() {
            if let ItemKind::Contract(contract) = &item.kind {
                let span_range = parse.span_to_text_range(item.span)?;
                if span_range.start() <= offset && offset < span_range.end() {
                    return Some(contract.name.to_string());
                }
            }
        }
        None
    })
}

fn find_function_item<'a>(parse: &'a Parse, entry: &DefEntry) -> Option<&'a Item<'static>> {
    let item = find_item_by_name_range(parse, entry.container(), entry.location().range())?;
    matches!(item.kind, ItemKind::Function(_)).then_some(item)
}
