use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use foundry_compilers::{
    Language, ProjectPathsConfig,
    artifacts::sources::{Source, Sources},
    resolver::{Graph, parse::SolParser},
    solc::SolcLanguage,
};
use sa_paths::NormalizedPath;
use sa_project_model::{
    FoundryResolver, FoundryWorkspace, Remapping, ResolvedImport, project_paths_from_config,
    resolve_import_path_with_resolver,
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
    remappings: &[Remapping],
) -> anyhow::Result<IndexResult> {
    let mut result = IndexResult::default();
    let paths = project_paths_from_config(workspace, remappings)
        .with_context(|| "indexer: failed to build project paths")?;
    let sol_paths = paths.with_language::<SolcLanguage>();
    let sources = read_input_files_lenient(&sol_paths);
    let graph = Graph::<SolParser>::resolve_sources(&sol_paths, sources)
        .with_context(|| "indexer: failed to resolve workspace sources")?;

    for node in &graph.nodes {
        result.files.push(IndexedFile {
            path: NormalizedPath::new(node.path().to_string_lossy()),
            text: node.content().to_string(),
        });
    }

    result
        .files
        .sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    Ok(result)
}

pub fn index_open_file_imports(
    workspace: &FoundryWorkspace,
    remappings: &[Remapping],
    open_path: &NormalizedPath,
    open_text: &str,
) -> anyhow::Result<IndexResult> {
    let resolver = FoundryResolver::new(workspace, remappings)?;
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

        for resolved in resolved_import_paths(workspace, remappings, &resolver, &path, &text, true)
        {
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
    remappings: &[Remapping],
    resolver: &FoundryResolver,
    current_path: &NormalizedPath,
    text: &str,
    allow_solar_fallback: bool,
) -> Vec<NormalizedPath> {
    match resolver.resolved_imports(current_path, text) {
        Ok(imports) => {
            resolved_imports_with_resolver(workspace, remappings, resolver, current_path, imports)
        }
        Err(error) => {
            debug!(
                ?error,
                path = %current_path,
                "indexer: failed to parse imports with foundry parser"
            );
            if allow_solar_fallback {
                sa_syntax::parse_imports(text)
                    .into_iter()
                    .filter_map(|path| {
                        resolve_import_path_with_resolver(
                            workspace,
                            remappings,
                            current_path,
                            &path,
                            Some(resolver),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            }
        }
    }
}

fn resolved_imports_with_resolver(
    workspace: &FoundryWorkspace,
    remappings: &[Remapping],
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
                    remappings,
                    current_path,
                    &import.path,
                    Some(resolver),
                )
            })
        })
        .collect()
}

fn read_input_files_lenient<Lang>(paths: &ProjectPathsConfig<Lang>) -> Sources
where
    Lang: Language,
{
    let mut sources = Sources::new();

    for file in paths.input_files_iter() {
        match Source::read(&file) {
            Ok(source) => {
                sources.insert(file.to_path_buf(), source);
            }
            Err(error) => {
                warn!(?error, path = %file.display(), "indexer: failed to read input file");
            }
        }
    }

    sources
}

#[cfg(test)]
mod tests {
    use std::fs;

    use sa_paths::NormalizedPath;
    use sa_project_model::{FoundryResolver, FoundryWorkspace, Remapping, ResolvedImport};
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
        let remappings = vec![Remapping::new("lib/", "lib/forge-std/src/")];
        let workspace = FoundryWorkspace::new(root_path);

        let result = index_workspace(&workspace, &remappings).expect("index workspace");

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
        let remappings = Vec::new();
        let workspace = FoundryWorkspace::new(root_path);

        let result = index_workspace(&workspace, &remappings).expect("index workspace");

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].text, main_text);
    }

    #[test]
    fn indexer_handles_unresolved_imports_without_failing() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("src")).expect("src dir");

        let main_text = r#"
import "./Missing.sol";
contract Main {}
"#;
        let main_path = root.join("src/Main.sol");
        fs::write(&main_path, main_text).expect("write main");

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let remappings = Vec::new();
        let workspace = FoundryWorkspace::new(root_path);

        let result = index_workspace(&workspace, &remappings).expect("index workspace");

        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(main_path.to_string_lossy())
        ));
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn indexer_resolves_absolute_imports_under_src() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("src")).expect("src dir");

        let dep_text = "contract Dep {}";
        let dep_path = root.join("src/Dep.sol");
        fs::write(&dep_path, dep_text).expect("write dep");

        let main_text = r#"
import "src/Dep.sol";
contract Main { Dep dep; }
"#;
        let main_path = root.join("src/Main.sol");
        fs::write(&main_path, main_text).expect("write main");

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let remappings = Vec::new();
        let workspace = FoundryWorkspace::new(root_path);

        let result = index_workspace(&workspace, &remappings).expect("index workspace");

        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(main_path.to_string_lossy())
        ));
        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(dep_path.to_string_lossy())
        ));
    }

    #[test]
    fn indexer_skips_unreadable_sources() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("src")).expect("src dir");

        let good_path = root.join("src/Good.sol");
        fs::write(&good_path, "contract Good {}").expect("write good");

        let bad_path = root.join("src/Bad.sol");
        fs::write(&bad_path, [0xff, 0xfe, 0xfd]).expect("write bad");

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let remappings = Vec::new();
        let workspace = FoundryWorkspace::new(root_path);

        let result = index_workspace(&workspace, &remappings).expect("index workspace");

        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(good_path.to_string_lossy())
        ));
        assert!(!result_contains_path(
            &result,
            &NormalizedPath::new(bad_path.to_string_lossy())
        ));
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
        let remappings = vec![
            Remapping::new("dep/", "lib/foo/dep/").with_context("lib/foo"),
            Remapping::new("dep/", "lib/default/dep/"),
        ];
        let workspace = FoundryWorkspace::new(root_path);
        let open_path = NormalizedPath::new(root.join("lib/foo/src/Main.sol").to_string_lossy());

        let result = super::index_open_file_imports(&workspace, &remappings, &open_path, open_text)
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
    fn index_open_file_imports_use_open_buffer_text() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");

        fs::create_dir_all(root.join("src")).expect("src dir");

        let dep_text = "contract Dep {}";
        let dep_path = root.join("src/Dep.sol");
        fs::write(&dep_path, dep_text).expect("write dep");

        let disk_text = "contract Main {}";
        let open_text = r#"
import "./Dep.sol";
contract Main { Dep dep; }
"#;
        let main_path = root.join("src/Main.sol");
        fs::write(&main_path, disk_text).expect("write main");

        let root_path = NormalizedPath::new(root.to_string_lossy());
        let remappings = Vec::new();
        let workspace = FoundryWorkspace::new(root_path);
        let open_path = NormalizedPath::new(main_path.to_string_lossy());

        let result = super::index_open_file_imports(&workspace, &remappings, &open_path, open_text)
            .expect("index open file imports");

        assert!(result_contains_path(
            &result,
            &NormalizedPath::new(dep_path.to_string_lossy())
        ));
        let main_entry = result
            .files
            .iter()
            .find(|file| file.path == open_path)
            .expect("main entry");
        assert_eq!(main_entry.text, open_text);
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
        let remappings = vec![Remapping::new(
            "@openzeppelin/contracts/",
            "lib/openzeppelin-contracts/contracts/",
        )];
        let workspace = FoundryWorkspace::new(root_path);
        let open_path = NormalizedPath::new(root.join("src/Main.sol").to_string_lossy());
        let resolver = FoundryResolver::new(&workspace, &remappings).expect("resolver");

        let mut imports = resolver
            .resolved_imports(&open_path, open_text)
            .expect("resolved imports");
        for import in &mut imports {
            import.resolved_path = None;
        }

        let resolved = super::resolved_imports_with_resolver(
            &workspace,
            &remappings,
            &resolver,
            &open_path,
            imports,
        );

        let expected = NormalizedPath::new(
            root.join(
                "lib/openzeppelin-contracts/contracts/token/ERC20/extensions/IERC20Permit.sol",
            )
            .to_string_lossy(),
        );
        assert!(resolved.iter().any(|path| path == &expected));
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
        let remappings = Vec::new();
        let workspace = FoundryWorkspace::new(root_path);
        let open_path = NormalizedPath::new(root.join("src/Main.sol").to_string_lossy());

        let result = super::index_open_file_imports(&workspace, &remappings, &open_path, open_text)
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
        let remappings = Vec::new();
        let workspace = FoundryWorkspace::new(root_path);
        let resolver = FoundryResolver::new(&workspace, &remappings).expect("resolver");
        let current_path = NormalizedPath::new("Main.sol");

        let imports = vec![ResolvedImport {
            path: "../Missing.sol".to_string(),
            resolved_path: None,
            aliases: Vec::new(),
        }];

        let resolved = super::resolved_imports_with_resolver(
            &workspace,
            &remappings,
            &resolver,
            &current_path,
            imports,
        );
        assert!(resolved.is_empty());
    }
}
