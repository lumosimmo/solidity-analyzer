use sa_base_db::FileId;
use sa_span::TextRange;

mod lint_fixes;

pub use lint_fixes::{LintFix, LintFixKind, is_fixable_lint, lint_fix};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: TextRange,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileEdit {
    pub file_id: FileId,
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceChange {
    edits: Vec<SourceFileEdit>,
}

impl SourceChange {
    pub fn edits(&self) -> &[SourceFileEdit] {
        &self.edits
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    pub fn insert_edit(&mut self, file_id: FileId, edit: TextEdit) {
        if let Some(entry) = self.edits.iter_mut().find(|entry| entry.file_id == file_id) {
            entry.edits.push(edit);
            return;
        }

        self.edits.push(SourceFileEdit {
            file_id,
            edits: vec![edit],
        });
    }

    pub fn normalize(&mut self) {
        self.edits.sort_by_key(|entry| entry.file_id);
        for entry in &mut self.edits {
            entry.edits.sort_by_key(|edit| edit.range.start());
        }
    }
}
