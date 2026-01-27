use foundry_compilers::resolver::SolImportAlias;
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryResolver, FoundryWorkspace, Remapping, resolve_import_path};

fn resolver_for_workspace(
    workspace: &FoundryWorkspace,
    remappings: &[Remapping],
) -> FoundryResolver {
    FoundryResolver::new(workspace, remappings).expect("resolver adapter")
}

fn assert_adapter_matches_foundry(
    workspace: &FoundryWorkspace,
    remappings: &[Remapping],
    importer: &NormalizedPath,
    import_path: &str,
) {
    let expected = resolve_import_path(workspace, remappings, importer, import_path);
    let resolver = resolver_for_workspace(workspace, remappings);
    let actual = resolver.resolve_import_path(importer, import_path);
    assert_eq!(actual, expected);
}

#[test]
fn resolver_adapter_matches_context_remapping() {
    let root = NormalizedPath::new("/workspace");
    let remappings = vec![
        Remapping::new("dep/", "lib/default/dep/"),
        Remapping::new("dep/", "lib/foo/dep/").with_context("lib/foo"),
    ];
    let workspace = FoundryWorkspace::new(root);

    let importer = NormalizedPath::new("/workspace/lib/foo/src/Main.sol");
    assert_adapter_matches_foundry(&workspace, &remappings, &importer, "dep/Thing.sol");
}

#[test]
fn resolver_adapter_matches_longest_prefix_rules() {
    let root = NormalizedPath::new("/workspace");
    let remappings = vec![
        Remapping::new("lib/", "lib/default/"),
        Remapping::new("lib/special/", "lib/override/"),
    ];
    let workspace = FoundryWorkspace::new(root);

    let importer = NormalizedPath::new("/workspace/src/Main.sol");
    assert_adapter_matches_foundry(&workspace, &remappings, &importer, "lib/special/Thing.sol");
}

#[test]
fn resolver_adapter_collects_import_aliases() {
    let root = NormalizedPath::new("/workspace");
    let remappings = Vec::new();
    let workspace = FoundryWorkspace::new(root);
    let resolver = resolver_for_workspace(&workspace, &remappings);
    let text = r#"
import {Foo as Bar, Baz} from "lib/Lib.sol";
import "./Other.sol";
"#;
    let file = NormalizedPath::new("/workspace/src/Main.sol");

    let imports = resolver
        .resolved_imports(&file, text)
        .expect("resolved imports");
    assert_eq!(imports.len(), 2);

    let aliases = &imports[0].aliases;
    assert_eq!(
        aliases,
        &[
            SolImportAlias::Contract("Bar".to_string(), "Foo".to_string()),
            SolImportAlias::File("Baz".to_string())
        ]
    );
}
