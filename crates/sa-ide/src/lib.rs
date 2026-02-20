use std::sync::Arc;

use forge_fmt::FormatterConfig;
use sa_base_db::{Database, FileId, LanguageKind, ProjectId};
use sa_config::ResolvedFoundryConfig;
use sa_hir::{Definition, DefinitionLocation, Semantics};
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryResolver, FoundryWorkspace};
use sa_span::{TextRange, TextSize};
use sa_vfs::VfsSnapshot;
use tracing::debug;

mod code_actions;
mod completion;
mod formatting;
mod hover;
mod rename;
mod signature_help;
mod symbols;
mod syntax_outline;
mod syntax_utils;

pub use code_actions::{CodeAction, CodeActionDiagnostic, CodeActionKind};
pub use completion::{CompletionInsertTextFormat, CompletionItem, CompletionItemKind};
pub use hover::HoverResult;
pub use sa_ide_assists::{SourceChange, SourceFileEdit, TextEdit};
pub use sa_ide_db::Reference;
pub use signature_help::{ParameterInformation, SignatureHelp, SignatureInformation};
pub use symbols::WorkspaceSymbol;
pub use syntax_outline::{SymbolInfo, SymbolKind};
pub use syntax_utils::docs_for_item;

#[derive(Default)]
pub struct AnalysisChange {
    vfs: Option<VfsSnapshot>,
    workspace: Option<FoundryWorkspace>,
    config: Option<ResolvedFoundryConfig>,
}

impl AnalysisChange {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_vfs(&mut self, vfs: VfsSnapshot) {
        self.vfs = Some(vfs);
    }

    pub fn set_workspace(&mut self, workspace: FoundryWorkspace) {
        self.workspace = Some(workspace);
    }

    pub fn set_config(&mut self, config: ResolvedFoundryConfig) {
        self.config = Some(config);
    }
}

pub struct AnalysisHost {
    db: Database,
    project_id: ProjectId,
}

impl AnalysisHost {
    pub fn new() -> Self {
        Self {
            db: Database::default(),
            project_id: ProjectId::from_raw(0),
        }
    }

    pub fn apply_change(&mut self, change: AnalysisChange) {
        if let Some(vfs) = change.vfs {
            for (file_id, _) in vfs.iter() {
                let text = vfs.file_text(file_id);
                let path = vfs.path(file_id);
                if let (Some(text), Some(path)) = (text, path) {
                    let version = vfs.file_version(file_id).unwrap_or(0);
                    self.db.set_file(
                        file_id,
                        Arc::from(text),
                        version,
                        LanguageKind::Solidity,
                        Arc::new(path.clone()),
                    );
                } else {
                    debug!(
                        ?file_id,
                        has_text = text.is_some(),
                        has_path = path.is_some(),
                        "skipping vfs entry missing text or path"
                    );
                }
            }
        }

        if let Some(config) = change.config {
            self.db.set_project_input(self.project_id, Arc::new(config));
        } else if let Some(workspace) = change.workspace {
            let active_profile = FoundryProfile::new("default");
            let config = ResolvedFoundryConfig::new(workspace, active_profile);
            self.db.set_project_input(self.project_id, Arc::new(config));
        }
    }

    pub fn snapshot(&self) -> Analysis {
        Analysis {
            db: self.db.clone(),
            project_id: self.project_id,
        }
    }
}

impl Default for AnalysisHost {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Analysis {
    db: Database,
    project_id: ProjectId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationTarget {
    pub file_id: FileId,
    pub range: TextRange,
    pub origin_range: Option<TextRange>,
}

impl Analysis {
    pub fn file_text(&self, file_id: FileId) -> Arc<str> {
        self.db.file_input(file_id).text(&self.db).clone()
    }

    pub fn file_path(&self, file_id: FileId) -> Arc<NormalizedPath> {
        self.db.file_path(file_id)
    }

    pub fn file_version(&self, file_id: FileId) -> u32 {
        self.db.file_input(file_id).version(&self.db)
    }

    pub fn file_kind(&self, file_id: FileId) -> LanguageKind {
        self.db.file_input(file_id).kind(&self.db)
    }

    pub fn workspace(&self) -> Arc<FoundryWorkspace> {
        self.db
            .project_input(self.project_id)
            .workspace(&self.db)
            .clone()
    }

    pub fn config(&self) -> Arc<ResolvedFoundryConfig> {
        self.db
            .project_input(self.project_id)
            .config(&self.db)
            .clone()
    }

    fn workspace_opt(&self) -> Option<Arc<FoundryWorkspace>> {
        self.db
            .project_input_opt(self.project_id)
            .map(|input| input.workspace(&self.db).clone())
    }

    pub fn syntax_outline(&self, file_id: FileId) -> Vec<SymbolInfo> {
        let text = self.file_text(file_id);
        let parse = sa_syntax::parse_file(&text);
        syntax_outline::syntax_outline(&parse)
    }

    pub fn goto_definition(&self, file_id: FileId, offset: TextSize) -> Option<NavigationTarget> {
        if let Some(target) = self.import_path_definition(file_id, offset) {
            return Some(target);
        }
        let semantics = Semantics::new(&self.db, self.project_id);
        if let Some(local) = semantics.resolve_local(file_id, offset) {
            return Some(NavigationTarget {
                file_id,
                range: local.range(),
                origin_range: None,
            });
        }
        self.workspace_opt()?;
        let DefinitionLocation {
            file_id,
            range,
            origin_range,
        } = semantics.source_to_def_location(file_id, offset)?;
        Some(NavigationTarget {
            file_id,
            range,
            origin_range,
        })
    }

    pub fn find_references(&self, file_id: FileId, offset: TextSize) -> Vec<Reference> {
        if self.workspace_opt().is_none() {
            return Vec::new();
        }
        let semantics = Semantics::new(&self.db, self.project_id);
        let Some(definition) = semantics.resolve_definition(file_id, offset) else {
            return Vec::new();
        };
        match definition {
            Definition::Global(def_id) => {
                sa_ide_db::find_references(&self.db, self.project_id, def_id)
            }
            Definition::Local(local) => sa_hir::local_references(&self.db, file_id, &local)
                .into_iter()
                .map(|range| Reference::new(file_id, range))
                .collect(),
        }
    }

    pub fn hover(&self, file_id: FileId, offset: TextSize) -> Option<HoverResult> {
        self.workspace_opt()?;
        hover::hover(&self.db, self.project_id, file_id, offset)
    }

    pub fn signature_help(&self, file_id: FileId, offset: TextSize) -> Option<SignatureHelp> {
        self.workspace_opt()?;
        signature_help::signature_help(&self.db, self.project_id, file_id, offset)
    }

    pub fn completions(&self, file_id: FileId, offset: TextSize) -> Vec<CompletionItem> {
        if self.workspace_opt().is_none() {
            return Vec::new();
        }
        completion::completions(&self.db, self.project_id, file_id, offset)
    }

    pub fn format_document(&self, file_id: FileId, config: &FormatterConfig) -> Option<TextEdit> {
        let text = self.file_text(file_id);
        formatting::format_edit(text.as_ref(), config)
    }

    pub fn code_actions(
        &self,
        file_id: FileId,
        diagnostics: &[CodeActionDiagnostic],
    ) -> Vec<CodeAction> {
        let text = self.file_text(file_id);
        code_actions::code_actions(file_id, text.as_ref(), diagnostics)
    }

    pub fn rename(
        &self,
        file_id: FileId,
        offset: TextSize,
        new_name: &str,
    ) -> Option<SourceChange> {
        self.workspace_opt()?;
        rename::rename(&self.db, self.project_id, file_id, offset, new_name)
    }

    pub fn document_symbols(&self, file_id: FileId) -> Vec<SymbolInfo> {
        if self.workspace_opt().is_some()
            && let Some(symbols) = symbols::document_symbols(&self.db, self.project_id, file_id)
        {
            return symbols;
        }
        self.syntax_outline(file_id)
    }

    pub fn workspace_symbols(&self, query: &str) -> Vec<WorkspaceSymbol> {
        if self.workspace_opt().is_none() {
            return Vec::new();
        }
        symbols::workspace_symbols(&self.db, self.project_id, query)
    }

    fn import_path_definition(
        &self,
        file_id: FileId,
        offset: TextSize,
    ) -> Option<NavigationTarget> {
        let text = self.file_text(file_id);
        let parse = sa_syntax::parse_file(&text);
        let current_path = self.db.file_path(file_id);

        let project = self.db.project_input_opt(self.project_id)?;
        let workspace = project.workspace(&self.db).clone();
        let remappings = project.config(&self.db).active_profile().remappings();
        let resolver = FoundryResolver::new(&workspace, remappings).ok()?;

        for (_, directive) in parse.tree().imports() {
            let Some(range) = parse.span_to_text_range(directive.path.span) else {
                continue;
            };
            if !(range.start() <= offset && offset < range.end()) {
                continue;
            }
            let import_path = parse.with_session(|| directive.path.value.as_str().to_string());
            let resolved = resolver.resolve_import_path(&current_path, &import_path)?;
            let target_file_id = self.file_id_for_path(&resolved)?;
            return Some(NavigationTarget {
                file_id: target_file_id,
                range: TextRange::empty(TextSize::from(0)),
                origin_range: Some(range),
            });
        }

        None
    }

    fn file_id_for_path(&self, path: &NormalizedPath) -> Option<FileId> {
        self.db.file_id_for_path(path)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sa_base_db::LanguageKind;
    use sa_config::ResolvedFoundryConfig;
    use sa_paths::NormalizedPath;
    use sa_project_model::{FoundryProfile, FoundryWorkspace};
    use sa_vfs::{Vfs, VfsChange};

    use super::{AnalysisChange, AnalysisHost};

    #[test]
    fn analysis_host_accepts_vfs_and_workspace_inputs() {
        let mut vfs = Vfs::default();
        let path = NormalizedPath::new("/workspace/src/Main.sol");
        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from("contract Main {}"),
        });
        let snapshot = vfs.snapshot();
        let file_id = snapshot.file_id(&path).expect("file id");

        let root = NormalizedPath::new("/workspace");
        let default_profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root);
        let config = ResolvedFoundryConfig::new(workspace.clone(), default_profile);

        let mut host = AnalysisHost::new();
        let mut change = AnalysisChange::new();
        change.set_vfs(snapshot);
        change.set_config(config);
        host.apply_change(change);

        let analysis = host.snapshot();
        assert_eq!(analysis.file_text(file_id).as_ref(), "contract Main {}");
        assert_eq!(analysis.file_version(file_id), 0);
        assert_eq!(analysis.file_kind(file_id), LanguageKind::Solidity);
    }
}
