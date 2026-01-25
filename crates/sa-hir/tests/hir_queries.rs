use sa_def::DefKind;
use sa_paths::NormalizedPath;
use sa_project_model::Remapping;
use sa_test_support::setup_db;

#[test]
fn lowers_multi_file_foundry_layout() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            "import \"lib/Lib.sol\"; contract Main {}",
        ),
        (
            NormalizedPath::new("/workspace/lib/forge-std/Lib.sol"),
            "contract Lib {}",
        ),
    ];
    let remappings = vec![Remapping::new("lib/", "lib/forge-std/")];
    let (db, project_id, snapshot) = setup_db(files, remappings);

    let program = sa_hir::lowered_program(&db, project_id);
    let file_ids = program.file_ids().collect::<Vec<_>>();
    assert_eq!(file_ids.len(), 2);

    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");
    let lib_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/lib/forge-std/Lib.sol"))
        .expect("lib file id");

    assert!(file_ids.contains(&main_id));
    assert!(file_ids.contains(&lib_id));
}

#[test]
fn resolves_imports_with_remappings() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            "import \"lib/Lib.sol\"; contract Main { Lib lib; }",
        ),
        (
            NormalizedPath::new("/workspace/lib/forge-std/Lib.sol"),
            "contract Lib {}",
        ),
    ];
    let remappings = vec![Remapping::new("lib/", "lib/forge-std/")];
    let (db, project_id, snapshot) = setup_db(files, remappings);

    let program = sa_hir::lowered_program(&db, project_id);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");

    let resolved = program.resolve_contract(main_id, "Lib");
    assert!(resolved.is_some(), "expected Lib to resolve from import");
}

#[test]
fn resolves_relative_imports() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            "import \"./utils/Utils.sol\"; contract Main { Utils u; }",
        ),
        (
            NormalizedPath::new("/workspace/src/utils/Utils.sol"),
            "contract Utils {}",
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);

    let program = sa_hir::lowered_program(&db, project_id);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");

    let resolved = program.resolve_contract(main_id, "Utils");
    assert!(
        resolved.is_some(),
        "expected Utils to resolve via ./ import"
    );
}

#[test]
fn resolve_symbol_finds_all_kinds() {
    let files = vec![(
        NormalizedPath::new("/workspace/src/Main.sol"),
        r#"
type Price is uint256;
struct Thing { uint256 thing_value; }
enum Kind { A }
event GlobalEvent();
error GlobalError();
function doThing() {}
contract Foo {
    event LocalEvent();
    error LocalError();
    modifier OnlyOwner() { _; }
    uint256 value;
}
"#,
    )];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let program = sa_hir::lowered_program(&db, project_id);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");

    let cases = [
        ("Foo", DefKind::Contract),
        ("Thing", DefKind::Struct),
        ("Kind", DefKind::Enum),
        ("doThing", DefKind::Function),
        ("GlobalEvent", DefKind::Event),
        ("GlobalError", DefKind::Error),
        ("LocalEvent", DefKind::Event),
        ("LocalError", DefKind::Error),
        ("OnlyOwner", DefKind::Modifier),
        ("value", DefKind::Variable),
        ("Price", DefKind::Udvt),
    ];

    for (name, expected_kind) in cases {
        let def_id = program
            .resolve_symbol(main_id, name)
            .unwrap_or_else(|| panic!("expected symbol {name} to resolve"));
        let entry = program
            .def_map()
            .entry(def_id)
            .unwrap_or_else(|| panic!("missing def entry for {name}"));
        assert_eq!(entry.kind(), expected_kind, "unexpected kind for {name}");
    }

    assert!(program.resolve_symbol(main_id, "NonExistent").is_none());
}

#[test]
fn resolve_symbol_prefers_local_over_imports_and_ignores_missing_imports() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"
import "lib/Lib.sol";
import "lib/Missing.sol";
contract Lib {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/lib/Lib.sol"),
            "contract Lib {}",
        ),
    ];
    let remappings = vec![Remapping::new("lib/", "lib/")];
    let (db, project_id, snapshot) = setup_db(files, remappings);
    let program = sa_hir::lowered_program(&db, project_id);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");

    let lib_def = program
        .resolve_symbol(main_id, "Lib")
        .expect("Lib should resolve");
    let lib_entry = program.def_map().entry(lib_def).expect("Lib entry");
    assert_eq!(lib_entry.location().file_id(), main_id);

    assert!(program.resolve_symbol(main_id, "Missing").is_none());
}
