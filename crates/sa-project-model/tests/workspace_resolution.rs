use sa_paths::NormalizedPath;
use sa_project_model::{
    FoundryResolver, FoundryWorkspace, Remapping, resolve_import_path,
    resolve_import_path_with_resolver,
};

mod helpers {
    use super::*;

    const ROOT: &str = "/workspace";

    pub fn workspace() -> FoundryWorkspace {
        let root = NormalizedPath::new(ROOT);
        FoundryWorkspace::new(root)
    }

    pub fn workspace_path(path: &str) -> NormalizedPath {
        NormalizedPath::new(format!("{ROOT}/{path}"))
    }

    pub fn resolver_for_workspace(
        workspace: &FoundryWorkspace,
        remappings: &[Remapping],
    ) -> FoundryResolver {
        FoundryResolver::new(workspace, remappings).expect("resolver")
    }

    pub fn assert_resolves_like_foundry(
        workspace: &FoundryWorkspace,
        remappings: &[Remapping],
        importer: &NormalizedPath,
        import_path: &str,
    ) {
        let expected = FoundryResolver::new(workspace, remappings)
            .expect("resolver")
            .resolve_import_path(importer, import_path);
        let actual = resolve_import_path(workspace, remappings, importer, import_path);
        assert_eq!(actual, expected);
    }
}

#[test]
fn resolve_import_path_uses_remappings() {
    let remappings = vec![Remapping::new("dep/", "lib/dep/")];
    let workspace = helpers::workspace();
    let importer = helpers::workspace_path("src/Main.sol");

    let resolved = resolve_import_path(&workspace, &remappings, &importer, "dep/Thing.sol");
    let expected = helpers::resolver_for_workspace(&workspace, &remappings)
        .resolve_import_path(&importer, "dep/Thing.sol");
    assert_eq!(resolved, expected);
}

#[test]
fn resolve_import_path_with_resolver_prefers_adapter_when_available() {
    let remappings = vec![Remapping::new("dep/", "lib/dep/")];
    let workspace = helpers::workspace();
    let resolver = helpers::resolver_for_workspace(&workspace, &remappings);
    let importer = helpers::workspace_path("src/Main.sol");

    let resolved = resolve_import_path_with_resolver(
        &workspace,
        &remappings,
        &importer,
        "dep/Thing.sol",
        Some(&resolver),
    );

    assert_eq!(
        resolved,
        resolver.resolve_import_path(&importer, "dep/Thing.sol")
    );
}

#[test]
fn resolve_import_path_with_resolver_falls_back_without_resolver() {
    let remappings = vec![Remapping::new("dep/", "lib/dep/")];
    let workspace = helpers::workspace();
    let importer = helpers::workspace_path("src/Main.sol");

    let resolved = resolve_import_path_with_resolver(
        &workspace,
        &remappings,
        &importer,
        "dep/Thing.sol",
        None,
    );

    assert_eq!(
        resolved,
        resolve_import_path(&workspace, &remappings, &importer, "dep/Thing.sol")
    );
}

#[test]
fn remapping_skips_shorter_contexts_after_match() {
    let remappings = vec![
        Remapping::new("dep/", "lib/foo/dep/").with_context("lib/foo"),
        Remapping::new("dep/", "lib/default/dep/"),
    ];
    let workspace = helpers::workspace();
    let importer = helpers::workspace_path("lib/foo/src/Main.sol");

    helpers::assert_resolves_like_foundry(&workspace, &remappings, &importer, "dep/Thing.sol");
}

#[test]
fn remapping_ignores_non_matching_prefix_before_match() {
    let remappings = vec![
        Remapping::new("lib/", "lib/default/"),
        Remapping::new("dep/", "lib/dep/"),
    ];
    let workspace = helpers::workspace();
    let importer = helpers::workspace_path("src/Main.sol");

    helpers::assert_resolves_like_foundry(&workspace, &remappings, &importer, "dep/Thing.sol");
}
