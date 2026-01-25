use heck::{AsLowerCamelCase, AsPascalCase};
use sa_base_db::FileId;
use sa_ide_assists::{LintFixKind, SourceChange, TextEdit, lint_fix};
use sa_span::TextRange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeActionDiagnostic {
    pub range: TextRange,
    pub code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeActionKind {
    QuickFix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeAction {
    pub title: String,
    pub kind: CodeActionKind,
    pub edit: SourceChange,
}

pub fn code_actions(
    file_id: FileId,
    text: &str,
    diagnostics: &[CodeActionDiagnostic],
) -> Vec<CodeAction> {
    let mut actions = Vec::new();

    for diagnostic in diagnostics {
        let Some(fix) = lint_fix(&diagnostic.code) else {
            continue;
        };
        let Some(replacement) = replacement_for_fix(fix.kind, text, diagnostic.range) else {
            continue;
        };

        let mut change = SourceChange::default();
        change.insert_edit(
            file_id,
            TextEdit {
                range: diagnostic.range,
                new_text: replacement,
            },
        );
        change.normalize();

        actions.push(CodeAction {
            title: fix.title.to_string(),
            kind: CodeActionKind::QuickFix,
            edit: change,
        });
    }

    actions
}

fn replacement_for_fix(kind: LintFixKind, text: &str, range: TextRange) -> Option<String> {
    let (start, end) = range_bounds(range, text)?;
    let name = text.get(start..end)?;

    let replacement = match kind {
        LintFixKind::MixedCaseVariable | LintFixKind::MixedCaseFunction => to_mixed_case(name),
        LintFixKind::PascalCaseStruct => AsPascalCase(name).to_string(),
    };

    if replacement == name {
        None
    } else {
        Some(replacement)
    }
}

fn range_bounds(range: TextRange, text: &str) -> Option<(usize, usize)> {
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    if start > end || end > text.len() {
        return None;
    }
    Some((start, end))
}

fn to_mixed_case(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut prefix_len = 0;
    while prefix_len < bytes.len() && bytes[prefix_len] == b'_' {
        prefix_len += 1;
    }

    let mut suffix_len = 0;
    while suffix_len < bytes.len() - prefix_len && bytes[bytes.len() - 1 - suffix_len] == b'_' {
        suffix_len += 1;
    }

    let core_end = bytes.len().saturating_sub(suffix_len);
    let core = &name[prefix_len..core_end];
    let converted = AsLowerCamelCase(core).to_string();

    let mut result = String::with_capacity(name.len() + converted.len());
    result.push_str(&name[..prefix_len]);
    result.push_str(&converted);
    result.push_str(&name[core_end..]);
    result
}
