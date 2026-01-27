use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use foundry_compilers::utils::canonicalize;
use sa_base_db::{FileId, LanguageKind, ProjectInput, SaDatabase, SaDatabaseExt};
use sa_config::{ResolvedFoundryConfig, solar_opts_from_config};
use sa_paths::{NormalizedPath, WorkspacePath};
use sa_project_model::{
    FoundryResolver, FoundryWorkspace, Remapping, resolve_import_path_with_resolver,
};
use sa_span::{TextRange, TextSize};
use sa_vfs::{Vfs, VfsChange, VfsSnapshot};
use solar::interface::diagnostics::{DiagCtxt, ErrorGuaranteed, InMemoryEmitter};
use solar::interface::source_map::{FileLoader, SourceMap};
use solar::interface::{Session, Span};
use solar::sema::Compiler;
use solar::sema::hir::SourceId;
use solar::sema::{Gcx, hir};
use tracing::{debug, warn};

mod completion;
mod contract_members;
mod exports;
mod references;
mod resolve;
mod symbols;
mod ty_utils;

pub use completion::{SemaCompletionItem, SemaCompletionKind};
pub use references::SemaReference;
pub use resolve::{ResolveOutcome, ResolvedSymbol, ResolvedSymbolKind};
pub use symbols::SemaSymbol;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemaFunctionSignature {
    pub label: String,
    pub parameters: Vec<String>,
}

#[salsa::db]
pub trait SemaDatabase: SaDatabase + SaDatabaseExt {}

#[salsa::db]
impl SemaDatabase for sa_base_db::Database {}

pub struct SemaSnapshot {
    compiler: Compiler,
    pub(crate) source_map: Arc<SourceMap>,
    source_id_by_file: HashMap<FileId, SourceId>,
    pub(crate) file_id_by_source: HashMap<SourceId, FileId>,
    reference_index: OnceLock<references::SemaReferenceIndex>,
}

impl SemaSnapshot {
    pub fn new(
        config: &ResolvedFoundryConfig,
        vfs: &VfsSnapshot,
        path_to_file_id: &HashMap<NormalizedPath, FileId>,
        skip_files: Option<&HashSet<FileId>>,
        resolve_imports: bool,
    ) -> Result<Self> {
        let (emitter, _buffer) = InMemoryEmitter::new();
        let dcx = DiagCtxt::new(Box::new(emitter));
        let source_map = Arc::new(SourceMap::empty());
        source_map.set_file_loader(VfsOverlayFileLoader::new(vfs.clone()));
        let opts = solar_opts_from_config(config);
        let session = Session::builder()
            .dcx(dcx)
            .source_map(Arc::clone(&source_map))
            .opts(opts)
            .build();
        let mut compiler = Compiler::new(session);

        let files = collect_workspace_files(config.workspace(), vfs, path_to_file_id, skip_files);
        let parse_result =
            compiler.enter_mut(|compiler| -> std::result::Result<(), ErrorGuaranteed> {
                let mut parser = compiler.parse();
                parser.set_resolve_imports(resolve_imports);
                parser.load_files(files.iter())?;
                parser.parse();
                let _ = compiler.lower_asts()?;
                Ok(())
            });
        if parse_result.is_err() {
            warn!("sema snapshot built with errors");
        }

        let (source_id_by_file, file_id_by_source) =
            compiler.enter(|compiler| build_source_mappings(compiler.gcx(), path_to_file_id));

        Ok(Self {
            compiler,
            source_map,
            source_id_by_file,
            file_id_by_source,
            reference_index: OnceLock::new(),
        })
    }

    pub fn with_gcx<T: Send>(
        &self,
        f: impl for<'gcx> FnOnce(solar::sema::Gcx<'gcx>) -> T + Send,
    ) -> T {
        self.compiler.enter(|compiler| f(compiler.gcx()))
    }

    pub fn source_id_for_file(&self, file_id: FileId) -> Option<SourceId> {
        self.source_id_by_file.get(&file_id).copied()
    }

    pub fn file_id_for_source(&self, source_id: SourceId) -> Option<FileId> {
        self.file_id_by_source.get(&source_id).copied()
    }

    pub fn span_to_text_range(&self, span: Span) -> Option<TextRange> {
        let range = self.source_map.span_to_range(span).ok()?;
        let start = TextSize::try_from(range.start).ok()?;
        let end = TextSize::try_from(range.end).ok()?;
        Some(TextRange::new(start, end))
    }

    pub fn function_signature_for_definition(
        &self,
        file_id: FileId,
        name_range: TextRange,
        name: &str,
        container: Option<&str>,
    ) -> Option<SemaFunctionSignature> {
        self.with_gcx(|gcx| {
            let item_id = self.item_id_for_name_range(gcx, file_id, name_range, name, container)?;
            let hir::Item::Function(function) = gcx.hir.item(item_id) else {
                return None;
            };
            Some(format_hir_function_signature(gcx, function))
        })
    }

    pub fn variable_label_for_definition(
        &self,
        file_id: FileId,
        name_range: TextRange,
        name: &str,
        container: Option<&str>,
    ) -> Option<String> {
        self.with_gcx(|gcx| {
            let item_id = self.item_id_for_name_range(gcx, file_id, name_range, name, container)?;
            let hir::Item::Variable(variable) = gcx.hir.item(item_id) else {
                return None;
            };
            let ty = gcx.type_of_item(item_id);
            let ty = ty.display(gcx).to_string();
            let name = variable.name.map(|ident| ident.as_str().to_string());
            Some(match name {
                Some(name) => format!("{ty} {name}"),
                None => ty,
            })
        })
    }

    pub fn function_abi_signature_for_definition(
        &self,
        file_id: FileId,
        name_range: TextRange,
        name: &str,
        container: Option<&str>,
    ) -> Option<String> {
        self.with_gcx(|gcx| {
            let item_id = self.item_id_for_name_range(gcx, file_id, name_range, name, container)?;
            let hir::ItemId::Function(_) = item_id else {
                return None;
            };
            Some(gcx.item_signature(item_id).to_string())
        })
    }

    pub fn references_for_definition(
        &self,
        definition_file_id: FileId,
        definition_range: TextRange,
    ) -> Option<&[SemaReference]> {
        self.reference_index()
            .references_for(definition_file_id, definition_range)
    }

    fn reference_index(&self) -> &references::SemaReferenceIndex {
        self.reference_index
            .get_or_init(|| references::SemaReferenceIndex::new(self))
    }

    fn item_id_for_name_range(
        &self,
        gcx: Gcx<'_>,
        file_id: FileId,
        name_range: TextRange,
        name: &str,
        container: Option<&str>,
    ) -> Option<hir::ItemId> {
        let source_id = self.source_id_for_file(file_id)?;
        let source = gcx.hir.source(source_id);
        let mut name_matches = Vec::new();
        let mut items: Vec<hir::ItemId> = Vec::new();

        if let Some(container_name) = container {
            let contract_id = source.items.iter().find_map(|item_id| {
                let contract_id = item_id.as_contract()?;
                let contract = gcx.hir.contract(contract_id);
                (contract.name.as_str() == container_name).then_some(contract_id)
            })?;
            let contract = gcx.hir.contract(contract_id);
            items.extend(contract.items.iter().copied());
            if let Some(ctor) = contract.ctor {
                items.push(ctor.into());
            }
            if let Some(fallback) = contract.fallback {
                items.push(fallback.into());
            }
            if let Some(receive) = contract.receive {
                items.push(receive.into());
            }
        } else {
            items.extend(source.items.iter().copied());
        }

        for item_id in items {
            let item = gcx.hir.item(item_id);
            let Some(item_range) = self.item_name_range(item) else {
                continue;
            };
            if item_range != name_range {
                if self.item_matches_name(item, name)
                    && self.item_matches_container(gcx, item, container)
                {
                    name_matches.push(item_id);
                }
                continue;
            }
            if !self.item_matches_container(gcx, item, container) {
                continue;
            }
            return Some(item_id);
        }
        if name_matches.len() == 1 {
            return Some(name_matches[0]);
        }
        None
    }

    fn item_name_range(&self, item: hir::Item<'_, '_>) -> Option<TextRange> {
        match item {
            hir::Item::Function(function) => {
                let span = function
                    .name
                    .map(|name| name.span)
                    .unwrap_or_else(|| function.keyword_span());
                self.span_to_text_range(span)
            }
            _ => item
                .name()
                .and_then(|name| self.span_to_text_range(name.span)),
        }
    }

    fn item_matches_container(
        &self,
        gcx: Gcx<'_>,
        item: hir::Item<'_, '_>,
        container: Option<&str>,
    ) -> bool {
        match (container, item.contract()) {
            (Some(container), Some(contract_id)) => {
                gcx.hir.contract(contract_id).name.as_str() == container
            }
            (Some(_), None) => false,
            (None, Some(_)) => false,
            (None, None) => true,
        }
    }

    fn item_matches_name(&self, item: hir::Item<'_, '_>, name: &str) -> bool {
        match item {
            hir::Item::Function(function) => match function.name {
                Some(ident) => ident.as_str() == name,
                None => function.kind.to_str() == name,
            },
            _ => item.name().is_some_and(|ident| ident.as_str() == name),
        }
    }
}

fn format_hir_function_signature<'gcx>(
    gcx: Gcx<'gcx>,
    function: &hir::Function<'gcx>,
) -> SemaFunctionSignature {
    let mut signature = String::new();
    signature.push_str(function.kind.to_str());
    if let Some(name) = function.name {
        signature.push(' ');
        signature.push_str(name.as_str());
    }

    let parameters = function
        .parameters
        .iter()
        .map(|&var_id| format_hir_param(gcx, var_id))
        .collect::<Vec<_>>();
    signature.push('(');
    signature.push_str(&parameters.join(", "));
    signature.push(')');

    let returns = function
        .returns
        .iter()
        .map(|&var_id| format_hir_param(gcx, var_id))
        .collect::<Vec<_>>();
    if !returns.is_empty() {
        signature.push_str(" returns (");
        signature.push_str(&returns.join(", "));
        signature.push(')');
    }

    SemaFunctionSignature {
        label: signature,
        parameters,
    }
}

fn format_hir_param<'gcx>(gcx: Gcx<'gcx>, var_id: hir::VariableId) -> String {
    let var = gcx.hir.variable(var_id);
    let ty = gcx.type_of_item(var_id.into());
    let ty = ty.display(gcx).to_string();
    match var.name {
        Some(name) => format!("{ty} {}", name.as_str()),
        None => ty,
    }
}

#[derive(Clone)]
pub struct SemaSnapshotResult {
    snapshot: Option<Arc<SemaSnapshot>>,
    no_imports_snapshot: Option<Arc<SemaSnapshot>>,
    missing_imports: HashSet<FileId>,
}

impl SemaSnapshotResult {
    pub fn new(
        snapshot: Option<Arc<SemaSnapshot>>,
        no_imports_snapshot: Option<Arc<SemaSnapshot>>,
        missing_imports: HashSet<FileId>,
    ) -> Self {
        Self {
            snapshot,
            no_imports_snapshot,
            missing_imports,
        }
    }

    pub fn as_ref(&self) -> Option<&SemaSnapshot> {
        self.snapshot
            .as_deref()
            .or(self.no_imports_snapshot.as_deref())
    }

    pub fn for_file(&self, file_id: FileId) -> Option<&SemaSnapshot> {
        let (preferred, fallback) = if self.missing_imports.contains(&file_id) {
            (
                self.no_imports_snapshot.as_deref(),
                self.snapshot.as_deref(),
            )
        } else {
            (
                self.snapshot.as_deref(),
                self.no_imports_snapshot.as_deref(),
            )
        };
        if let Some(snapshot) = preferred
            && snapshot.source_id_for_file(file_id).is_some()
        {
            return Some(snapshot);
        }
        let snapshot = fallback?;
        snapshot.source_id_for_file(file_id)?;
        Some(snapshot)
    }
}

impl PartialEq for SemaSnapshotResult {
    fn eq(&self, other: &Self) -> bool {
        let snapshots_match = match (&self.snapshot, &other.snapshot) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        };
        if !snapshots_match {
            return false;
        }
        let fallback_match = match (&self.no_imports_snapshot, &other.no_imports_snapshot) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        };
        if !fallback_match {
            return false;
        }
        self.missing_imports == other.missing_imports
    }
}

impl Eq for SemaSnapshotResult {}

unsafe impl salsa::Update for SemaSnapshotResult {
    unsafe fn maybe_update(old_pointer: *mut Self, new_value: Self) -> bool {
        let old = unsafe { &mut *old_pointer };
        let snapshot_update = match (&old.snapshot, &new_value.snapshot) {
            (Some(old_snapshot), Some(new_snapshot)) => !Arc::ptr_eq(old_snapshot, new_snapshot),
            (None, None) => false,
            _ => true,
        };
        let fallback_update = match (&old.no_imports_snapshot, &new_value.no_imports_snapshot) {
            (Some(old_snapshot), Some(new_snapshot)) => !Arc::ptr_eq(old_snapshot, new_snapshot),
            (None, None) => false,
            _ => true,
        };
        let missing_update = old.missing_imports != new_value.missing_imports;
        let should_update = snapshot_update || fallback_update || missing_update;

        if should_update {
            *old = new_value;
        }

        should_update
    }
}

#[salsa::tracked]
pub fn sema_snapshot_for_project(
    db: &dyn SemaDatabase,
    project: ProjectInput,
) -> SemaSnapshotResult {
    let config = project.config(db).clone();
    let workspace = config.workspace().clone();
    let remappings = config.active_profile().remappings();
    let (vfs, path_to_file_id) = vfs_snapshot_from_db(db, &workspace);
    let missing_imports = files_with_missing_imports(db, &workspace, remappings, &path_to_file_id);
    let snapshot = SemaSnapshot::new(
        &config,
        &vfs,
        &path_to_file_id,
        Some(&missing_imports),
        true,
    )
    .ok()
    .map(Arc::new);
    let no_imports_snapshot = if !missing_imports.is_empty() || snapshot.is_none() {
        SemaSnapshot::new(&config, &vfs, &path_to_file_id, None, false)
            .ok()
            .map(Arc::new)
    } else {
        None
    };
    SemaSnapshotResult::new(snapshot, no_imports_snapshot, missing_imports)
}

fn vfs_snapshot_from_db(
    db: &dyn SemaDatabase,
    workspace: &FoundryWorkspace,
) -> (VfsSnapshot, HashMap<NormalizedPath, FileId>) {
    let mut vfs = Vfs::default();
    let mut path_to_file_id = HashMap::new();
    for file_id in db.file_ids() {
        let file_input = db.file_input(file_id);
        if file_input.kind(db) != LanguageKind::Solidity {
            continue;
        }
        let path = db.file_path(file_id);
        if !is_workspace_path(workspace, &path) {
            continue;
        }
        path_to_file_id.insert((*path).clone(), file_id);
        vfs.apply_change(VfsChange::Set {
            path: (*path).clone(),
            text: file_input.text(db).clone(),
        });
    }

    (vfs.snapshot(), path_to_file_id)
}

fn files_with_missing_imports(
    db: &dyn SemaDatabase,
    workspace: &FoundryWorkspace,
    remappings: &[Remapping],
    path_to_file_id: &HashMap<NormalizedPath, FileId>,
) -> HashSet<FileId> {
    let mut missing = HashSet::new();
    let resolver = FoundryResolver::new(workspace, remappings).ok();
    for (path, file_id) in path_to_file_id {
        let text = db.file_input(*file_id).text(db);
        let parse = sa_syntax::parse_file(text);
        if !parse.errors().is_empty() {
            continue;
        }
        if let Some(resolver) = resolver.as_ref() {
            match resolver.resolved_imports(path, text.as_ref()) {
                Ok(imports) => {
                    for import in imports {
                        let resolved = import.resolved_path.or_else(|| {
                            resolve_import_path_with_resolver(
                                workspace,
                                remappings,
                                path,
                                &import.path,
                                Some(resolver),
                            )
                        });
                        let resolved = resolved
                            .or_else(|| resolve_relative_import_fallback(path, &import.path));
                        let Some(resolved) = resolved else {
                            missing.insert(*file_id);
                            break;
                        };
                        if !path_to_file_id.contains_key(&resolved) {
                            missing.insert(*file_id);
                            break;
                        }
                    }
                }
                Err(error) => {
                    debug!(
                        ?error,
                        path = %path,
                        "sema: failed to parse imports with foundry parser"
                    );
                }
            }
        }
    }
    missing
}

fn resolve_relative_import_fallback(
    current_path: &NormalizedPath,
    import_path: &str,
) -> Option<NormalizedPath> {
    // Best-effort for in-memory files where Foundry resolution requires on-disk paths.
    let import_path = import_path.replace('\\', "/");
    if !import_path.starts_with("./") && !import_path.starts_with("../") {
        return None;
    }
    let current = Path::new(current_path.as_str());
    let base = current.parent().unwrap_or_else(|| Path::new("."));
    let combined = base.join(import_path);
    Some(NormalizedPath::new(combined.to_string_lossy()))
}

fn collect_workspace_files(
    workspace: &FoundryWorkspace,
    vfs: &VfsSnapshot,
    path_to_file_id: &HashMap<NormalizedPath, FileId>,
    skip_files: Option<&HashSet<FileId>>,
) -> Vec<PathBuf> {
    vfs.iter()
        .filter_map(|(file_id, path)| {
            if !is_workspace_path(workspace, path) {
                return None;
            }
            if !path.as_str().ends_with(".sol") {
                return None;
            }
            // skip_files is keyed by DB FileId; map via path since VFS ids are rebuilt.
            if skip_files.is_some_and(|skip| {
                path_to_file_id
                    .get(path)
                    .is_some_and(|db_file_id| skip.contains(db_file_id))
            }) {
                return None;
            }
            let text = vfs.file_text(file_id)?;
            let parse = sa_syntax::parse_file(text);
            if !parse.errors().is_empty() {
                return None;
            }
            Some(PathBuf::from(path.as_str()))
        })
        .collect()
}

fn is_workspace_path(workspace: &FoundryWorkspace, path: &NormalizedPath) -> bool {
    let roots = [
        workspace.root(),
        workspace.src(),
        workspace.lib(),
        workspace.test(),
        workspace.script(),
    ];
    roots
        .iter()
        .any(|root| WorkspacePath::new(root, path).is_some())
}

fn build_source_mappings(
    gcx: solar::sema::Gcx<'_>,
    path_to_file_id: &HashMap<NormalizedPath, FileId>,
) -> (HashMap<FileId, SourceId>, HashMap<SourceId, FileId>) {
    let mut source_id_by_file = HashMap::new();
    let mut file_id_by_source = HashMap::new();

    for source_id in gcx.hir.source_ids() {
        let source = gcx.hir.source(source_id);
        let Some(path) = source.file.name.as_real() else {
            continue;
        };
        let Some(file_id) = file_id_for_path(path, path_to_file_id) else {
            continue;
        };
        source_id_by_file.insert(file_id, source_id);
        file_id_by_source.insert(source_id, file_id);
    }

    (source_id_by_file, file_id_by_source)
}

fn file_id_for_path(
    path: &Path,
    path_to_file_id: &HashMap<NormalizedPath, FileId>,
) -> Option<FileId> {
    let normalized = NormalizedPath::new(path.to_string_lossy());
    if let Some(file_id) = path_to_file_id.get(&normalized) {
        return Some(*file_id);
    }
    let canonical = canonicalize(path).ok()?;
    let normalized = NormalizedPath::new(canonical.to_string_lossy());
    path_to_file_id.get(&normalized).copied()
}

pub struct VfsOverlayFileLoader {
    snapshot: VfsSnapshot,
    fallback: solar::interface::source_map::RealFileLoader,
}

impl VfsOverlayFileLoader {
    pub fn new(snapshot: VfsSnapshot) -> Self {
        Self {
            snapshot,
            fallback: solar::interface::source_map::RealFileLoader,
        }
    }

    fn snapshot_text(&self, path: &Path) -> Option<String> {
        let normalized = self.normalized_path_for(path);
        let file_id = self.snapshot.file_id(&normalized)?;
        let text = self.snapshot.file_text(file_id)?;
        Some(text.to_string())
    }

    fn normalized_path_for(&self, path: &Path) -> NormalizedPath {
        let normalized = NormalizedPath::new(path.to_string_lossy());
        if self.snapshot.file_id(&normalized).is_some() {
            return normalized;
        }

        let canonical = match canonicalize(path) {
            Ok(canonical) => canonical,
            Err(error) => {
                warn!(path = %path.display(), error = %error, "Failed to canonicalize path");
                path.to_path_buf()
            }
        };
        NormalizedPath::new(canonical.to_string_lossy())
    }
}

impl FileLoader for VfsOverlayFileLoader {
    fn canonicalize_path(&self, path: &Path) -> io::Result<PathBuf> {
        if self.snapshot_text(path).is_some() {
            return Ok(path.to_path_buf());
        }
        self.fallback.canonicalize_path(path)
    }

    fn load_stdin(&self) -> io::Result<String> {
        self.fallback.load_stdin()
    }

    fn load_file(&self, path: &Path) -> io::Result<String> {
        if let Some(text) = self.snapshot_text(path) {
            return Ok(text);
        }
        self.fallback.load_file(path)
    }

    fn load_binary_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.fallback.load_binary_file(path)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    use sa_base_db::{Database, FileId, LanguageKind, ProjectId};
    use sa_config::ResolvedFoundryConfig;
    use sa_paths::NormalizedPath;
    use sa_project_model::{FoundryProfile, FoundryWorkspace, Remapping};
    use sa_test_support::extract_offset;
    use sa_test_utils::{Fixture, FixtureBuilder};
    use sa_vfs::VfsSnapshot;
    use solar::interface::source_map::FileLoader;

    use super::{
        SemaSnapshot, VfsOverlayFileLoader, collect_workspace_files, files_with_missing_imports,
        sema_snapshot_for_project, vfs_snapshot_from_db,
    };

    fn path_map(entries: &[(NormalizedPath, FileId)]) -> HashMap<NormalizedPath, FileId> {
        entries.iter().cloned().collect()
    }

    fn snapshot_for_fixture(fixture: &Fixture) -> (SemaSnapshot, HashMap<NormalizedPath, FileId>) {
        let path_to_file_id = fixture
            .vfs_snapshot()
            .iter()
            .map(|(file_id, path)| (path.clone(), file_id))
            .collect::<HashMap<_, _>>();
        let snapshot = SemaSnapshot::new(
            fixture.config(),
            fixture.vfs_snapshot(),
            &path_to_file_id,
            None,
            true,
        )
        .expect("sema snapshot");
        (snapshot, path_to_file_id)
    }

    fn populate_db_from_vfs(db: &mut Database, vfs: &VfsSnapshot) {
        for (file_id, path) in vfs.iter() {
            let text = vfs.file_text(file_id).expect("file text");
            let version = vfs.file_version(file_id).unwrap_or(0);
            db.set_file(
                file_id,
                Arc::from(text),
                version,
                LanguageKind::Solidity,
                Arc::new(path.clone()),
            );
        }
    }

    fn drop_ast_for_file(snapshot: &mut SemaSnapshot, file_id: FileId) {
        let source_id = snapshot.source_id_for_file(file_id).expect("source id");
        snapshot.compiler.enter_mut(|compiler| {
            let sources = compiler.sources_mut();
            sources[source_id].ast = None;
        });
    }

    #[test]
    fn collect_workspace_files_skips_by_db_file_id() {
        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file(
                "src/Missing.sol",
                r#"
import "./MissingDep.sol";

contract Missing {}
"#,
            )
            .file(
                "src/Ok.sol",
                r#"
contract Ok {}
"#,
            )
            .build()
            .expect("fixture");

        let missing_path = fixture
            .normalized_path("src/Missing.sol")
            .expect("missing path");
        let ok_path = fixture.normalized_path("src/Ok.sol").expect("ok path");
        let missing_db_id = FileId::from_raw(100);
        let ok_db_id = FileId::from_raw(200);

        let path_to_file_id = path_map(&[
            (missing_path.clone(), missing_db_id),
            (ok_path.clone(), ok_db_id),
        ]);

        let mut skip_files = HashSet::new();
        skip_files.insert(missing_db_id);

        let files = collect_workspace_files(
            fixture.config().workspace(),
            fixture.vfs_snapshot(),
            &path_to_file_id,
            Some(&skip_files),
        );
        let files: HashSet<PathBuf> = files.into_iter().collect();

        assert!(!files.contains(&PathBuf::from(missing_path.as_str())));
        assert!(files.contains(&PathBuf::from(ok_path.as_str())));
    }

    #[test]
    fn missing_imports_respect_active_profile_remappings() {
        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file(
                "src/Main.sol",
                r#"
import "vendor/Foo.sol";

contract Main {
    Foo foo;
}
"#,
            )
            .file(
                "lib/alt/Foo.sol",
                r#"
contract Foo {}
"#,
            )
            .build()
            .expect("fixture");

        let root = NormalizedPath::new(fixture.root().to_string_lossy());
        let workspace = FoundryWorkspace::new(root);
        let active_profile =
            FoundryProfile::new("dev").with_remappings(vec![Remapping::new("vendor/", "lib/alt/")]);
        let config = ResolvedFoundryConfig::new(workspace, active_profile);
        let vfs = fixture.vfs_snapshot();
        let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");

        let mut db = Database::default();
        populate_db_from_vfs(&mut db, vfs);

        let project_id = ProjectId::from_raw(0);
        db.set_project_input(project_id, Arc::new(config));

        let snapshot = sema_snapshot_for_project(&db, db.project_input(project_id));
        assert!(
            !snapshot.missing_imports.contains(&main_file_id),
            "expected dev profile remapping to resolve imports"
        );

        let workspace = db.project_input(project_id).workspace(&db);
        let (_, path_to_file_id) = vfs_snapshot_from_db(&db, workspace.as_ref());
        let missing_default =
            files_with_missing_imports(&db, workspace.as_ref(), &[], &path_to_file_id);
        assert!(
            missing_default.contains(&main_file_id),
            "expected default profile to miss dev remapping imports"
        );
    }

    #[test]
    fn identifier_completions_include_imports_when_ast_missing() {
        let (main_text, offset) = extract_offset(
            r#"
import "./Dep.sol";

contract Main {
    function test() public {
        /*caret*/
    }
}
"#
            .trim(),
        );
        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file("src/Main.sol", main_text)
            .file(
                "src/Dep.sol",
                r#"
contract Dep {}
"#
                .trim(),
            )
            .build()
            .expect("fixture");
        let (mut snapshot, path_to_file_id) = snapshot_for_fixture(&fixture);
        let main_path = fixture.normalized_path("src/Main.sol").expect("main path");
        let file_id = *path_to_file_id.get(&main_path).expect("file id");

        drop_ast_for_file(&mut snapshot, file_id);

        let completions = snapshot
            .identifier_completions(file_id, offset)
            .expect("completions");
        let labels: Vec<_> = completions.iter().map(|item| item.label.as_str()).collect();

        assert!(labels.contains(&"Dep"));
    }

    #[test]
    fn collect_workspace_files_ignores_invalid_and_non_solidity() {
        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file(
                "src/Ok.sol",
                r#"
contract Ok {}
"#,
            )
            .file(
                "src/Bad.sol",
                r#"
contract {
"#,
            )
            .file(
                "src/Readme.txt",
                r#"
not solidity
"#,
            )
            .build()
            .expect("fixture");

        let path_to_file_id = fixture
            .vfs_snapshot()
            .iter()
            .map(|(file_id, path)| (path.clone(), file_id))
            .collect::<HashMap<_, _>>();

        let files = collect_workspace_files(
            fixture.config().workspace(),
            fixture.vfs_snapshot(),
            &path_to_file_id,
            None,
        );
        let files: HashSet<PathBuf> = files.into_iter().collect();

        let ok_path = fixture.normalized_path("src/Ok.sol").expect("ok path");
        let bad_path = fixture.normalized_path("src/Bad.sol").expect("bad path");
        let readme_path = fixture
            .normalized_path("src/Readme.txt")
            .expect("readme path");

        assert!(files.contains(&PathBuf::from(ok_path.as_str())));
        assert!(!files.contains(&PathBuf::from(bad_path.as_str())));
        assert!(!files.contains(&PathBuf::from(readme_path.as_str())));
    }

    #[test]
    fn sema_snapshot_result_prefers_no_imports_for_missing_imports() {
        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file(
                "src/Main.sol",
                r#"
import "./Missing.sol";
contract Main {}
"#,
            )
            .file(
                "src/Ok.sol",
                r#"
contract Ok {}
"#,
            )
            .build()
            .expect("fixture");

        let vfs = fixture.vfs_snapshot();
        let mut db = Database::default();
        populate_db_from_vfs(&mut db, vfs);

        let project_id = ProjectId::from_raw(0);
        db.set_project_input(project_id, Arc::new(fixture.config().clone()));

        let result = sema_snapshot_for_project(&db, db.project_input(project_id));
        let snapshot = result.snapshot.as_ref().expect("snapshot");
        let no_imports = result
            .no_imports_snapshot
            .as_ref()
            .expect("no-imports snapshot");

        let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
        let ok_file_id = fixture.file_id("src/Ok.sol").expect("ok file id");

        let main_snapshot = result.for_file(main_file_id).expect("main snapshot");
        let ok_snapshot = result.for_file(ok_file_id).expect("ok snapshot");

        assert_eq!(
            main_snapshot as *const SemaSnapshot,
            Arc::as_ptr(no_imports)
        );
        assert_eq!(ok_snapshot as *const SemaSnapshot, Arc::as_ptr(snapshot));
    }

    #[test]
    fn vfs_overlay_loader_reads_snapshot_when_disk_missing() {
        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file(
                "src/Main.sol",
                r#"
contract Main {}
"#,
            )
            .build()
            .expect("fixture");

        let file_path = fixture.root().join("src/Main.sol");
        let snapshot_text = fs::read_to_string(&file_path).expect("read snapshot text");
        fs::remove_file(&file_path).expect("remove file");

        let loader = VfsOverlayFileLoader::new(fixture.vfs_snapshot().clone());
        let canonical = loader
            .canonicalize_path(&file_path)
            .expect("canonicalize path");
        assert_eq!(canonical, file_path);

        let loaded = loader.load_file(&file_path).expect("load file");
        assert_eq!(loaded, snapshot_text);
    }

    #[cfg(unix)]
    #[test]
    fn file_id_for_path_resolves_symlinked_paths() {
        use std::os::unix::fs::symlink;

        let fixture = FixtureBuilder::new()
            .expect("fixture builder")
            .file(
                "src/real/Lib.sol",
                r#"
contract Lib {}
"#,
            )
            .build()
            .expect("fixture");

        let real_dir = fixture.root().join("src/real");
        let link_dir = fixture.root().join("src/link");
        symlink(&real_dir, &link_dir).expect("create symlink");

        let file_id = fixture.file_id("src/real/Lib.sol").expect("file id");
        let path_to_file_id = fixture
            .vfs_snapshot()
            .iter()
            .map(|(file_id, path)| (path.clone(), file_id))
            .collect::<HashMap<_, _>>();

        let symlink_path = link_dir.join("Lib.sol");
        let resolved = super::file_id_for_path(&symlink_path, &path_to_file_id);

        assert_eq!(resolved, Some(file_id));
    }
}
