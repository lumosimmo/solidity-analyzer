use sa_paths::NormalizedPath;
use sa_project_model::{
    FoundryProfile, FoundryResolver, FoundryWorkspace, Remapping, resolve_import_path,
    resolve_import_path_with_profile, resolve_import_path_with_resolver,
};

mod helpers {
    use super::*;

    const ROOT: &str = "/workspace";

    pub fn workspace_with_remappings(remappings: Vec<Remapping>) -> FoundryWorkspace {
        let root = NormalizedPath::new(ROOT);
        let profile = FoundryProfile::new("default").with_remappings(remappings);
        FoundryWorkspace::new(root, profile)
    }

    pub fn workspace_path(path: &str) -> NormalizedPath {
        NormalizedPath::new(format!("{ROOT}/{path}"))
    }

    pub fn resolver_for_workspace(workspace: &FoundryWorkspace) -> FoundryResolver {
        FoundryResolver::new(workspace, None).expect("resolver")
    }
}

#[test]
fn named_profile_inherits_default_remappings_when_empty() {
    let default_remappings = vec![Remapping::new("dep/", "lib/dep/")];
    let mut workspace = helpers::workspace_with_remappings(default_remappings.clone());
    workspace.add_profile(FoundryProfile::new("dev"));

    let resolved = workspace.profile(Some("dev"));

    assert_eq!(resolved.remappings(), default_remappings.as_slice());
}

#[test]
fn missing_profile_returns_default() {
    let workspace = helpers::workspace_with_remappings(vec![Remapping::new("dep/", "lib/dep/")]);

    let resolved = workspace.profile(Some("unknown"));

    assert_eq!(resolved.name(), "default");
    assert_eq!(resolved.remappings().len(), 1);
}

#[test]
fn resolve_import_path_defaults_profile_and_uses_remappings() {
    let workspace = helpers::workspace_with_remappings(vec![Remapping::new("dep/", "lib/dep/")]);
    let importer = helpers::workspace_path("src/Main.sol");

    let resolved = resolve_import_path(&workspace, &importer, "dep/Thing.sol");

    assert_eq!(resolved, Some(helpers::workspace_path("lib/dep/Thing.sol")));
}

#[test]
fn resolve_import_path_with_resolver_prefers_adapter_when_available() {
    let workspace = helpers::workspace_with_remappings(vec![Remapping::new("dep/", "lib/dep/")]);
    let resolver = helpers::resolver_for_workspace(&workspace);
    let importer = helpers::workspace_path("src/Main.sol");

    let resolved =
        resolve_import_path_with_resolver(&workspace, &importer, "dep/Thing.sol", Some(&resolver));

    assert_eq!(
        resolved,
        resolver.resolve_import_path(&importer, "dep/Thing.sol")
    );
}

#[test]
fn resolve_import_path_with_resolver_falls_back_without_resolver() {
    let workspace = helpers::workspace_with_remappings(vec![Remapping::new("dep/", "lib/dep/")]);
    let importer = helpers::workspace_path("src/Main.sol");

    let resolved = resolve_import_path_with_resolver(&workspace, &importer, "dep/Thing.sol", None);

    assert_eq!(
        resolved,
        resolve_import_path(&workspace, &importer, "dep/Thing.sol")
    );
}

#[test]
fn remapping_skips_shorter_contexts_after_match() {
    let workspace = helpers::workspace_with_remappings(vec![
        Remapping::new("dep/", "lib/foo/dep/").with_context("lib/foo"),
        Remapping::new("dep/", "lib/default/dep/"),
    ]);
    let importer = helpers::workspace_path("lib/foo/src/Main.sol");

    let resolved = resolve_import_path_with_profile(&workspace, &importer, "dep/Thing.sol", None);

    assert_eq!(
        resolved,
        Some(helpers::workspace_path("lib/foo/dep/Thing.sol"))
    );
}

#[test]
fn remapping_ignores_non_matching_prefix_before_match() {
    let workspace = helpers::workspace_with_remappings(vec![
        Remapping::new("lib/", "lib/default/"),
        Remapping::new("dep/", "lib/dep/"),
    ]);
    let importer = helpers::workspace_path("src/Main.sol");

    let resolved = resolve_import_path_with_profile(&workspace, &importer, "dep/Thing.sol", None);

    assert_eq!(resolved, Some(helpers::workspace_path("lib/dep/Thing.sol")));
}
