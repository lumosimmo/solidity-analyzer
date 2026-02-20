use std::collections::{HashMap, HashSet};
use std::path::Path;

use sa_base_db::{FileId, FileInput, ProjectId, ProjectInput};
use sa_def::{DefDatabase, DefEntry, DefId, DefKind, DefMap};
use sa_paths::NormalizedPath;
use sa_project_model::{
    FoundryResolver, FoundryWorkspace, Remapping, resolve_import_path_with_resolver,
};
use sa_sema::{ResolveOutcome, ResolvedSymbol, ResolvedSymbolKind, SemaDatabase};
use sa_span::{TextRange, TextSize, is_ident_byte};
use sa_syntax::ast::ItemKind;
use sa_syntax::tokens::IdentRangeCollector;
use sa_syntax::{Parse, ParsedImport, ParsedImportItems};

mod locals;

pub use locals::{LocalDef, LocalDefKind, LocalScopes, local_references, local_scopes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Definition {
    Global(DefId),
    Local(LocalDef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionLocation {
    pub file_id: FileId,
    pub range: TextRange,
    pub origin_range: Option<TextRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleDefinition {
    name: String,
    kind: DefKind,
}

impl VisibleDefinition {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> DefKind {
        self.kind
    }
}

#[salsa::db]
pub trait HirDatabase: SemaDatabase {}

#[salsa::db]
impl HirDatabase for sa_base_db::Database {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFile {
    imports: Vec<ParsedImport>,
}

impl ParsedFile {
    pub fn new(imports: Vec<ParsedImport>) -> Self {
        Self { imports }
    }

    fn imports(&self) -> &[ParsedImport] {
        &self.imports
    }
}

unsafe impl salsa::Update for ParsedFile {
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
pub fn parse_file(db: &dyn HirDatabase, file: FileInput) -> ParsedFile {
    let text = file.text(db);
    let imports = sa_syntax::parse_imports_with_items(text);
    ParsedFile::new(imports)
}

pub fn parse(db: &dyn HirDatabase, file_id: FileId) -> ParsedFile {
    parse_file(db, db.file_input(file_id)).clone()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirProgram {
    defs: DefMap,
    files: HashMap<FileId, HirFile>,
}

impl HirProgram {
    pub fn file_ids(&self) -> impl Iterator<Item = FileId> + '_ {
        self.files.keys().copied()
    }

    pub fn def_map(&self) -> &DefMap {
        &self.defs
    }

    pub fn visible_definitions_in_file(&self, file_id: FileId) -> Vec<VisibleDefinition> {
        let mut defs = Vec::new();
        let mut seen = HashSet::new();
        let mut visited = HashSet::new();
        self.collect_visible_definitions(file_id, &mut defs, &mut seen, &mut visited);
        defs
    }

    fn collect_visible_definitions(
        &self,
        file_id: FileId,
        defs: &mut Vec<VisibleDefinition>,
        seen: &mut HashSet<(String, DefKind)>,
        visited: &mut HashSet<FileId>,
    ) {
        if !visited.insert(file_id) {
            return;
        }
        self.push_top_level_defs_in_file(file_id, defs, seen);

        let Some(file) = self.files.get(&file_id) else {
            return;
        };
        for import in &file.imports {
            let Some(imported_id) = import.file_id else {
                continue;
            };
            match &import.items {
                ParsedImportItems::Plain => {
                    self.collect_visible_definitions(imported_id, defs, seen, visited);
                }
                ParsedImportItems::Aliases(aliases) => {
                    for alias in aliases {
                        let entries = self.exported_entries_for_name(imported_id, &alias.name);
                        for entry in entries {
                            if entry.container().is_some() {
                                continue;
                            }
                            self.push_visible_definition(
                                alias.local_name().to_string(),
                                entry.kind(),
                                defs,
                                seen,
                            );
                        }
                    }
                }
                ParsedImportItems::SourceAlias(alias) | ParsedImportItems::Glob(alias) => {
                    self.push_visible_definition(alias.clone(), DefKind::Udvt, defs, seen);
                }
            }
        }
    }

    pub fn contract_member_definitions_in_file(
        &self,
        file_id: FileId,
        contract_name: &str,
    ) -> Vec<VisibleDefinition> {
        let mut defs = Vec::new();
        let mut seen = HashSet::new();
        for entry in self.defs.entries() {
            if entry.location().file_id() != file_id {
                continue;
            }
            if entry.container() != Some(contract_name) {
                continue;
            }
            self.push_visible_definition(
                entry.location().name().to_string(),
                entry.kind(),
                &mut defs,
                &mut seen,
            );
        }
        defs
    }

    fn push_top_level_defs_in_file(
        &self,
        file_id: FileId,
        defs: &mut Vec<VisibleDefinition>,
        seen: &mut HashSet<(String, DefKind)>,
    ) {
        for entry in self.defs.entries() {
            if entry.location().file_id() != file_id {
                continue;
            }
            if entry.container().is_some() {
                continue;
            }
            self.push_visible_definition(
                entry.location().name().to_string(),
                entry.kind(),
                defs,
                seen,
            );
        }
    }

    fn push_visible_definition(
        &self,
        name: String,
        kind: DefKind,
        defs: &mut Vec<VisibleDefinition>,
        seen: &mut HashSet<(String, DefKind)>,
    ) {
        if seen.insert((name.clone(), kind)) {
            defs.push(VisibleDefinition { name, kind });
        }
    }

    fn exported_entries_for_name<'a>(&'a self, file_id: FileId, name: &str) -> Vec<&'a DefEntry> {
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        self.collect_exported_entries(file_id, name, &mut visited, &mut entries);
        entries
    }

    fn exported_names_for_definition(
        &self,
        file_id: FileId,
        target_file_id: FileId,
        target_name: &str,
    ) -> Vec<String> {
        let mut visible = Vec::new();
        let mut seen = HashSet::new();
        let mut visited = HashSet::new();
        self.collect_visible_definitions(file_id, &mut visible, &mut seen, &mut visited);

        visible
            .into_iter()
            .filter_map(|def| {
                let entries = self.exported_entries_for_name(file_id, def.name());
                let matches_target = entries.iter().any(|entry| {
                    entry.location().file_id() == target_file_id
                        && entry.location().name() == target_name
                });
                matches_target.then(|| def.name().to_string())
            })
            .collect()
    }

    fn collect_exported_entries<'a>(
        &'a self,
        file_id: FileId,
        name: &str,
        visited: &mut HashSet<FileId>,
        entries: &mut Vec<&'a DefEntry>,
    ) {
        if !visited.insert(file_id) {
            return;
        }

        entries.extend(self.defs.entries_by_name_in_file(file_id, name));

        let Some(file) = self.files.get(&file_id) else {
            return;
        };
        for import in &file.imports {
            let Some(imported_id) = import.file_id else {
                continue;
            };
            match &import.items {
                ParsedImportItems::Plain => {
                    self.collect_exported_entries(imported_id, name, visited, entries);
                }
                ParsedImportItems::Aliases(aliases) => {
                    for alias in aliases {
                        if alias.local_name() != name {
                            continue;
                        }
                        self.collect_exported_entries(imported_id, &alias.name, visited, entries);
                    }
                }
                ParsedImportItems::SourceAlias(_) | ParsedImportItems::Glob(_) => {}
            }
        }
    }

    pub fn resolve_contract(&self, file_id: FileId, name: &str) -> Option<DefId> {
        self.resolve_in_file(file_id, DefKind::Contract, name)
    }

    pub fn resolve_symbol_kind(&self, file_id: FileId, kind: DefKind, name: &str) -> Option<DefId> {
        self.resolve_in_file(file_id, kind, name)
    }

    pub fn resolve_symbol_kind_candidates(
        &self,
        file_id: FileId,
        kind: DefKind,
        name: &str,
    ) -> Vec<DefId> {
        let mut candidates = HashSet::new();
        let mut visited = HashSet::new();
        self.collect_symbol_candidates(file_id, name, kind, &mut visited, &mut candidates);
        candidates.into_iter().collect()
    }

    fn collect_symbol_candidates(
        &self,
        file_id: FileId,
        name: &str,
        kind: DefKind,
        visited: &mut HashSet<FileId>,
        candidates: &mut HashSet<DefId>,
    ) {
        if !visited.insert(file_id) {
            return;
        }
        for entry in self.defs.entries_by_name_in_file(file_id, name) {
            if entry.kind() == kind {
                candidates.insert(entry.id());
            }
        }

        let Some(file) = self.files.get(&file_id) else {
            return;
        };
        for import in &file.imports {
            let Some(imported_id) = import.file_id else {
                continue;
            };
            match &import.items {
                ParsedImportItems::Plain => {
                    self.collect_symbol_candidates(imported_id, name, kind, visited, candidates);
                }
                ParsedImportItems::Aliases(aliases) => {
                    for alias in aliases {
                        if alias.local_name() != name {
                            continue;
                        }
                        self.collect_symbol_candidates(
                            imported_id,
                            &alias.name,
                            kind,
                            visited,
                            candidates,
                        );
                    }
                }
                ParsedImportItems::SourceAlias(_) | ParsedImportItems::Glob(_) => {}
            }
        }
    }

    pub fn resolve_qualified_symbol(
        &self,
        file_id: FileId,
        qualifier: &str,
        name: &str,
    ) -> Option<DefId> {
        let file = self.files.get(&file_id)?;
        let mut targets = HashSet::new();
        for import in &file.imports {
            if !import.matches_qualifier(qualifier) {
                continue;
            }
            let Some(imported_id) = import.file_id else {
                continue;
            };
            targets.insert(imported_id);
        }

        if targets.len() != 1 {
            return None;
        }
        let imported_id = *targets.iter().next()?;
        self.resolve_symbol_in_file_only(imported_id, name)
    }

    pub fn resolve_contract_qualified_symbol(
        &self,
        file_id: FileId,
        qualifier: &str,
        name: &str,
    ) -> Option<DefId> {
        let contract_id = self.resolve_contract(file_id, qualifier)?;
        let contract_entry = self.defs.entry(contract_id)?;
        let container = contract_entry.location().name();
        let contract_file_id = contract_entry.location().file_id();
        self.resolve_symbol_in_container(contract_file_id, container, name)
    }

    pub fn local_names_for_imported(
        &self,
        file_id: FileId,
        imported_file_id: FileId,
        imported_name: &str,
    ) -> Vec<String> {
        let mut names = Vec::new();
        let Some(file) = self.files.get(&file_id) else {
            return names;
        };
        for import in &file.imports {
            let Some(imported_id) = import.file_id else {
                continue;
            };
            let exported_names =
                self.exported_names_for_definition(imported_id, imported_file_id, imported_name);
            if exported_names.is_empty() {
                continue;
            }
            match &import.items {
                ParsedImportItems::Plain => {
                    names.extend(exported_names);
                }
                ParsedImportItems::Aliases(aliases) => {
                    for alias in aliases {
                        if exported_names.iter().any(|name| name == &alias.name) {
                            names.push(alias.local_name().to_string());
                        }
                    }
                }
                ParsedImportItems::SourceAlias(_) | ParsedImportItems::Glob(_) => {}
            }
        }
        names.sort();
        names.dedup();
        names
    }

    pub fn qualifier_names_for_imported(
        &self,
        file_id: FileId,
        imported_file_id: FileId,
    ) -> Vec<String> {
        let mut names = Vec::new();
        let Some(file) = self.files.get(&file_id) else {
            return names;
        };

        let mut alias_targets: HashMap<String, HashSet<FileId>> = HashMap::new();
        for import in &file.imports {
            let Some(alias) = import.qualifier_name() else {
                continue;
            };
            let Some(target_id) = import.file_id else {
                continue;
            };
            alias_targets
                .entry(alias.to_string())
                .or_default()
                .insert(target_id);
        }

        names.extend(alias_targets.into_iter().filter_map(|(alias, targets)| {
            if targets.len() == 1 && targets.contains(&imported_file_id) {
                Some(alias)
            } else {
                None
            }
        }));
        names.sort();
        names.dedup();
        names
    }

    pub fn resolve_symbol(&self, file_id: FileId, name: &str) -> Option<DefId> {
        self.resolve_in_file(file_id, DefKind::Contract, name)
            .or_else(|| self.resolve_in_file(file_id, DefKind::Struct, name))
            .or_else(|| self.resolve_in_file(file_id, DefKind::Enum, name))
            .or_else(|| self.resolve_in_file(file_id, DefKind::Function, name))
            .or_else(|| self.resolve_in_file(file_id, DefKind::Event, name))
            .or_else(|| self.resolve_in_file(file_id, DefKind::Error, name))
            .or_else(|| self.resolve_in_file(file_id, DefKind::Modifier, name))
            .or_else(|| self.resolve_in_file(file_id, DefKind::Variable, name))
            .or_else(|| self.resolve_in_file(file_id, DefKind::Udvt, name))
    }

    fn resolve_symbol_in_file_only(&self, file_id: FileId, name: &str) -> Option<DefId> {
        self.resolve_in_file_only(file_id, DefKind::Contract, name)
            .or_else(|| self.resolve_in_file_only(file_id, DefKind::Struct, name))
            .or_else(|| self.resolve_in_file_only(file_id, DefKind::Enum, name))
            .or_else(|| self.resolve_in_file_only(file_id, DefKind::Function, name))
            .or_else(|| self.resolve_in_file_only(file_id, DefKind::Event, name))
            .or_else(|| self.resolve_in_file_only(file_id, DefKind::Error, name))
            .or_else(|| self.resolve_in_file_only(file_id, DefKind::Modifier, name))
            .or_else(|| self.resolve_in_file_only(file_id, DefKind::Variable, name))
            .or_else(|| self.resolve_in_file_only(file_id, DefKind::Udvt, name))
    }

    fn resolve_symbol_in_container(
        &self,
        file_id: FileId,
        container: &str,
        name: &str,
    ) -> Option<DefId> {
        self.resolve_in_container(file_id, DefKind::Struct, name, container)
            .or_else(|| self.resolve_in_container(file_id, DefKind::Enum, name, container))
            .or_else(|| self.resolve_in_container(file_id, DefKind::Function, name, container))
            .or_else(|| self.resolve_in_container(file_id, DefKind::Event, name, container))
            .or_else(|| self.resolve_in_container(file_id, DefKind::Error, name, container))
            .or_else(|| self.resolve_in_container(file_id, DefKind::Modifier, name, container))
            .or_else(|| self.resolve_in_container(file_id, DefKind::Variable, name, container))
            .or_else(|| self.resolve_in_container(file_id, DefKind::Udvt, name, container))
    }

    fn resolve_in_file_only(&self, file_id: FileId, kind: DefKind, name: &str) -> Option<DefId> {
        self.defs
            .entries_by_name_in_file(file_id, name)
            .into_iter()
            .find(|entry| entry.kind() == kind)
            .map(|entry| entry.id())
    }

    fn resolve_in_container(
        &self,
        file_id: FileId,
        kind: DefKind,
        name: &str,
        container: &str,
    ) -> Option<DefId> {
        self.defs
            .entries_by_name_in_file(file_id, name)
            .into_iter()
            .find(|entry| entry.kind() == kind && entry.container() == Some(container))
            .map(|entry| entry.id())
    }

    fn resolve_in_file(&self, file_id: FileId, kind: DefKind, name: &str) -> Option<DefId> {
        let local = self
            .defs
            .entries_by_name_in_file(file_id, name)
            .into_iter()
            .find(|entry| entry.kind() == kind);
        if let Some(entry) = local {
            return Some(entry.id());
        }

        let file = self.files.get(&file_id)?;
        let mut visited = HashSet::new();
        visited.insert(file_id);
        for import in &file.imports {
            let Some(imported_id) = import.file_id else {
                continue;
            };
            let Some(imported_name) = import.imported_name_for_local(name) else {
                continue;
            };
            if let Some(entry) =
                self.resolve_in_exports(imported_id, &imported_name, kind, &mut visited)
            {
                return Some(entry);
            }
        }

        None
    }

    fn resolve_in_exports(
        &self,
        file_id: FileId,
        name: &str,
        kind: DefKind,
        visited: &mut HashSet<FileId>,
    ) -> Option<DefId> {
        if !visited.insert(file_id) {
            return None;
        }

        if let Some(entry) = self
            .defs
            .entries_by_name_in_file(file_id, name)
            .into_iter()
            .find(|entry| entry.kind() == kind)
        {
            return Some(entry.id());
        }

        let file = self.files.get(&file_id)?;
        for import in &file.imports {
            let Some(imported_id) = import.file_id else {
                continue;
            };
            match &import.items {
                ParsedImportItems::Plain => {
                    if let Some(entry) = self.resolve_in_exports(imported_id, name, kind, visited) {
                        return Some(entry);
                    }
                }
                ParsedImportItems::Aliases(aliases) => {
                    for alias in aliases {
                        if alias.local_name() != name {
                            continue;
                        }
                        if let Some(entry) =
                            self.resolve_in_exports(imported_id, &alias.name, kind, visited)
                        {
                            return Some(entry);
                        }
                    }
                }
                ParsedImportItems::SourceAlias(_) | ParsedImportItems::Glob(_) => {}
            }
        }

        None
    }
}

unsafe impl salsa::Update for HirProgram {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct HirFile {
    file_id: FileId,
    path: NormalizedPath,
    imports: Vec<Import>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Import {
    path: String,
    resolved_path: Option<NormalizedPath>,
    file_id: Option<FileId>,
    items: ParsedImportItems,
}

impl Import {
    fn qualifier_name(&self) -> Option<&str> {
        match &self.items {
            ParsedImportItems::SourceAlias(alias) | ParsedImportItems::Glob(alias) => {
                Some(alias.as_str())
            }
            ParsedImportItems::Plain | ParsedImportItems::Aliases(_) => None,
        }
    }

    fn matches_qualifier(&self, qualifier: &str) -> bool {
        self.qualifier_name() == Some(qualifier)
    }

    fn imported_name_for_local(&self, local: &str) -> Option<String> {
        match &self.items {
            ParsedImportItems::Plain => Some(local.to_string()),
            ParsedImportItems::Aliases(aliases) => aliases
                .iter()
                .find_map(|alias| (alias.local_name() == local).then(|| alias.name.clone())),
            ParsedImportItems::SourceAlias(_) | ParsedImportItems::Glob(_) => None,
        }
    }
}

#[salsa::tracked]
pub fn lowered_program_for_project(db: &dyn HirDatabase, project: ProjectInput) -> HirProgram {
    let workspace = project.workspace(db).clone();
    let remappings = project.config(db).active_profile().remappings();
    let mut def_db = DefDatabase::new();
    let mut file_texts = Vec::new();
    let mut files = HashMap::new();

    let mut path_to_file_id = HashMap::new();
    for file_id in db.file_ids() {
        let path = db.file_path(file_id);
        path_to_file_id.insert(path.as_str().to_string(), file_id);
    }

    for file_id in db.file_ids() {
        let text = db.file_input(file_id).text(db).clone();
        file_texts.push((file_id, text.clone()));

        let path = db.file_path(file_id);
        let parsed = parse_file(db, db.file_input(file_id));
        let imports = collect_imports(
            &workspace,
            remappings,
            &path,
            parsed,
            &path_to_file_id,
            &text,
        );
        files.insert(
            file_id,
            HirFile {
                file_id,
                path: (*path).clone(),
                imports,
            },
        );
    }

    let def_map = def_db.collect(
        file_texts
            .iter()
            .map(|(file_id, text)| (*file_id, text.as_ref())),
    );

    HirProgram {
        defs: def_map,
        files,
    }
}

pub fn lowered_program(db: &dyn HirDatabase, project_id: ProjectId) -> HirProgram {
    lowered_program_for_project(db, db.project_input(project_id))
}

pub fn visible_definitions(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
) -> Vec<VisibleDefinition> {
    let program = lowered_program(db, project_id);
    program.visible_definitions_in_file(file_id)
}

pub fn contract_member_definitions_at_offset(
    db: &dyn HirDatabase,
    project_id: ProjectId,
    file_id: FileId,
    offset: TextSize,
) -> Vec<VisibleDefinition> {
    let text = db.file_input(file_id).text(db);
    let parse = sa_syntax::parse_file(text.as_ref());
    let Some(contract_info) = contract_info_at_offset(&parse, text.as_ref(), offset) else {
        return Vec::new();
    };
    let program = lowered_program(db, project_id);
    contract_member_definitions_with_inheritance(db, &program, file_id, contract_info)
}

struct ContractInfo {
    name: String,
    bases: Vec<Vec<String>>,
}

fn contract_info_at_offset(parse: &Parse, text: &str, offset: TextSize) -> Option<ContractInfo> {
    contract_info_at_offset_from_parse(parse, offset)
        .or_else(|| contract_info_at_offset_fallback(text, offset))
}

fn contract_info_at_offset_from_parse(parse: &Parse, offset: TextSize) -> Option<ContractInfo> {
    parse.with_session(|| {
        for item in parse.tree().items.iter() {
            if let ItemKind::Contract(contract) = &item.kind {
                let range = parse.span_to_text_range(item.span)?;
                if range.start() <= offset && offset < range.end() {
                    return Some(ContractInfo {
                        name: contract.name.to_string(),
                        bases: contract_base_paths(contract),
                    });
                }
            }
        }
        None
    })
}

fn contract_info_at_offset_fallback(text: &str, offset: TextSize) -> Option<ContractInfo> {
    let offset = usize::from(offset).min(text.len());
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let mut scanner = FallbackScanner::new(text);
    let mut brace_depth = 0usize;

    while let Some(token) = scanner.next_token() {
        match token.kind {
            FallbackTokenKind::Punct('{') => brace_depth += 1,
            FallbackTokenKind::Punct('}') => brace_depth = brace_depth.saturating_sub(1),
            FallbackTokenKind::Ident(ref ident)
                if brace_depth == 0 && is_contract_keyword(ident) =>
            {
                if let Some(decl) = parse_contract_decl(&mut scanner, text, token.start) {
                    brace_depth = 0;
                    if offset >= decl.start && offset <= decl.close_brace {
                        return Some(ContractInfo {
                            name: decl.name,
                            bases: decl.bases,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    None
}

fn is_contract_keyword(ident: &str) -> bool {
    matches!(ident, "contract" | "library" | "interface")
}

struct ContractDecl {
    start: usize,
    open_brace: usize,
    close_brace: usize,
    name: String,
    bases: Vec<Vec<String>>,
}

fn parse_contract_decl(
    scanner: &mut FallbackScanner<'_>,
    text: &str,
    contract_start: usize,
) -> Option<ContractDecl> {
    let name_token = scanner.next_token()?;
    let name = match name_token.kind {
        FallbackTokenKind::Ident(ident) => ident,
        _ => return None,
    };

    let mut is_end: Option<usize> = None;
    let mut open_brace: Option<usize> = None;

    while let Some(token) = scanner.next_token() {
        match token.kind {
            FallbackTokenKind::Ident(ident) if ident == "is" => {
                is_end = Some(token.end);
            }
            FallbackTokenKind::Punct('{') => {
                open_brace = Some(token.start);
                break;
            }
            _ => {}
        }
    }

    let open_brace = open_brace?;
    let close_brace = matching_close_brace(text, open_brace).unwrap_or(text.len());
    scanner.idx = close_brace.saturating_add(1);

    let bases = is_end
        .map(|start| parse_base_paths_in_range(text, start, open_brace))
        .unwrap_or_default();

    Some(ContractDecl {
        start: contract_start,
        open_brace,
        close_brace,
        name,
        bases,
    })
}

fn matching_close_brace(text: &str, open_brace: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if open_brace >= bytes.len() {
        return None;
    }
    let mut depth = 0usize;
    let mut i = open_brace.saturating_add(1);
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'/' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'/' {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if next == b'*' {
                i += 2;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        if b == b'\'' || b == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            if depth == 0 {
                return Some(i);
            }
            depth = depth.saturating_sub(1);
        }
        i += 1;
    }
    None
}

fn parse_base_paths_in_range(text: &str, start: usize, end: usize) -> Vec<Vec<String>> {
    let bytes = text.as_bytes();
    let mut i = start.min(bytes.len());
    let end = end.min(bytes.len());

    let mut bases: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut paren_depth = 0usize;

    while i < end {
        let b = bytes[i];
        if b == b'/' && i + 1 < end {
            let next = bytes[i + 1];
            if next == b'/' {
                i += 2;
                while i < end && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if next == b'*' {
                i += 2;
                while i + 1 < end {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        if b == b'\'' || b == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if b == b'(' {
            paren_depth += 1;
            i += 1;
            continue;
        }
        if b == b')' {
            paren_depth = paren_depth.saturating_sub(1);
            i += 1;
            continue;
        }
        if paren_depth > 0 {
            i += 1;
            continue;
        }
        if is_ident_byte(b) {
            let start_ident = i;
            i += 1;
            while i < end && is_ident_byte(bytes[i]) {
                i += 1;
            }
            if let Ok(ident) = std::str::from_utf8(&bytes[start_ident..i]) {
                current.push(ident.to_string());
            }
            continue;
        }
        if b == b',' {
            if !current.is_empty() {
                bases.push(std::mem::take(&mut current));
            }
            i += 1;
            continue;
        }
        i += 1;
    }

    if !current.is_empty() {
        bases.push(current);
    }

    bases
}

fn contract_decl_by_name_fallback(text: &str, contract_name: &str) -> Option<ContractDecl> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut scanner = FallbackScanner::new(text);
    let mut brace_depth = 0usize;
    while let Some(token) = scanner.next_token() {
        match token.kind {
            FallbackTokenKind::Punct('{') => brace_depth += 1,
            FallbackTokenKind::Punct('}') => brace_depth = brace_depth.saturating_sub(1),
            FallbackTokenKind::Ident(ref ident)
                if brace_depth == 0 && is_contract_keyword(ident) =>
            {
                if let Some(decl) = parse_contract_decl(&mut scanner, text, token.start) {
                    brace_depth = 0;
                    if decl.name == contract_name {
                        return Some(decl);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn fallback_contract_member_definitions(text: &str, contract_name: &str) -> Vec<VisibleDefinition> {
    let Some(decl) = contract_decl_by_name_fallback(text, contract_name) else {
        return Vec::new();
    };

    let mut scanner = FallbackScanner::new(text);
    scanner.idx = decl.open_brace.saturating_add(1);
    let mut brace_depth = 1usize;

    let mut defs = Vec::new();
    let mut seen = HashSet::new();
    let mut pending_kind: Option<DefKind> = None;
    let mut statement_idents: Vec<String> = Vec::new();
    let mut statement_has_decl_keyword = false;
    let mut statement_skip_variable = false;

    while let Some(token) = scanner.next_token() {
        if token.start > decl.close_brace {
            break;
        }
        match token.kind {
            FallbackTokenKind::Ident(ident) => {
                if brace_depth != 1 {
                    continue;
                }
                if let Some(kind) = pending_kind.take() {
                    if seen.insert((ident.clone(), kind)) {
                        defs.push(VisibleDefinition { name: ident, kind });
                    }
                    continue;
                }
                match ident.as_str() {
                    "function" => {
                        pending_kind = Some(DefKind::Function);
                        statement_has_decl_keyword = true;
                    }
                    "event" => {
                        pending_kind = Some(DefKind::Event);
                        statement_has_decl_keyword = true;
                    }
                    "error" => {
                        pending_kind = Some(DefKind::Error);
                        statement_has_decl_keyword = true;
                    }
                    "modifier" => {
                        pending_kind = Some(DefKind::Modifier);
                        statement_has_decl_keyword = true;
                    }
                    "struct" => {
                        pending_kind = Some(DefKind::Struct);
                        statement_has_decl_keyword = true;
                    }
                    "enum" => {
                        pending_kind = Some(DefKind::Enum);
                        statement_has_decl_keyword = true;
                    }
                    "type" => {
                        pending_kind = Some(DefKind::Udvt);
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
            FallbackTokenKind::Punct('{') => {
                if brace_depth == 1 {
                    statement_idents.clear();
                    statement_has_decl_keyword = false;
                    statement_skip_variable = false;
                    pending_kind = None;
                }
                brace_depth += 1;
            }
            FallbackTokenKind::Punct('}') => {
                if brace_depth > 0 {
                    brace_depth = brace_depth.saturating_sub(1);
                }
                if brace_depth == 0 {
                    break;
                }
            }
            FallbackTokenKind::Punct(';') => {
                if brace_depth == 1 {
                    if !statement_has_decl_keyword
                        && !statement_skip_variable
                        && let Some(name) = statement_idents.last()
                    {
                        let name = name.clone();
                        if seen.insert((name.clone(), DefKind::Variable)) {
                            defs.push(VisibleDefinition {
                                name,
                                kind: DefKind::Variable,
                            });
                        }
                    }
                    statement_idents.clear();
                    statement_has_decl_keyword = false;
                    statement_skip_variable = false;
                }
            }
            _ => {}
        }
    }

    defs
}

fn skip_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start.saturating_add(1);
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i = i.saturating_add(2);
            continue;
        }
        if b == quote {
            return i.saturating_add(1);
        }
        i += 1;
    }
    bytes.len()
}

struct FallbackScanner<'a> {
    bytes: &'a [u8],
    idx: usize,
}

struct FallbackToken {
    kind: FallbackTokenKind,
    start: usize,
    end: usize,
}

enum FallbackTokenKind {
    Ident(String),
    Punct(char),
}

impl<'a> FallbackScanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            idx: 0,
        }
    }

    fn next_token(&mut self) -> Option<FallbackToken> {
        self.skip_trivia();
        if self.idx >= self.bytes.len() {
            return None;
        }
        let start = self.idx;
        let b = self.bytes[self.idx];
        if is_ident_byte(b) {
            self.idx += 1;
            while self.idx < self.bytes.len() && is_ident_byte(self.bytes[self.idx]) {
                self.idx += 1;
            }
            let end = self.idx;
            let ident = std::str::from_utf8(&self.bytes[start..end])
                .ok()?
                .to_string();
            Some(FallbackToken {
                kind: FallbackTokenKind::Ident(ident),
                start,
                end,
            })
        } else {
            self.idx += 1;
            Some(FallbackToken {
                kind: FallbackTokenKind::Punct(b as char),
                start,
                end: self.idx,
            })
        }
    }

    fn skip_trivia(&mut self) {
        while self.idx < self.bytes.len() {
            let b = self.bytes[self.idx];
            if b.is_ascii_whitespace() {
                self.idx += 1;
                continue;
            }
            if b == b'/' && self.idx + 1 < self.bytes.len() {
                let next = self.bytes[self.idx + 1];
                if next == b'/' {
                    self.idx += 2;
                    while self.idx < self.bytes.len() && self.bytes[self.idx] != b'\n' {
                        self.idx += 1;
                    }
                    continue;
                }
                if next == b'*' {
                    self.idx += 2;
                    while self.idx + 1 < self.bytes.len() {
                        if self.bytes[self.idx] == b'*' && self.bytes[self.idx + 1] == b'/' {
                            self.idx += 2;
                            break;
                        }
                        self.idx += 1;
                    }
                    continue;
                }
            }
            if b == b'\'' || b == b'"' {
                self.idx = skip_string(self.bytes, self.idx);
                continue;
            }
            break;
        }
    }
}

fn contract_member_definitions_with_inheritance(
    db: &dyn HirDatabase,
    program: &HirProgram,
    file_id: FileId,
    contract_info: ContractInfo,
) -> Vec<VisibleDefinition> {
    let mut defs = Vec::new();
    let mut seen = HashSet::new();

    let mut current_defs =
        program.contract_member_definitions_in_file(file_id, &contract_info.name);
    if current_defs.is_empty() {
        let text = db.file_input(file_id).text(db);
        current_defs = fallback_contract_member_definitions(text.as_ref(), &contract_info.name);
    }
    merge_visible_definitions(current_defs, &mut defs, &mut seen);

    let mut visited = HashSet::new();
    let mut pending = Vec::new();
    for base_path in &contract_info.bases {
        if let Some(base_id) = resolve_contract_path(program, file_id, base_path) {
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

        let mut base_defs = program.contract_member_definitions_in_file(base_file_id, base_name);
        if base_defs.is_empty() {
            let text = db.file_input(base_file_id).text(db);
            base_defs = fallback_contract_member_definitions(text.as_ref(), base_name);
        }
        merge_visible_definitions(base_defs, &mut defs, &mut seen);

        let base_paths = contract_bases_in_file(db, base_file_id, base_name);
        for base_path in base_paths {
            if let Some(next_id) = resolve_contract_path(program, base_file_id, &base_path) {
                pending.push(next_id);
            }
        }
    }

    defs
}

fn merge_visible_definitions(
    additions: Vec<VisibleDefinition>,
    defs: &mut Vec<VisibleDefinition>,
    seen: &mut HashSet<(String, DefKind)>,
) {
    for def in additions {
        if seen.insert((def.name().to_string(), def.kind())) {
            defs.push(def);
        }
    }
}

fn resolve_contract_path(program: &HirProgram, file_id: FileId, path: &[String]) -> Option<DefId> {
    let name = path.last()?.as_str();
    let def_id = if path.len() == 1 {
        program.resolve_contract(file_id, name)
    } else {
        let qualifier = path.first()?.as_str();
        program.resolve_qualified_symbol(file_id, qualifier, name)
    };
    match def_id {
        Some(def_id @ DefId::Contract(_)) => Some(def_id),
        _ => None,
    }
}

fn contract_bases_in_file(
    db: &dyn HirDatabase,
    file_id: FileId,
    contract_name: &str,
) -> Vec<Vec<String>> {
    let text = db.file_input(file_id).text(db);
    let parse = sa_syntax::parse_file(text.as_ref());
    let bases = contract_bases_in_parse(&parse, contract_name);
    if bases.is_empty() {
        contract_bases_in_file_fallback(text.as_ref(), contract_name)
    } else {
        bases
    }
}

fn contract_bases_in_file_fallback(text: &str, contract_name: &str) -> Vec<Vec<String>> {
    contract_decl_by_name_fallback(text, contract_name)
        .map(|decl| decl.bases)
        .unwrap_or_default()
}

fn contract_bases_in_parse(parse: &Parse, contract_name: &str) -> Vec<Vec<String>> {
    parse.with_session(|| {
        for item in parse.tree().items.iter() {
            if let ItemKind::Contract(contract) = &item.kind
                && contract.name.as_str() == contract_name
            {
                return contract_base_paths(contract);
            }
        }
        Vec::new()
    })
}

fn contract_base_paths(contract: &sa_syntax::ast::ItemContract<'_>) -> Vec<Vec<String>> {
    contract
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
        .collect()
}

fn collect_imports(
    workspace: &FoundryWorkspace,
    remappings: &[Remapping],
    current_path: &NormalizedPath,
    parsed: &ParsedFile,
    path_to_file_id: &HashMap<String, FileId>,
    text: &str,
) -> Vec<Import> {
    let resolver = FoundryResolver::new(workspace, remappings).ok();
    let resolved_by_path = resolver
        .as_ref()
        .and_then(|resolver| resolver.resolved_imports(current_path, text).ok())
        .map(|imports| {
            let mut resolved = HashMap::new();
            for import in imports {
                resolved.insert(import.path, import.resolved_path);
            }
            resolved
        });

    parsed
        .imports()
        .iter()
        .map(|import| {
            let path = import.path.clone();
            let resolved = resolved_by_path
                .as_ref()
                .and_then(|resolved| resolved.get(&path).cloned())
                .flatten()
                .or_else(|| {
                    resolve_import_path_with_resolver(
                        workspace,
                        remappings,
                        current_path,
                        &path,
                        resolver.as_ref(),
                    )
                });
            let resolved =
                resolved.or_else(|| resolve_relative_import_fallback(current_path, &path));
            let file_id = resolved
                .as_ref()
                .and_then(|resolved| path_to_file_id.get(resolved.as_str()).copied());
            Import {
                path,
                resolved_path: resolved,
                file_id,
                items: import.items.clone(),
            }
        })
        .collect()
}

fn resolve_relative_import_fallback(
    current_path: &NormalizedPath,
    import_path: &str,
) -> Option<NormalizedPath> {
    // Best-effort for VFS-only files where Foundry's resolver requires on-disk paths.
    let import_path = import_path.replace('\\', "/");
    if !import_path.starts_with("./") && !import_path.starts_with("../") {
        return None;
    }
    let current = Path::new(current_path.as_str());
    let base = current.parent().unwrap_or_else(|| Path::new("."));
    let combined = base.join(import_path);
    Some(NormalizedPath::new(combined.to_string_lossy()))
}

pub struct Semantics<'db> {
    db: &'db dyn HirDatabase,
    project_id: ProjectId,
}

impl<'db> Semantics<'db> {
    pub fn new(db: &'db dyn HirDatabase, project_id: ProjectId) -> Self {
        Self { db, project_id }
    }

    pub fn resolve_local(&self, file_id: FileId, offset: TextSize) -> Option<LocalDef> {
        let text = self.db.file_input(file_id).text(self.db);
        let locator = IdentRangeCollector::new();
        let (qualifier, name) = locator.qualified_name_at_offset(text.as_ref(), offset)?;
        if qualifier.is_some() {
            return None;
        }
        let locals = local_scopes(self.db, file_id);
        locals.resolve(&name, offset)
    }

    pub fn resolve_definition(&self, file_id: FileId, offset: TextSize) -> Option<Definition> {
        if let Some(local) = self.resolve_local(file_id, offset) {
            return Some(Definition::Local(local));
        }
        self.source_to_def(file_id, offset).map(Definition::Global)
    }

    pub fn source_to_def_location(
        &self,
        file_id: FileId,
        offset: TextSize,
    ) -> Option<DefinitionLocation> {
        if let Some(outcome) = self.sema_resolution(file_id, offset) {
            return match outcome {
                ResolveOutcome::Resolved(symbol) => Some(DefinitionLocation {
                    file_id: symbol.definition_file_id,
                    range: symbol.definition_range,
                    origin_range: Some(symbol.origin_range),
                }),
                ResolveOutcome::Unresolved { .. } => None,
                ResolveOutcome::Unavailable => None,
            };
        }

        let def_id = self.source_to_def_fallback(file_id, offset)?;
        let program = lowered_program(self.db, self.project_id);
        let entry = program.def_map().entry(def_id)?;
        Some(DefinitionLocation {
            file_id: entry.location().file_id(),
            range: entry.location().range(),
            origin_range: None,
        })
    }

    pub fn source_to_def(&self, file_id: FileId, offset: TextSize) -> Option<DefId> {
        if let Some(outcome) = self.sema_resolution(file_id, offset) {
            return match outcome {
                ResolveOutcome::Resolved(symbol) => {
                    let program = lowered_program(self.db, self.project_id);
                    def_id_from_symbol(&program, &symbol)
                }
                ResolveOutcome::Unresolved { .. } => None,
                ResolveOutcome::Unavailable => None,
            };
        }
        self.source_to_def_fallback(file_id, offset)
    }

    fn sema_resolution(&self, file_id: FileId, offset: TextSize) -> Option<ResolveOutcome> {
        let project = self.db.project_input(self.project_id);
        let snapshot = sa_sema::sema_snapshot_for_project(self.db, project);
        let snapshot = snapshot.for_file(file_id)?;
        match snapshot.resolve_definition(file_id, offset) {
            ResolveOutcome::Unavailable => None,
            outcome => Some(outcome),
        }
    }

    fn source_to_def_fallback(&self, file_id: FileId, offset: TextSize) -> Option<DefId> {
        let text = self.db.file_input(file_id).text(self.db);
        let locator = IdentRangeCollector::new();
        let (qualifier, name) = locator.qualified_name_at_offset(text.as_ref(), offset)?;
        let program = lowered_program(self.db, self.project_id);
        match qualifier {
            Some(qualifier) => {
                let locals = local_scopes(self.db, file_id);
                let mut parts = qualifier.name.split('.');
                let first = parts.next()?;
                let second = parts.next();
                let has_more = parts.next().is_some();

                if locals.resolve(first, qualifier.start).is_some() {
                    if second.is_none() && !has_more {
                        return program.resolve_symbol(file_id, &name);
                    }
                    return None;
                }

                match (second, has_more) {
                    (None, _) => program
                        .resolve_qualified_symbol(file_id, first, &name)
                        .or_else(|| {
                            program.resolve_contract_qualified_symbol(file_id, first, &name)
                        }),
                    (Some(second), false) => program
                        .resolve_qualified_symbol(file_id, first, second)
                        .and_then(|container_id| {
                            let entry = program.def_map().entry(container_id)?;
                            let container = entry.location().name();
                            let container_file_id = entry.location().file_id();
                            program.resolve_symbol_in_container(container_file_id, container, &name)
                        }),
                    (Some(_), true) => None,
                }
            }
            None => program.resolve_symbol(file_id, &name),
        }
    }
}

fn def_id_from_symbol(program: &HirProgram, symbol: &ResolvedSymbol) -> Option<DefId> {
    let kind = match symbol.kind {
        ResolvedSymbolKind::Contract => DefKind::Contract,
        ResolvedSymbolKind::Function => DefKind::Function,
        ResolvedSymbolKind::Modifier => DefKind::Modifier,
        ResolvedSymbolKind::Struct => DefKind::Struct,
        ResolvedSymbolKind::Enum => DefKind::Enum,
        ResolvedSymbolKind::Event => DefKind::Event,
        ResolvedSymbolKind::Error => DefKind::Error,
        ResolvedSymbolKind::Variable => DefKind::Variable,
        ResolvedSymbolKind::Udvt => DefKind::Udvt,
    };
    for entry in program
        .def_map()
        .entries_by_name_in_file(symbol.definition_file_id, &symbol.name)
        .into_iter()
        .filter(|entry| entry.kind() == kind && entry.container() == symbol.container.as_deref())
    {
        if entry.location().range() == symbol.definition_range {
            return Some(entry.id());
        }
    }
    None
}
