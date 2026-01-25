use sa_base_db::{FileId, ProjectId};
use sa_hir::HirDatabase;
use sa_span::{TextRange, TextSize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionItemKind,
    pub replacement_range: TextRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    sa_ide_completion::completions(db, project_id, file_id, offset)
        .into_iter()
        .map(CompletionItem::from)
        .collect()
}

impl From<sa_ide_completion::CompletionItem> for CompletionItem {
    fn from(item: sa_ide_completion::CompletionItem) -> Self {
        Self {
            label: item.label,
            kind: item.kind.into(),
            replacement_range: item.replacement_range,
        }
    }
}

impl From<sa_ide_completion::CompletionItemKind> for CompletionItemKind {
    fn from(kind: sa_ide_completion::CompletionItemKind) -> Self {
        match kind {
            sa_ide_completion::CompletionItemKind::Contract => Self::Contract,
            sa_ide_completion::CompletionItemKind::Function => Self::Function,
            sa_ide_completion::CompletionItemKind::Struct => Self::Struct,
            sa_ide_completion::CompletionItemKind::Enum => Self::Enum,
            sa_ide_completion::CompletionItemKind::Event => Self::Event,
            sa_ide_completion::CompletionItemKind::Error => Self::Error,
            sa_ide_completion::CompletionItemKind::Modifier => Self::Modifier,
            sa_ide_completion::CompletionItemKind::Variable => Self::Variable,
            sa_ide_completion::CompletionItemKind::Type => Self::Type,
            sa_ide_completion::CompletionItemKind::File => Self::File,
        }
    }
}
