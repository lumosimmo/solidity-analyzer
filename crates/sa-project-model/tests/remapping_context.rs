use sa_paths::NormalizedPath;
use sa_project_model::{
    FoundryProfile, FoundryResolver, FoundryWorkspace, Remapping, resolve_import_path_with_profile,
};

fn workspace_with_remappings(remappings: Vec<Remapping>) -> FoundryWorkspace {
    let root = NormalizedPath::new("/workspace");
    let profile = FoundryProfile::new("default").with_remappings(remappings);
    FoundryWorkspace::new(root, profile)
}

fn assert_resolves_like_foundry(
    workspace: &FoundryWorkspace,
    importer: &NormalizedPath,
    import_path: &str,
) {
    let expected = FoundryResolver::new(workspace, None)
        .expect("resolver")
        .resolve_import_path(importer, import_path);
    let actual = resolve_import_path_with_profile(workspace, importer, import_path, None);
    assert_eq!(actual, expected);
}

#[test]
fn context_specific_remappings_resolve_by_importer_path() {
    let workspace = workspace_with_remappings(vec![
        Remapping::new("dep/", "lib/default/dep/"),
        Remapping::new("dep/", "lib/foo/dep/").with_context("lib/foo"),
        Remapping::new("dep/", "lib/bar/dep/").with_context("lib/bar"),
    ]);

    let foo_path = NormalizedPath::new("/workspace/lib/foo/src/Main.sol");
    let bar_path = NormalizedPath::new("/workspace/lib/bar/src/Main.sol");
    let default_path = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &foo_path, "dep/Thing.sol");
    assert_resolves_like_foundry(&workspace, &bar_path, "dep/Thing.sol");
    assert_resolves_like_foundry(&workspace, &default_path, "dep/Thing.sol");
}

#[test]
fn context_specific_remapping_beats_longer_prefix() {
    let workspace = workspace_with_remappings(vec![
        Remapping::new("dep/long/", "lib/default/dep/long/"),
        Remapping::new("dep/", "lib/foo/dep/").with_context("lib/foo"),
    ]);
    let importer = NormalizedPath::new("/workspace/lib/foo/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &importer, "dep/long/Thing.sol");
}

#[test]
fn longest_prefix_is_order_independent() {
    let root = NormalizedPath::new("/workspace");
    let remappings_a = vec![
        Remapping::new("lib/", "lib/default/"),
        Remapping::new("lib/special/", "lib/override/"),
    ];
    let remappings_b = vec![
        Remapping::new("lib/special/", "lib/override/"),
        Remapping::new("lib/", "lib/default/"),
    ];

    let workspace_a = FoundryWorkspace::new(
        root.clone(),
        FoundryProfile::new("default").with_remappings(remappings_a),
    );
    let workspace_b = FoundryWorkspace::new(
        root,
        FoundryProfile::new("default").with_remappings(remappings_b),
    );

    let importer = NormalizedPath::new("/workspace/src/Main.sol");
    assert_resolves_like_foundry(&workspace_a, &importer, "lib/special/Thing.sol");
    assert_resolves_like_foundry(&workspace_b, &importer, "lib/special/Thing.sol");
}

#[test]
fn remapping_with_contracts_segment_resolves() {
    let root = NormalizedPath::new("/workspace");
    let profile = FoundryProfile::new("default").with_remappings(vec![Remapping::new(
        "@oz/",
        "lib/openzeppelin-contracts/contracts/",
    )]);
    let workspace = FoundryWorkspace::new(root, profile);
    let importer = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &importer, "@oz/token/ERC20/ERC20.sol");
}

#[test]
fn remapped_imports_normalize_backslashes() {
    let workspace = workspace_with_remappings(vec![Remapping::new("lib/", "lib/forge-std/src/")]);
    let importer = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &importer, r"lib\Test.sol");
}

#[test]
fn absolute_import_paths_are_not_rewritten() {
    let workspace = workspace_with_remappings(Vec::new());
    let importer = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &importer, "/opt/Lib.sol");
}

#[test]
fn context_remapping_ignores_paths_outside_workspace() {
    let workspace = workspace_with_remappings(vec![
        Remapping::new("dep/", "lib/default/dep/"),
        Remapping::new("dep/", "lib/alt/dep/").with_context("lib/alt"),
    ]);
    let importer = NormalizedPath::new("/external/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &importer, "dep/Thing.sol");
}

#[test]
fn absolute_import_paths_preserve_windows_unc_roots() {
    let workspace = workspace_with_remappings(Vec::new());
    let importer = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &importer, r"\\server\share\Lib.sol");
    assert_resolves_like_foundry(&workspace, &importer, r"C:\Lib\Foo.sol");
}
