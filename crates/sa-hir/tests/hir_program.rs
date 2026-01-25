use std::sync::Arc;

use sa_base_db::LanguageKind;
use sa_def::DefKind;
use sa_hir::{Semantics, contract_member_definitions_at_offset, lowered_program, parse};
use sa_paths::NormalizedPath;
use sa_test_support::{extract_offset, setup_db};

fn file_id(snapshot: &sa_vfs::VfsSnapshot, path: &str) -> sa_vfs::FileId {
    snapshot
        .file_id(&NormalizedPath::new(path))
        .unwrap_or_else(|| panic!("missing file id for {}", path))
}

#[test]
fn parse_and_program_update_when_files_change() {
    let files = vec![(
        NormalizedPath::new("/workspace/src/Main.sol"),
        "import \"./A.sol\"; contract Main {}",
    )];
    let (mut db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    let parsed = parse(&db, main_id);
    let program = lowered_program(&db, project_id);

    let path = db.file_path(main_id);
    let text = db.file_input(main_id).text(&db).clone();
    db.set_file(
        main_id,
        text.clone(),
        1,
        LanguageKind::Solidity,
        path.clone(),
    );
    let parsed_same = parse(&db, main_id);
    assert_eq!(parsed, parsed_same);

    let updated: Arc<str> =
        Arc::from("import \"./A.sol\"; import \"./B.sol\"; contract Main {} contract Extra {}");
    db.set_file(main_id, updated, 2, LanguageKind::Solidity, path);
    let parsed_updated = parse(&db, main_id);
    assert_ne!(parsed, parsed_updated);

    let program_updated = lowered_program(&db, project_id);
    assert_ne!(program, program_updated);
}

#[test]
fn visible_definitions_include_aliases_and_source_aliases() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"
import { Foo as AliasFoo, value as AliasValue } from "./Lib.sol";
import "./Lib.sol" as LibAlias;
import * as LibGlob from "./Lib.sol";
import "./Missing.sol";

contract Main {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            r#"
contract Foo {
    uint256 value;
}
"#,
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let program = lowered_program(&db, project_id);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    let defs = program.visible_definitions_in_file(main_id);
    let names = defs
        .iter()
        .map(|def| (def.name().to_string(), def.kind()))
        .collect::<Vec<_>>();

    assert!(names.contains(&("AliasFoo".to_string(), DefKind::Contract)));
    assert!(names.contains(&("LibAlias".to_string(), DefKind::Udvt)));
    assert!(names.contains(&("LibGlob".to_string(), DefKind::Udvt)));
    assert!(!names.iter().any(|(name, _)| name == "AliasValue"));
}

#[test]
fn local_names_and_qualifiers_for_imported_symbols() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"
import "./Lib.sol";
import { Foo as AliasFoo, Missing as AliasMissing } from "./Lib.sol";
import "./Lib.sol" as LibAlias;
import * as LibGlob from "./Lib.sol";
import "./Other.sol" as Shared;
import "./Lib.sol" as Shared;

contract Main {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            r#"
contract Foo {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Other.sol"),
            r#"
contract Other {}
"#,
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let program = lowered_program(&db, project_id);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_id = file_id(&snapshot, "/workspace/src/Lib.sol");

    let names = program.local_names_for_imported(main_id, lib_id, "Foo");
    assert_eq!(names, vec!["AliasFoo".to_string(), "Foo".to_string()]);

    let qualifiers = program.qualifier_names_for_imported(main_id, lib_id);
    assert_eq!(
        qualifiers,
        vec!["LibAlias".to_string(), "LibGlob".to_string()]
    );
}

#[test]
fn resolve_symbol_kind_candidates_collect_imports_and_aliases() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"
import "./LibA.sol";
import { Foo as AliasFoo } from "./LibB.sol";

contract Foo {}
contract Main { Foo local; AliasFoo alias; }
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/LibA.sol"),
            r#"
contract Foo {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/LibB.sol"),
            r#"
contract Foo {}
"#,
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let program = lowered_program(&db, project_id);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_a_id = file_id(&snapshot, "/workspace/src/LibA.sol");
    let lib_b_id = file_id(&snapshot, "/workspace/src/LibB.sol");

    let candidates = program.resolve_symbol_kind_candidates(main_id, DefKind::Contract, "Foo");
    assert_eq!(candidates.len(), 2);
    let mut candidate_files = candidates
        .iter()
        .map(|def_id| {
            program
                .def_map()
                .entry(*def_id)
                .expect("entry")
                .location()
                .file_id()
        })
        .collect::<Vec<_>>();
    candidate_files.sort();
    let mut expected = vec![lib_a_id, main_id];
    expected.sort();
    assert_eq!(candidate_files, expected);

    let alias_candidates =
        program.resolve_symbol_kind_candidates(main_id, DefKind::Contract, "AliasFoo");
    assert_eq!(alias_candidates.len(), 1);
    let alias_entry = program
        .def_map()
        .entry(alias_candidates[0])
        .expect("alias entry");
    assert_eq!(alias_entry.location().file_id(), lib_b_id);
}

#[test]
fn resolve_qualified_symbol_handles_ambiguous_aliases() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"
import "./Lib.sol" as Libs;
import "./Other.sol" as Libs;

contract Main { Libs.Foo value; }
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            r#"
contract Foo {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Other.sol"),
            r#"
contract Foo {}
"#,
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let program = lowered_program(&db, project_id);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    assert!(
        program
            .resolve_qualified_symbol(main_id, "Libs", "Foo")
            .is_none()
    );
}

#[test]
fn resolve_qualified_symbol_and_contract_qualified_symbol() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"
import "./Lib.sol" as Libs;

contract Foo {
    struct Bar {}
}

contract Main { Foo.Bar value; Libs.Baz other; }
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            r#"
contract Baz {}
"#,
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let program = lowered_program(&db, project_id);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_id = file_id(&snapshot, "/workspace/src/Lib.sol");

    let baz = program
        .resolve_qualified_symbol(main_id, "Libs", "Baz")
        .expect("qualified Baz");
    let baz_entry = program.def_map().entry(baz).expect("baz entry");
    assert_eq!(baz_entry.location().file_id(), lib_id);

    let bar = program
        .resolve_contract_qualified_symbol(main_id, "Foo", "Bar")
        .expect("qualified Bar");
    let bar_entry = program.def_map().entry(bar).expect("bar entry");
    assert_eq!(bar_entry.location().file_id(), main_id);
    assert_eq!(bar_entry.container(), Some("Foo"));
}

#[test]
fn contract_member_definitions_include_inherited_members() {
    let (text, offset) = extract_offset(
        r#"
import "./Base.sol" as Libs;

contract Parent {
    function parentFn() public {}
}

contract Child is Parent, Libs.Base {
    /*caret*/function childFn() public {}
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), text),
        (
            NormalizedPath::new("/workspace/src/Base.sol"),
            r#"
contract Base {
    function baseFn() public {}
}
"#
            .to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    let defs = contract_member_definitions_at_offset(&db, project_id, main_id, offset);
    let names = defs.iter().map(|def| def.name()).collect::<Vec<_>>();
    assert!(names.contains(&"childFn"));
    assert!(names.contains(&"parentFn"));
    assert!(names.contains(&"baseFn"));
}

#[test]
fn contract_member_definitions_empty_outside_contract() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function foo() public {}
}

/*caret*/contract Bar {}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    let defs = contract_member_definitions_at_offset(&db, project_id, main_id, offset);
    assert!(defs.is_empty());
}

#[test]
fn resolve_definition_prefers_locals() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {}

contract Main {
    function run(uint256 value) public {
        val/*caret*/ue;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    let semantics = Semantics::new(&db, project_id);
    let definition = semantics
        .resolve_definition(main_id, offset)
        .expect("definition");
    match definition {
        sa_hir::Definition::Local(local) => {
            assert_eq!(local.name(), "value");
        }
        sa_hir::Definition::Global(_) => panic!("expected local definition"),
    }
}
