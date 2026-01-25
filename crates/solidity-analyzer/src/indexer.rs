use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use sa_paths::NormalizedPath;
use sa_project_model::{
    FoundryResolver, FoundryWorkspace, ResolvedImport, resolve_import_path_with_resolver,
};
use tracing::{debug, warn};

use crate::lsp_utils;

/// An indexed file with its path and contents.
#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub path: NormalizedPath,
    pub text: String,
}

/// Result of indexing a workspace.
#[derive(Debug, Default)]
pub struct IndexResult {
    pub files: Vec<IndexedFile>,
}

impl IndexResult {
    /// Returns an iterator over the paths of indexed files.
    #[cfg(test)]
    pub fn paths(&self) -> impl Iterator<Item = &NormalizedPath> {
        self.files.iter().map(|f| &f.path)
    }
}

pub fn index_workspace(
    workspace: &FoundryWorkspace,
    profile: Option<&str>,
) -> anyhow::Result<IndexResult> {
    let resolver = FoundryResolver::new(workspace, profile)?;
    let mut result = IndexResult::default();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    // Cache for file existence checks to avoid repeated stat calls.
    let mut file_exists_cache: HashMap<String, bool> = HashMap::new();

    for root in [workspace.src(), workspace.test(), workspace.script()] {
        let root_path = PathBuf::from(root.as_str());
        for entry in collect_solidity_files(&root_path)? {
            let normalized = NormalizedPath::new(entry.to_string_lossy());
            let key = normalized.as_str().to_string();
            file_exists_cache.insert(key, true);
            if seen.insert(normalized.clone()) {
                queue.push_back(normalized);
            }
        }
    }

    while let Some(path) = queue.pop_front() {
        let text = match fs::read_to_string(path.as_str()) {
            Ok(text) => text,
            Err(error) => {
                debug!(?error, path = %path, "indexer: failed to read file");
                continue;
            }
        };

        for resolved in resolved_import_paths(workspace, &resolver, &path, &text) {
            let resolved_str = resolved.as_str().to_string();
            let exists = *file_exists_cache
                .entry(resolved_str.clone())
                .or_insert_with(|| PathBuf::from(&resolved_str).is_file());
            if !exists {
                continue;
            }
            if seen.insert(resolved.clone()) {
                queue.push_back(resolved);
            }
        }

        // Store the file with its content.
        result.files.push(IndexedFile { path, text });
    }

    result
        .files
        .sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    Ok(result)
}

pub fn index_open_file_imports(
    workspace: &FoundryWorkspace,
    profile: Option<&str>,
    open_path: &NormalizedPath,
    open_text: &str,
) -> anyhow::Result<IndexResult> {
    let resolver = FoundryResolver::new(workspace, profile)?;
    let mut result = IndexResult::default();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();

    let open_path = lsp_utils::normalize_path(Path::new(open_path.as_str()));
    queue.push_back((open_path, Some(open_text.to_string())));

    while let Some((path, text_override)) = queue.pop_front() {
        if !seen.insert(path.clone()) {
            continue;
        }

        let text = match text_override {
            Some(text) => text,
            None => match fs::read_to_string(path.as_str()) {
                Ok(text) => text,
                Err(error) => {
                    debug!(?error, path = %path, "indexer: failed to read file");
                    continue;
                }
            },
        };

        for resolved in resolved_import_paths(workspace, &resolver, &path, &text) {
            let resolved = lsp_utils::normalize_path(Path::new(resolved.as_str()));
            if !PathBuf::from(resolved.as_str()).is_file() {
                continue;
            }
            if !seen.contains(&resolved) {
                queue.push_back((resolved, None));
            }
        }

        result.files.push(IndexedFile { path, text });
    }

    result
        .files
        .sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    Ok(result)
}

fn resolved_import_paths(
    workspace: &FoundryWorkspace,
    resolver: &FoundryResolver,
    current_path: &NormalizedPath,
    text: &str,
) -> Vec<NormalizedPath> {
    match resolver.resolved_imports(current_path, text) {
        Ok(imports) => resolved_imports_with_resolver(workspace, resolver, current_path, imports),
        Err(error) => {
            debug!(
                ?error,
                path = %current_path,
                "indexer: failed to parse imports with foundry parser"
            );
            sa_syntax::parse_imports(text)
                .into_iter()
                .filter_map(|path| {
                    resolve_import_path_with_resolver(
                        workspace,
                        current_path,
                        &path,
                        Some(resolver),
                    )
                })
                .collect()
        }
    }
}

fn resolved_imports_with_resolver(
    workspace: &FoundryWorkspace,
    resolver: &FoundryResolver,
    current_path: &NormalizedPath,
    imports: Vec<ResolvedImport>,
) -> Vec<NormalizedPath> {
    imports
        .into_iter()
        .filter_map(|import| {
            import.resolved_path.or_else(|| {
                resolve_import_path_with_resolver(
                    workspace,
                    current_path,
                    &import.path,
                    Some(resolver),
                )
            })
        })
        .collect()
}

fn collect_solidity_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    // Track visited directories by canonical path to avoid symlink loops.
    let mut visited_dirs: HashSet<PathBuf> = HashSet::new();

    // Canonicalize and track the root directory.
    match fs::canonicalize(root) {
        Ok(canonical_root) => {
            visited_dirs.insert(canonical_root);
        }
        Err(error) => {
            warn!(
                ?error,
                root = %root.display(),
                "indexer: failed to canonicalize root directory, symlink loop detection may be impacted"
            );
        }
    }

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                warn!(?error, dir = %dir.display(), "indexer: failed to read directory");
                continue;
            }
        };

        let mut sorted_entries = Vec::new();
        for entry in entries {
            match entry {
                Ok(e) => sorted_entries.push(e),
                Err(error) => {
                    warn!(?error, dir = %dir.display(), "indexer: failed to read directory entry");
                    continue;
                }
            }
        }
        sorted_entries.sort_by_key(|entry| entry.path());

        for entry in sorted_entries {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(error) => {
                    warn!(?error, path = %path.display(), "indexer: failed to read metadata");
                    continue;
                }
            };

            if metadata.is_dir() {
                // Check for symlink loops by canonicalizing the path.
                match fs::canonicalize(&path) {
                    Ok(canonical) => {
                        if visited_dirs.insert(canonical) {
                            stack.push(path);
                        } else {
                            debug!(path = %path.display(), "indexer: skipping already-visited directory (symlink loop)");
                        }
                    }
                    Err(error) => {
                        warn!(?error, path = %path.display(), "indexer: failed to canonicalize directory");
                        continue;
                    }
                }
            } else if metadata.is_file() && path.extension().is_some_and(|ext| ext == "sol") {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use sa_paths::NormalizedPath;
    use sa_project_model::{
        FoundryProfile, FoundryResolver, FoundryWorkspace, Remapping, ResolvedImport,
    };
    use tempfile::tempdir;

    use super::index_workspace;

    fn result_contains_path(result: &super::IndexResult, path: &NormalizedPath) -> bool {
        result.paths().any(|p| p == path)
    }

    #[test]
    fn indexer_includes_entry_roots_and_imported_libs() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::create_dir_all(root.join("test")).expect("test dir");
        fs::create_dir_all(root.join("script")).expect("script dir");
        fs::create_dir_all(root.join("lib/forge-std/src")).expect("lib dir");
        fs::create_dir_all(root.join("lib/unused")).expect("unused lib dir");

        let main_text = r#"
import "lib/DepLib.sol";

contract Main {
    DepLib dep;
}
"#;
        let test_text = r#"
contract MainTest {}
"#;
        let script_text = r#"
contract DeployScript {}
"#;
        let dep_text = r#"
contract DepLib {}
"#;
        let unused_text = r#"
contract Unused {}
"#;

        fs::write(root.join("src/Main.sol"), main_text).expect("write main");
        fs::write(root.join("test/Main.t.sol"), test_text).expect("write test");
        fs::write(root.join("script/Deploy.sol"), script_text).expect("write script");
        fs::write(root.join("lib/forge-std/src/DepLib.sol"), dep_text).expect("write dep");
        fs::write(root.join("lib/unused/Unused.sol"), unused_text).expect("write unused");

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let profile = FoundryProfile::new("default")
            .with_remappings(vec![Remapping::new("lib/", "lib/forge-std/src/")]);
        let workspace = FoundryWorkspace::new(root_path, profile);

        let result = index_workspace(&workspace, None).expect("index workspace");

        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(root.join("src/Main.sol").to_string_lossy())
        ));
        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(root.join("test/Main.t.sol").to_string_lossy())
        ));
        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(root.join("script/Deploy.sol").to_string_lossy())
        ));
        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(root.join("lib/forge-std/src/DepLib.sol").to_string_lossy())
        ));
        assert!(!result_contains_path(
            &result,
            &NormalizedPath::new(root.join("lib/unused/Unused.sol").to_string_lossy())
        ));
    }

    #[test]
    fn indexer_returns_file_contents() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("src")).expect("src dir");

        let main_text = "contract Main {}";
        fs::write(root.join("src/Main.sol"), main_text).expect("write main");

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root_path, profile);

        let result = index_workspace(&workspace, None).expect("index workspace");

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].text, main_text);
    }

    #[test]
    fn indexer_resolves_context_remapped_imports_for_open_file() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("lib/foo/src")).expect("foo src dir");
        fs::create_dir_all(root.join("lib/foo/dep")).expect("foo dep dir");
        fs::create_dir_all(root.join("lib/default/dep")).expect("default dep dir");

        let open_text = r#"
import "dep/Thing.sol";

contract Main {}
"#;
        let dep_text = r#"
contract Thing {}
"#;
        fs::write(root.join("lib/foo/dep/Thing.sol"), dep_text).expect("write foo dep");
        fs::write(root.join("lib/default/dep/Thing.sol"), dep_text).expect("write default dep");

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let profile = FoundryProfile::new("default").with_remappings(vec![
            Remapping::new("dep/", "lib/default/dep/"),
            Remapping::new("dep/", "lib/foo/dep/").with_context("lib/foo"),
        ]);
        let workspace = FoundryWorkspace::new(root_path, profile);
        let open_path = NormalizedPath::new(root.join("lib/foo/src/Main.sol").to_string_lossy());

        let result = super::index_open_file_imports(&workspace, None, &open_path, open_text)
            .expect("index open file imports");

        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(root.join("lib/foo/dep/Thing.sol").to_string_lossy())
        ));
        assert!(!result_contains_path(
            &result,
            &NormalizedPath::new(root.join("lib/default/dep/Thing.sol").to_string_lossy())
        ));
    }

    #[test]
    fn indexer_falls_back_when_resolved_import_path_missing() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::create_dir_all(
            root.join("lib/openzeppelin-contracts/contracts/token/ERC20/extensions"),
        )
        .expect("lib dir");

        let open_text = r#"
import "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";

contract Main {}
"#;
        fs::write(
            root.join(
                "lib/openzeppelin-contracts/contracts/token/ERC20/extensions/IERC20Permit.sol",
            ),
            "interface IERC20Permit {}",
        )
        .expect("write dep");

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let profile = FoundryProfile::new("default").with_remappings(vec![Remapping::new(
            "@openzeppelin/contracts/",
            "lib/openzeppelin-contracts/contracts/",
        )]);
        let workspace = FoundryWorkspace::new(root_path, profile);
        let open_path = NormalizedPath::new(root.join("src/Main.sol").to_string_lossy());
        let resolver = FoundryResolver::new(&workspace, None).expect("resolver");

        let mut imports = resolver
            .resolved_imports(&open_path, open_text)
            .expect("resolved imports");
        for import in &mut imports {
            import.resolved_path = None;
        }

        let resolved =
            super::resolved_imports_with_resolver(&workspace, &resolver, &open_path, imports);

        let expected = NormalizedPath::new(
            root.join(
                "lib/openzeppelin-contracts/contracts/token/ERC20/extensions/IERC20Permit.sol",
            )
            .to_string_lossy(),
        );
        assert!(resolved.iter().any(|path| path == &expected));
    }

    #[test]
    fn collect_solidity_files_returns_empty_for_missing_root() {
        let temp = tempdir().expect("tempdir");
        let missing = temp.path().join("missing");
        let files = super::collect_solidity_files(&missing).expect("collect files");
        assert!(files.is_empty());
    }

    #[test]
    fn collect_solidity_files_handles_non_directory_root() {
        let temp = tempdir().expect("tempdir");
        let root_file = temp.path().join("NotADir.sol");
        fs::write(&root_file, "contract NotADir {}").expect("write file");

        let files = super::collect_solidity_files(&root_file).expect("collect files");
        assert!(files.is_empty());
    }

    #[test]
    fn index_open_file_imports_skips_missing_imports() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("src")).expect("src dir");

        let open_text = r#"
import "./Missing.sol";

contract Main {}
"#;

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root_path, profile);
        let open_path = NormalizedPath::new(root.join("src/Main.sol").to_string_lossy());

        let result = super::index_open_file_imports(&workspace, None, &open_path, open_text)
            .expect("index open file imports");

        assert!(result_contains_path(&result, &open_path));
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn resolved_imports_with_resolver_skips_unresolved_imports() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("src")).expect("src dir");

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root_path, profile);
        let resolver = FoundryResolver::new(&workspace, None).expect("resolver");
        let current_path = NormalizedPath::new("Main.sol");

        let imports = vec![ResolvedImport {
            path: "../Missing.sol".to_string(),
            resolved_path: None,
            aliases: Vec::new(),
        }];

        let resolved =
            super::resolved_imports_with_resolver(&workspace, &resolver, &current_path, imports);
        assert!(resolved.is_empty());
    }
}
