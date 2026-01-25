use sa_def::DefKind;
use sa_hir::lowered_program;
use sa_paths::NormalizedPath;
use sa_project_model::Remapping;
use sa_span::TextRange;
use sa_test_support::{setup_db, slice_range};

#[test]
fn symbol_search_finds_workspace_symbols() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"contract Main {}"#,
        ),
        (
            NormalizedPath::new("/workspace/lib/Lib.sol"),
            r#"contract Lib {}"#,
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);

    let symbols = sa_ide_db::symbol_search(&db, project_id, "Lib");
    assert_eq!(symbols.len(), 1);

    let symbol = &symbols[0];
    assert_eq!(symbol.name(), "Lib");
    assert_eq!(
        symbol.file_id(),
        snapshot
            .file_id(&NormalizedPath::new("/workspace/lib/Lib.sol"))
            .expect("lib file id")
    );
}

#[test]
fn find_references_across_files() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"import "lib/Lib.sol";

contract Main {
    Lib lib;
}
"#,
        ),
        (
            NormalizedPath::new("/workspace/lib/Lib.sol"),
            r#"contract Lib {}"#,
        ),
    ];
    let remappings = vec![Remapping::new("lib/", "lib/")];
    let (db, project_id, _snapshot) = setup_db(files, remappings);

    let program = lowered_program(&db, project_id);
    let lib_def = program
        .def_map()
        .entries()
        .iter()
        .find(|entry| entry.kind() == DefKind::Contract && entry.location().name() == "Lib")
        .expect("Lib definition")
        .id();

    let refs = sa_ide_db::find_references(&db, project_id, lib_def);
    assert!(!refs.is_empty());
    assert!(
        refs.iter()
            .all(|r| r.range() != TextRange::empty(Default::default()))
    );
}

#[test]
fn symbol_search_reports_extended_kinds() {
    let text = r#"contract Foo {
    event Ping(address indexed from);
    error Oops(uint256 code);
    modifier onlyOwner() {
        _;
    }
    uint256 value;
    struct Bar {
        uint256 inner;
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
    let (db, project_id, snapshot) = setup_db(vec![(path.clone(), text)], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let cases = [
        ("Foo", DefKind::Contract),
        ("Ping", DefKind::Event),
        ("Oops", DefKind::Error),
        ("onlyOwner", DefKind::Modifier),
        ("value", DefKind::Variable),
        ("Bar", DefKind::Struct),
        ("State", DefKind::Enum),
        ("UserId", DefKind::Udvt),
        ("baz", DefKind::Function),
    ];

    for (name, kind) in cases {
        let symbols = sa_ide_db::symbol_search(&db, project_id, name);
        assert_eq!(symbols.len(), 1, "expected one symbol for {name}");
        let symbol = &symbols[0];
        assert_eq!(symbol.name(), name);
        assert_eq!(symbol.kind(), kind);
        assert_eq!(symbol.file_id(), file_id);
        assert_eq!(slice_range(text, symbol.range()), name);
    }
}
