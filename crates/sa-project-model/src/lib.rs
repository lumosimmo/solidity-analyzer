use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use foundry_compilers::{
    ProjectPathsConfig, SourceParser,
    artifacts::{
        remappings::Remapping as FoundryRemapping,
        sources::{Source, Sources},
    },
    resolver::{SolImportAlias, parse::SolParser},
};
use sa_paths::{NormalizedPath, WorkspacePath};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remapping {
    context: Option<String>,
    from: String,
    to: String,
}

impl Remapping {
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            context: None,
            from: from.into(),
            to: to.into(),
        }
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    pub fn context(&self) -> Option<&str> {
        self.context.as_deref()
    }

    pub fn from(&self) -> &str {
        &self.from
    }

    pub fn to(&self) -> &str {
        &self.to
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundryProfile {
    name: String,
    solc_version: Option<String>,
    remappings: Vec<Remapping>,
}

impl FoundryProfile {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            solc_version: None,
            remappings: Vec::new(),
        }
    }

    pub fn with_solc_version(mut self, version: impl Into<String>) -> Self {
        self.solc_version = Some(version.into());
        self
    }

    pub fn with_remappings(mut self, remappings: Vec<Remapping>) -> Self {
        self.remappings = remappings;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn solc_version(&self) -> Option<&str> {
        self.solc_version.as_deref()
    }

    pub fn remappings(&self) -> &[Remapping] {
        &self.remappings
    }

    fn overlay(default: &FoundryProfile, named: &FoundryProfile) -> FoundryProfile {
        let remappings = if named.remappings.is_empty() {
            default.remappings.clone()
        } else {
            named.remappings.clone()
        };

        FoundryProfile {
            name: named.name.clone(),
            solc_version: named
                .solc_version
                .clone()
                .or_else(|| default.solc_version.clone()),
            remappings,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundryWorkspace {
    root: NormalizedPath,
    src: NormalizedPath,
    lib: NormalizedPath,
    test: NormalizedPath,
    script: NormalizedPath,
    default_profile: FoundryProfile,
    profiles: HashMap<String, FoundryProfile>,
}

impl FoundryWorkspace {
    pub fn new(root: NormalizedPath, default_profile: FoundryProfile) -> Self {
        let root_str = root.as_str();
        let src = NormalizedPath::new(format!("{root_str}/src"));
        let lib = NormalizedPath::new(format!("{root_str}/lib"));
        let test = NormalizedPath::new(format!("{root_str}/test"));
        let script = NormalizedPath::new(format!("{root_str}/script"));

        Self::from_paths(root, src, lib, test, script, default_profile)
    }

    pub fn from_paths(
        root: NormalizedPath,
        src: NormalizedPath,
        lib: NormalizedPath,
        test: NormalizedPath,
        script: NormalizedPath,
        default_profile: FoundryProfile,
    ) -> Self {
        Self {
            root,
            src,
            lib,
            test,
            script,
            default_profile,
            profiles: HashMap::new(),
        }
    }

    pub fn root(&self) -> &NormalizedPath {
        &self.root
    }

    pub fn src(&self) -> &NormalizedPath {
        &self.src
    }

    pub fn lib(&self) -> &NormalizedPath {
        &self.lib
    }

    pub fn test(&self) -> &NormalizedPath {
        &self.test
    }

    pub fn script(&self) -> &NormalizedPath {
        &self.script
    }

    pub fn add_profile(&mut self, profile: FoundryProfile) {
        self.profiles.insert(profile.name().to_string(), profile);
    }

    pub fn profile(&self, name: Option<&str>) -> FoundryProfile {
        let name = match name {
            None | Some("default") => return self.default_profile.clone(),
            Some(name) => name,
        };

        match self.profiles.get(name) {
            Some(profile) => FoundryProfile::overlay(&self.default_profile, profile),
            None => self.default_profile.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedImport {
    pub path: String,
    pub resolved_path: Option<NormalizedPath>,
    pub aliases: Vec<SolImportAlias>,
}

#[derive(Clone, Debug)]
pub struct FoundryResolver {
    paths: ProjectPathsConfig,
}

impl FoundryResolver {
    pub fn new(workspace: &FoundryWorkspace, profile: Option<&str>) -> Result<Self> {
        let resolved_profile = workspace.profile(profile);
        let mut remappings = resolved_profile.remappings().to_vec();
        remappings.sort_by(|left, right| {
            let left_context = left.context().map(|context| context.len()).unwrap_or(0);
            let right_context = right.context().map(|context| context.len()).unwrap_or(0);
            right_context
                .cmp(&left_context)
                .then_with(|| right.from().len().cmp(&left.from().len()))
        });
        let paths = project_paths_from_config(workspace, &remappings)?;
        Ok(Self { paths })
    }

    pub fn resolve_import_path(
        &self,
        current_path: &NormalizedPath,
        import_path: &str,
    ) -> Option<NormalizedPath> {
        let import_path = normalize_import_path(import_path);
        let import_path = Path::new(import_path.as_ref());
        let current = Path::new(current_path.as_str());
        let cwd = current.parent().unwrap_or_else(|| Path::new("."));
        let resolved = self.paths.resolve_import(cwd, import_path).ok()?;
        Some(NormalizedPath::new(resolved.to_string_lossy()))
    }

    pub fn resolved_imports(
        &self,
        current_path: &NormalizedPath,
        text: &str,
    ) -> Result<Vec<ResolvedImport>> {
        let mut parser = SolParser::new(&self.paths);
        let current_norm = current_path.clone();
        let current_path_buf = PathBuf::from(current_norm.as_str());
        let mut sources = Sources::from_iter([(current_path_buf.clone(), Source::new(text))]);
        let mut nodes = parser
            .parse_sources(&mut sources)
            .with_context(|| "failed to parse imports with foundry parser")?;

        let (_, node) = nodes.pop().with_context(|| "missing parsed import data")?;
        let mut imports = Vec::new();
        for import in node.data.imports {
            let path = import.data.path().to_string_lossy().to_string();
            let resolved_path = self.resolve_import_path(&current_norm, &path);
            imports.push(ResolvedImport {
                path,
                resolved_path,
                aliases: import.data.aliases().to_vec(),
            });
        }
        Ok(imports)
    }
}

pub fn project_paths_from_config(
    workspace: &FoundryWorkspace,
    remappings: &[Remapping],
) -> Result<ProjectPathsConfig> {
    let root = PathBuf::from(workspace.root().as_str());
    let sources = PathBuf::from(workspace.src().as_str());
    let tests = PathBuf::from(workspace.test().as_str());
    let scripts = PathBuf::from(workspace.script().as_str());
    let libs = vec![PathBuf::from(workspace.lib().as_str())];

    let mut builder = ProjectPathsConfig::builder()
        .root(&root)
        .sources(sources)
        .tests(tests)
        .scripts(scripts)
        .libs(libs);

    for remapping in remappings {
        builder = builder.remapping(FoundryRemapping {
            context: remapping.context().map(|context| context.to_string()),
            name: remapping.from().to_string(),
            path: remapping.to().to_string(),
        });
    }

    builder
        .build()
        .with_context(|| "failed to build project paths config")
}

pub fn resolve_import_path(
    workspace: &FoundryWorkspace,
    current_path: &NormalizedPath,
    import_path: &str,
) -> Option<NormalizedPath> {
    resolve_import_path_with_profile(workspace, current_path, import_path, None)
}

pub fn resolve_import_path_with_resolver(
    workspace: &FoundryWorkspace,
    current_path: &NormalizedPath,
    import_path: &str,
    resolver: Option<&FoundryResolver>,
) -> Option<NormalizedPath> {
    if let Some(resolver) = resolver {
        resolver
            .resolve_import_path(current_path, import_path)
            .or_else(|| resolve_import_path(workspace, current_path, import_path))
    } else {
        resolve_import_path(workspace, current_path, import_path)
    }
}

pub fn resolve_import_path_with_profile(
    workspace: &FoundryWorkspace,
    current_path: &NormalizedPath,
    import_path: &str,
    profile: Option<&str>,
) -> Option<NormalizedPath> {
    let import_path = normalize_import_path(import_path);
    let import_path = import_path.as_ref();
    if import_path.starts_with("./") || import_path.starts_with("../") {
        let parent = current_path
            .as_str()
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or(".");
        if parent == "." {
            // Bare filename with no parent directory.
            // `./foo` can strip the prefix and anchor to workspace root.
            // `../foo` is unresolvable since there's no parent to go up from.
            if import_path.starts_with("../") {
                return None;
            }
            let combined = import_path.trim_start_matches("./").to_string();
            return Some(join_workspace_path(workspace, &combined));
        }
        let combined = format!("{parent}/{import_path}");
        return Some(join_workspace_path(workspace, &combined));
    }

    let resolved_profile = workspace.profile(profile);
    let context = remapping_context(workspace, current_path);
    if let Some(remapped) = remap_import_path(import_path, &context, resolved_profile.remappings())
    {
        return Some(join_workspace_path(workspace, &remapped));
    }

    Some(join_workspace_path(workspace, import_path))
}

fn remapping_context(workspace: &FoundryWorkspace, current_path: &NormalizedPath) -> String {
    WorkspacePath::new(workspace.root(), current_path)
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| current_path.as_str().to_string())
}

fn remap_import_path(import_path: &str, context: &str, remappings: &[Remapping]) -> Option<String> {
    let mut longest_context = 0usize;
    let mut longest_prefix = 0usize;
    let mut best_target = None;
    let mut unprefixed = None;

    for remapping in remappings {
        let remap_context = remapping.context().unwrap_or("");
        if remap_context.len() < longest_context {
            continue;
        }
        if !context.starts_with(remap_context) {
            continue;
        }
        if remap_context.len() == longest_context && remapping.from().len() < longest_prefix {
            continue;
        }
        let stripped = match import_path.strip_prefix(remapping.from()) {
            Some(stripped) => stripped,
            None => continue,
        };

        longest_context = remap_context.len();
        longest_prefix = remapping.from().len();
        best_target = Some(remapping.to());
        unprefixed = Some(stripped);
    }

    let best_target = best_target?;
    let unprefixed = unprefixed?;
    Some(format!("{best_target}{unprefixed}"))
}

fn join_workspace_path(workspace: &FoundryWorkspace, path: &str) -> NormalizedPath {
    if is_absolute_path(path) {
        NormalizedPath::new(path)
    } else {
        NormalizedPath::new(format!("{}/{}", workspace.root().as_str(), path))
    }
}

fn is_absolute_path(path: &str) -> bool {
    if path.starts_with("\\\\") || path.starts_with("//") {
        return true;
    }

    if path.starts_with('/') {
        return true;
    }

    // Windows absolute paths: require drive letter + ':' + path separator (/ or \)
    // "C:/..." or "C:\..." are absolute, but "C:foo" is drive-relative (not absolute)
    path.chars().next().map(|c| c.is_ascii_alphabetic()) == Some(true)
        && path.chars().nth(1) == Some(':')
        && path.chars().nth(2).is_some_and(|c| c == '/' || c == '\\')
}

fn normalize_import_path(path: &str) -> std::borrow::Cow<'_, str> {
    if path.contains('\\') {
        std::borrow::Cow::Owned(path.replace('\\', "/"))
    } else {
        std::borrow::Cow::Borrowed(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FoundryProfile, FoundryWorkspace, Remapping, is_absolute_path,
        resolve_import_path_with_profile,
    };
    use sa_paths::NormalizedPath;

    #[test]
    fn workspace_paths_are_wired() {
        let root = NormalizedPath::new("/workspace");
        let default_profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root, default_profile);

        assert_eq!(workspace.src().as_str(), "/workspace/src");
        assert_eq!(workspace.lib().as_str(), "/workspace/lib");
        assert_eq!(workspace.test().as_str(), "/workspace/test");
        assert_eq!(workspace.script().as_str(), "/workspace/script");
    }

    #[test]
    fn profiles_overlay_default_settings() {
        let root = NormalizedPath::new("/workspace");
        let default_profile = FoundryProfile::new("default")
            .with_solc_version("0.8.20")
            .with_remappings(vec![Remapping::new("lib/", "lib/forge-std/")]);

        let mut workspace = FoundryWorkspace::new(root, default_profile.clone());
        let dev_profile = FoundryProfile::new("dev")
            .with_remappings(vec![Remapping::new("src/", "src/overrides/")]);
        workspace.add_profile(dev_profile);

        let resolved = workspace.profile(Some("dev"));
        assert_eq!(resolved.solc_version(), Some("0.8.20"));
        assert_eq!(resolved.remappings().len(), 1);
        assert_eq!(resolved.remappings()[0].from(), "src/");
        assert_eq!(resolved.remappings()[0].to(), "src/overrides/");
    }

    #[test]
    fn absolute_path_detection_respects_drive_letters() {
        assert!(is_absolute_path("/workspace/src/Main.sol"));
        assert!(is_absolute_path("C:/workspace/src/Main.sol"));
        assert!(is_absolute_path("c:\\workspace\\src\\Main.sol"));
        assert!(is_absolute_path(r"\\server\share\src\Main.sol"));
        assert!(is_absolute_path("//server/share/src/Main.sol"));
        assert!(is_absolute_path(r"\\?\C:\workspace\src\Main.sol"));
        assert!(!is_absolute_path(":/workspace/src/Main.sol"));
        assert!(!is_absolute_path("relative/path.sol"));
        // Drive-relative paths (C:foo) are not absolute
        assert!(!is_absolute_path("C:foo.sol"));
        assert!(!is_absolute_path("D:relative/path.sol"));
    }

    #[test]
    fn bare_filename_with_dotslash_import_anchors_to_workspace() {
        let root = NormalizedPath::new("/workspace");
        let profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root, profile);
        let current = NormalizedPath::new("Foo.sol");

        let resolved = resolve_import_path_with_profile(&workspace, &current, "./Bar.sol", None);
        assert_eq!(resolved, Some(NormalizedPath::new("/workspace/Bar.sol")));
    }

    #[test]
    fn backslash_relative_imports_are_normalized() {
        let root = NormalizedPath::new("/workspace");
        let profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root, profile);
        let current = NormalizedPath::new("/workspace/src/Foo.sol");

        let resolved =
            resolve_import_path_with_profile(&workspace, &current, r"..\lib\Bar.sol", None);
        assert_eq!(
            resolved,
            Some(NormalizedPath::new("/workspace/lib/Bar.sol"))
        );
    }

    #[test]
    fn bare_filename_with_dotdotslash_import_returns_none() {
        let root = NormalizedPath::new("/workspace");
        let profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root, profile);
        let current = NormalizedPath::new("Foo.sol");

        let resolved = resolve_import_path_with_profile(&workspace, &current, "../Bar.sol", None);
        assert!(resolved.is_none());
    }

    #[test]
    fn profile_aware_remapping_uses_specified_profile() {
        let root = NormalizedPath::new("/workspace");
        let default_profile = FoundryProfile::new("default")
            .with_remappings(vec![Remapping::new("lib/", "lib/default/")]);

        let mut workspace = FoundryWorkspace::new(root, default_profile);
        let dev_profile =
            FoundryProfile::new("dev").with_remappings(vec![Remapping::new("lib/", "lib/dev/")]);
        workspace.add_profile(dev_profile);

        let current = NormalizedPath::new("/workspace/src/Main.sol");

        // Default profile uses lib/default/
        let resolved_default =
            resolve_import_path_with_profile(&workspace, &current, "lib/Foo.sol", None);
        assert_eq!(
            resolved_default,
            Some(NormalizedPath::new("/workspace/lib/default/Foo.sol"))
        );

        // Dev profile uses lib/dev/
        let resolved_dev =
            resolve_import_path_with_profile(&workspace, &current, "lib/Foo.sol", Some("dev"));
        assert_eq!(
            resolved_dev,
            Some(NormalizedPath::new("/workspace/lib/dev/Foo.sol"))
        );
    }
}
