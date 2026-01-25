use std::collections::HashSet;

use sa_base_db::{FileId, LanguageKind, ProjectId, ProjectInput};
use sa_def::{DefId, DefKind};
use sa_hir::{HirDatabase, local_scopes, lowered_program_for_project};
use sa_sema::{ResolvedSymbolKind, SemaSymbol, sema_snapshot_for_project};
use sa_span::TextRange;
use sa_syntax::tokens::{IdentRangeCollector, QualifiedIdentRange};

#[salsa::db]
pub trait IdeDatabase: HirDatabase {}

#[salsa::db]
impl IdeDatabase for sa_base_db::Database {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    name: String,
    kind: DefKind,
    file_id: FileId,
    range: TextRange,
}

impl Symbol {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> DefKind {
        self.kind
    }

    pub fn file_id(&self) -> FileId {
        self.file_id
    }

    pub fn range(&self) -> TextRange {
        self.range
    }
}

unsafe impl salsa::Update for Symbol {
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
pub struct Reference {
    file_id: FileId,
    range: TextRange,
}

impl Reference {
    pub fn new(file_id: FileId, range: TextRange) -> Self {
        Self { file_id, range }
    }

    pub fn file_id(&self) -> FileId {
        self.file_id
    }

    pub fn range(&self) -> TextRange {
        self.range
    }
}

unsafe impl salsa::Update for Reference {
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

#[salsa::tracked]
pub fn symbol_search_for_project(
    db: &dyn IdeDatabase,
    project: ProjectInput,
    query: String,
) -> Vec<Symbol> {
    if let Some(symbols) = sema_symbol_search(db, project, &query) {
        return symbols;
    }
    let program = lowered_program_for_project(db, project);
    let mut symbols = program
        .def_map()
        .entries()
        .iter()
        .filter(|entry| entry.location().name().contains(&query))
        .map(|entry| Symbol {
            name: entry.location().name().to_string(),
            kind: entry.kind(),
            file_id: entry.location().file_id(),
            range: entry.location().range(),
        })
        .collect::<Vec<_>>();

    symbols.sort_by(|a, b| (a.name.as_str(), a.file_id).cmp(&(b.name.as_str(), b.file_id)));
    symbols
}

pub fn symbol_search(db: &dyn IdeDatabase, project_id: ProjectId, query: &str) -> Vec<Symbol> {
    symbol_search_for_project(db, db.project_input(project_id), query.to_string())
}

#[salsa::tracked]
pub fn find_references_for_project(
    db: &dyn IdeDatabase,
    project: ProjectInput,
    def_id: DefId,
) -> Vec<Reference> {
    let program = lowered_program_for_project(db, project);
    if let Some(sema_refs) = sema_references_for_def(db, project, def_id, &program) {
        return sema_refs;
    }
    let Some(entry) = program.def_map().entry(def_id) else {
        return Vec::new();
    };
    let name = entry.location().name();
    let kind = entry.kind();
    let def_file_id = entry.location().file_id();
    let container = entry.container();
    let ident_ranges = IdentRangeCollector::new();

    let mut refs = Vec::new();
    for file_id in db.file_ids() {
        let file_input = db.file_input(file_id);
        if file_input.kind(db) != LanguageKind::Solidity {
            continue;
        }

        let mut names = Vec::new();
        if file_id == def_file_id {
            names.push(name.to_string());
        }
        names.extend(program.local_names_for_imported(file_id, def_file_id, name));
        let mut qualifiers = program.qualifier_names_for_imported(file_id, def_file_id);
        if let Some(container_name) = container {
            qualifiers.extend(program.local_names_for_imported(
                file_id,
                def_file_id,
                container_name,
            ));
        }
        if names.is_empty() && qualifiers.is_empty() {
            continue;
        }
        names.sort();
        names.dedup();
        qualifiers.sort();
        qualifiers.dedup();

        let text = file_input.text(db);
        let locals = local_scopes(db, file_id);
        let dot_qualified_starts: HashSet<_> = ident_ranges
            .collect_dot_qualified_ranges(text.as_ref())
            .into_iter()
            .map(|range| range.start())
            .collect();
        for reference_name in names {
            let candidates = program.resolve_symbol_kind_candidates(file_id, kind, &reference_name);
            let is_match = if candidates.len() == 1 {
                candidates[0] == def_id
            } else {
                let container_candidates = candidates
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        program
                            .def_map()
                            .entry(*candidate)
                            .is_some_and(|entry| entry.container() == container)
                    })
                    .collect::<Vec<_>>();
                container_candidates.len() == 1 && container_candidates[0] == def_id
            };
            if !is_match {
                continue;
            }

            for range in ident_ranges.collect(text.as_ref(), &reference_name) {
                if !dot_qualified_starts.contains(&range.start())
                    && locals.resolve(&reference_name, range.start()).is_some()
                {
                    continue;
                }
                refs.push(Reference { file_id, range });
            }
        }

        let has_global_qualifier = |qualifier: &str| {
            program
                .def_map()
                .entries_by_name_in_file(file_id, qualifier)
                .into_iter()
                .next()
                .is_some()
        };

        for qualifier in qualifiers {
            if has_global_qualifier(&qualifier) {
                continue;
            }
            for QualifiedIdentRange {
                range,
                qualifier_start,
            } in ident_ranges.collect_qualified(text.as_ref(), &qualifier, name)
            {
                if locals.resolve(&qualifier, qualifier_start).is_some() {
                    continue;
                }
                refs.push(Reference { file_id, range });
            }
        }
    }

    refs.sort_by(|a, b| (a.file_id, a.range.start()).cmp(&(b.file_id, b.range.start())));
    refs
}

fn sema_references_for_def(
    db: &dyn IdeDatabase,
    project: ProjectInput,
    def_id: DefId,
    program: &sa_hir::HirProgram,
) -> Option<Vec<Reference>> {
    let entry = program.def_map().entry(def_id)?;
    let def_file_id = entry.location().file_id();
    let def_range = entry.location().range();
    let snapshot = sema_snapshot_for_project(db, project);
    let snapshot = snapshot.for_file(def_file_id)?;
    let refs = match snapshot.references_for_definition(def_file_id, def_range) {
        Some(refs) => refs,
        None => return Some(Vec::new()),
    };

    let name = entry.location().name();
    let kind = entry.kind();

    let mut references = refs
        .iter()
        .map(|reference| Reference::new(reference.file_id(), reference.range()))
        .collect::<Vec<_>>();
    references.retain(|reference| {
        if reference.file_id() == def_file_id {
            return true;
        }
        let candidates = program.resolve_symbol_kind_candidates(reference.file_id(), kind, name);
        if candidates.len() <= 1 {
            return true;
        }
        let mut candidate_files = HashSet::new();
        for candidate in candidates {
            let Some(entry) = program.def_map().entry(candidate) else {
                continue;
            };
            candidate_files.insert(entry.location().file_id());
        }
        candidate_files.len() <= 1
    });
    references.sort_by(|a, b| (a.file_id, a.range.start()).cmp(&(b.file_id, b.range.start())));
    Some(references)
}

fn sema_symbol_search(
    db: &dyn IdeDatabase,
    project: ProjectInput,
    query: &str,
) -> Option<Vec<Symbol>> {
    let snapshot = sema_snapshot_for_project(db, project);
    let snapshot = snapshot.as_ref()?;
    let mut symbols = snapshot
        .workspace_symbols(query)
        .into_iter()
        .map(symbol_from_sema)
        .collect::<Vec<_>>();

    symbols.sort_by(|a, b| (a.name.as_str(), a.file_id).cmp(&(b.name.as_str(), b.file_id)));
    Some(symbols)
}

fn symbol_from_sema(symbol: SemaSymbol) -> Symbol {
    Symbol {
        name: symbol.name,
        kind: def_kind_from_sema(symbol.kind),
        file_id: symbol.file_id,
        range: symbol.selection_range,
    }
}

fn def_kind_from_sema(kind: ResolvedSymbolKind) -> DefKind {
    match kind {
        ResolvedSymbolKind::Contract => DefKind::Contract,
        ResolvedSymbolKind::Function => DefKind::Function,
        ResolvedSymbolKind::Modifier => DefKind::Modifier,
        ResolvedSymbolKind::Struct => DefKind::Struct,
        ResolvedSymbolKind::Enum => DefKind::Enum,
        ResolvedSymbolKind::Event => DefKind::Event,
        ResolvedSymbolKind::Error => DefKind::Error,
        ResolvedSymbolKind::Variable => DefKind::Variable,
        ResolvedSymbolKind::Udvt => DefKind::Udvt,
    }
}

pub fn find_references(
    db: &dyn IdeDatabase,
    project_id: ProjectId,
    def_id: DefId,
) -> Vec<Reference> {
    find_references_for_project(db, db.project_input(project_id), def_id)
}
