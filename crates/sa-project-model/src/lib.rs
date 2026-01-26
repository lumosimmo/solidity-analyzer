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
use sa_paths::NormalizedPath;

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
        let remappings = resolved_profile.remappings().to_vec();
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
    let resolver = FoundryResolver::new(workspace, None).ok()?;
    resolver.resolve_import_path(current_path, import_path)
}

pub fn resolve_import_path_with_resolver(
    workspace: &FoundryWorkspace,
    current_path: &NormalizedPath,
    import_path: &str,
    resolver: Option<&FoundryResolver>,
) -> Option<NormalizedPath> {
    if let Some(resolver) = resolver {
        resolver.resolve_import_path(current_path, import_path)
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
    let resolver = FoundryResolver::new(workspace, profile).ok()?;
    resolver.resolve_import_path(current_path, import_path)
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
    use super::{FoundryProfile, FoundryResolver, FoundryWorkspace, Remapping};
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
    fn resolve_import_path_with_profile_delegates_to_foundry() {
        let root = NormalizedPath::new("/workspace");
        let default_profile = FoundryProfile::new("default")
            .with_remappings(vec![Remapping::new("lib/", "lib/default/")]);

        let mut workspace = FoundryWorkspace::new(root, default_profile);
        let dev_profile =
            FoundryProfile::new("dev").with_remappings(vec![Remapping::new("lib/", "lib/dev/")]);
        workspace.add_profile(dev_profile);

        let current = NormalizedPath::new("/workspace/src/Main.sol");
        let resolver = FoundryResolver::new(&workspace, Some("dev")).expect("resolver");

        let resolved = super::resolve_import_path_with_profile(
            &workspace,
            &current,
            "lib/Foo.sol",
            Some("dev"),
        );
        let expected = resolver.resolve_import_path(&current, "lib/Foo.sol");
        assert_eq!(resolved, expected);
    }
}
