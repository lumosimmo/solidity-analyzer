use std::sync::Arc;

use sa_config::ResolvedFoundryConfig;
use sa_ide::{AnalysisChange, AnalysisHost, SymbolKind};
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryWorkspace};
use sa_span::{TextRange, lsp};
use sa_vfs::{Vfs, VfsChange};

fn slice_range(text: &str, range: TextRange) -> &str {
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    &text[start..end]
}

#[test]
fn syntax_outline_builds_nested_symbols_with_ranges() {
    let text = r#"contract Foo {
    struct Bar {
        uint256 value;
    }

    function baz() external {}
}

function topLevel() {}
"#;

    let mut vfs = Vfs::default();
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    vfs.apply_change(VfsChange::Set {
        path: path.clone(),
        text: Arc::from(text),
    });
    let snapshot = vfs.snapshot();
    let file_id = snapshot.file_id(&path).expect("file id");

    let root = NormalizedPath::new("/workspace");
    let default_profile = FoundryProfile::new("default");
    let workspace = FoundryWorkspace::new(root);
    let config = ResolvedFoundryConfig::new(workspace.clone(), default_profile);

    let mut host = AnalysisHost::new();
    let mut change = AnalysisChange::new();
    change.set_vfs(snapshot);
    change.set_config(config);
    host.apply_change(change);

    let analysis = host.snapshot();
    let symbols = analysis.syntax_outline(file_id);

    assert_eq!(symbols.len(), 2);
    let contract = &symbols[0];
    assert_eq!(contract.kind, SymbolKind::Contract);
    assert_eq!(contract.name, "Foo");
    assert_eq!(contract.children.len(), 2);

    let struct_symbol = &contract.children[0];
    assert_eq!(struct_symbol.kind, SymbolKind::Struct);
    assert_eq!(struct_symbol.name, "Bar");
    assert_eq!(slice_range(text, struct_symbol.selection_range), "Bar");

    let function_symbol = &contract.children[1];
    assert_eq!(function_symbol.kind, SymbolKind::Function);
    assert_eq!(function_symbol.name, "baz");
    assert_eq!(slice_range(text, function_symbol.selection_range), "baz");

    assert!(contract.range.start() <= struct_symbol.range.start());
    assert!(contract.range.end() >= struct_symbol.range.end());
    assert!(contract.range.start() <= function_symbol.range.start());
    assert!(contract.range.end() >= function_symbol.range.end());

    let top_level = &symbols[1];
    assert_eq!(top_level.kind, SymbolKind::Function);
    assert_eq!(top_level.name, "topLevel");
    assert_eq!(slice_range(text, top_level.selection_range), "topLevel");

    let lsp_range = lsp::to_lsp_range(contract.selection_range, text);
    assert_eq!(lsp_range.start.line, 0);
    assert_eq!(lsp_range.start.character, 9);
    assert_eq!(lsp_range.end.line, 0);
    assert_eq!(lsp_range.end.character, 12);
}
