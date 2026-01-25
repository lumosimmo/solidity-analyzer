use sa_ide::SymbolKind;
use sa_paths::NormalizedPath;
use sa_test_support::{setup_analysis, slice_range};

fn find_symbol<'a>(symbols: &'a [sa_ide::SymbolInfo], name: &str) -> &'a sa_ide::SymbolInfo {
    symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("missing symbol {name}"))
}

#[test]
fn document_symbols_match_outline_structure() {
    let text = r#"contract Foo {
    event Ping(address indexed from);
    error Oops(uint256 code);
    modifier onlyOwner() {
        _;
    }
    uint256 value;
    struct Bar {
        uint256 value;
    }
    enum State {
        On,
        Off
    }
    type UserId is uint256;

    function baz() external {}
}
"#;

    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");
    let symbols = analysis.document_symbols(file_id);

    assert_eq!(symbols.len(), 1);
    let contract = &symbols[0];
    assert_eq!(contract.kind, SymbolKind::Contract);
    assert_eq!(contract.name, "Foo");
    assert_eq!(contract.children.len(), 8);

    let struct_symbol = find_symbol(&contract.children, "Bar");
    assert_eq!(struct_symbol.kind, SymbolKind::Struct);
    assert_eq!(struct_symbol.name, "Bar");
    assert_eq!(slice_range(text, struct_symbol.selection_range), "Bar");

    let function_symbol = find_symbol(&contract.children, "baz");
    assert_eq!(function_symbol.kind, SymbolKind::Function);
    assert_eq!(function_symbol.name, "baz");
    assert_eq!(slice_range(text, function_symbol.selection_range), "baz");

    let event_symbol = find_symbol(&contract.children, "Ping");
    assert_eq!(event_symbol.kind, SymbolKind::Event);
    assert_eq!(slice_range(text, event_symbol.selection_range), "Ping");

    let error_symbol = find_symbol(&contract.children, "Oops");
    assert_eq!(error_symbol.kind, SymbolKind::Error);
    assert_eq!(slice_range(text, error_symbol.selection_range), "Oops");

    let modifier_symbol = find_symbol(&contract.children, "onlyOwner");
    assert_eq!(modifier_symbol.kind, SymbolKind::Modifier);
    assert_eq!(
        slice_range(text, modifier_symbol.selection_range),
        "onlyOwner"
    );

    let variable_symbol = find_symbol(&contract.children, "value");
    assert_eq!(variable_symbol.kind, SymbolKind::Variable);
    assert_eq!(slice_range(text, variable_symbol.selection_range), "value");

    let enum_symbol = find_symbol(&contract.children, "State");
    assert_eq!(enum_symbol.kind, SymbolKind::Enum);
    assert_eq!(slice_range(text, enum_symbol.selection_range), "State");

    let udvt_symbol = find_symbol(&contract.children, "UserId");
    assert_eq!(udvt_symbol.kind, SymbolKind::Udvt);
    assert_eq!(slice_range(text, udvt_symbol.selection_range), "UserId");
}
