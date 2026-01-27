use sa_paths::NormalizedPath;
use sa_project_model::{FoundryResolver, FoundryWorkspace, Remapping, resolve_import_path};

fn workspace() -> FoundryWorkspace {
    let root = NormalizedPath::new("/workspace");
    FoundryWorkspace::new(root)
}

fn assert_resolves_like_foundry(
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

#[test]
fn context_specific_remappings_resolve_by_importer_path() {
    let remappings = vec![
        Remapping::new("dep/", "lib/default/dep/"),
        Remapping::new("dep/", "lib/foo/dep/").with_context("lib/foo"),
        Remapping::new("dep/", "lib/bar/dep/").with_context("lib/bar"),
    ];
    let workspace = workspace();

    let foo_path = NormalizedPath::new("/workspace/lib/foo/src/Main.sol");
    let bar_path = NormalizedPath::new("/workspace/lib/bar/src/Main.sol");
    let default_path = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &remappings, &foo_path, "dep/Thing.sol");
    assert_resolves_like_foundry(&workspace, &remappings, &bar_path, "dep/Thing.sol");
    assert_resolves_like_foundry(&workspace, &remappings, &default_path, "dep/Thing.sol");
}

#[test]
fn context_specific_remapping_beats_longer_prefix() {
    let remappings = vec![
        Remapping::new("dep/long/", "lib/default/dep/long/"),
        Remapping::new("dep/", "lib/foo/dep/").with_context("lib/foo"),
    ];
    let workspace = workspace();
    let importer = NormalizedPath::new("/workspace/lib/foo/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &remappings, &importer, "dep/long/Thing.sol");
}

#[test]
fn longest_prefix_is_order_independent() {
    let remappings_a = vec![
        Remapping::new("lib/", "lib/default/"),
        Remapping::new("lib/special/", "lib/override/"),
    ];
    let remappings_b = vec![
        Remapping::new("lib/special/", "lib/override/"),
        Remapping::new("lib/", "lib/default/"),
    ];

    let workspace_a = workspace();
    let workspace_b = workspace();

    let importer = NormalizedPath::new("/workspace/src/Main.sol");
    assert_resolves_like_foundry(
        &workspace_a,
        &remappings_a,
        &importer,
        "lib/special/Thing.sol",
    );
    assert_resolves_like_foundry(
        &workspace_b,
        &remappings_b,
        &importer,
        "lib/special/Thing.sol",
    );
}

#[test]
fn remapping_with_contracts_segment_resolves() {
    let remappings = vec![Remapping::new(
        "@oz/",
        "lib/openzeppelin-contracts/contracts/",
    )];
    let workspace = workspace();
    let importer = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(
        &workspace,
        &remappings,
        &importer,
        "@oz/token/ERC20/ERC20.sol",
    );
}

#[test]
fn remapped_imports_normalize_backslashes() {
    let remappings = vec![Remapping::new("lib/", "lib/forge-std/src/")];
    let workspace = workspace();
    let importer = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &remappings, &importer, r"lib\Test.sol");
}

#[test]
fn absolute_import_paths_are_not_rewritten() {
    let remappings = Vec::new();
    let workspace = workspace();
    let importer = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &remappings, &importer, "/opt/Lib.sol");
}

#[test]
fn context_remapping_ignores_paths_outside_workspace() {
    let remappings = vec![
        Remapping::new("dep/", "lib/default/dep/"),
        Remapping::new("dep/", "lib/alt/dep/").with_context("lib/alt"),
    ];
    let workspace = workspace();
    let importer = NormalizedPath::new("/external/src/Main.sol");

    assert_resolves_like_foundry(&workspace, &remappings, &importer, "dep/Thing.sol");
}

#[test]
fn absolute_import_paths_preserve_windows_unc_roots() {
    let remappings = Vec::new();
    let workspace = workspace();
    let importer = NormalizedPath::new("/workspace/src/Main.sol");

    assert_resolves_like_foundry(
        &workspace,
        &remappings,
        &importer,
        r"\\server\share\Lib.sol",
    );
    assert_resolves_like_foundry(&workspace, &remappings, &importer, r"C:\Lib\Foo.sol");
}
