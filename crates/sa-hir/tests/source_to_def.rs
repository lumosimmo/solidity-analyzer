use sa_base_db::{Database, ProjectId};
use sa_def::DefKind;
use sa_hir::Semantics;
use sa_paths::NormalizedPath;
use sa_project_model::Remapping;
use sa_span::TextSize;
use sa_test_support::{extract_offset, extract_offsets, setup_db};

fn file_id(snapshot: &sa_vfs::VfsSnapshot, path: &str) -> sa_vfs::FileId {
    snapshot
        .file_id(&NormalizedPath::new(path))
        .unwrap_or_else(|| panic!("missing file id for {}", path))
}

fn resolve_and_verify_def(
    db: &Database,
    project_id: ProjectId,
    file_id: sa_vfs::FileId,
    offset: TextSize,
    expected_name: &str,
    expected_file_id: sa_vfs::FileId,
) {
    let semantics = Semantics::new(db, project_id);
    let def = semantics
        .source_to_def(file_id, offset)
        .expect("definition");
    let program = sa_hir::lowered_program(db, project_id);
    let location = program.def_map().entry(def).expect("def entry").location();

    assert_eq!(location.name(), expected_name);
    assert_eq!(location.file_id(), expected_file_id);
}

fn resolve_and_verify_def_with_container(
    db: &Database,
    project_id: ProjectId,
    file_id: sa_vfs::FileId,
    offset: TextSize,
    expected_name: &str,
    expected_file_id: sa_vfs::FileId,
    expected_container: Option<&str>,
) {
    let semantics = Semantics::new(db, project_id);
    let def = semantics
        .source_to_def(file_id, offset)
        .expect("definition");
    let program = sa_hir::lowered_program(db, project_id);
    let entry = program.def_map().entry(def).expect("def entry");
    let location = entry.location();

    assert_eq!(location.name(), expected_name);
    assert_eq!(location.file_id(), expected_file_id);
    assert_eq!(entry.container(), expected_container);
}

#[test]
fn resolves_source_to_def_in_same_file() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {}

contract Bar {
    Fo/*caret*/o f;
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    resolve_and_verify_def(&db, project_id, main_id, offset, "Foo", main_id);
}

#[test]
fn resolves_source_to_def_across_imports() {
    let (main_text, offset) = extract_offset(
        r#"
import "lib/Lib.sol";

contract Main {
    Li/*caret*/b lib;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/lib/forge-std/Lib.sol"),
            r#"
contract Lib {}
"#
            .to_string(),
        ),
    ];
    let remappings = vec![Remapping::new("lib/", "lib/forge-std/")];
    let (db, project_id, snapshot) = setup_db(files, remappings);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_id = file_id(&snapshot, "/workspace/lib/forge-std/Lib.sol");

    resolve_and_verify_def(&db, project_id, main_id, offset, "Lib", lib_id);
}

#[test]
fn resolves_source_to_def_via_import_alias() {
    let (main_text, offset) = extract_offset(
        r#"
import { Lib as Other } from "lib/Lib.sol";

contract Main {
    Ot/*caret*/her lib;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/lib/forge-std/Lib.sol"),
            r#"
contract Lib {}
"#
            .to_string(),
        ),
    ];
    let remappings = vec![Remapping::new("lib/", "lib/forge-std/")];
    let (db, project_id, snapshot) = setup_db(files, remappings);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_id = file_id(&snapshot, "/workspace/lib/forge-std/Lib.sol");

    resolve_and_verify_def(&db, project_id, main_id, offset, "Lib", lib_id);
}

#[test]
fn resolves_source_to_def_via_source_alias() {
    let (main_text, offset) = extract_offset(
        r#"
import "lib/Lib.sol" as Libs;

contract Main {
    Libs.Li/*caret*/b lib;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/lib/forge-std/Lib.sol"),
            r#"
contract Lib {}
"#
            .to_string(),
        ),
    ];
    let remappings = vec![Remapping::new("lib/", "lib/forge-std/")];
    let (db, project_id, snapshot) = setup_db(files, remappings);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_id = file_id(&snapshot, "/workspace/lib/forge-std/Lib.sol");

    resolve_and_verify_def(&db, project_id, main_id, offset, "Lib", lib_id);
}

#[test]
fn resolves_source_to_def_via_module_alias() {
    let (main_text, offset) = extract_offset(
        r#"
import * as Libs from "lib/Lib.sol";

contract Main {
    Libs.Li/*caret*/b lib;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/lib/forge-std/Lib.sol"),
            r#"
contract Lib {}
"#
            .to_string(),
        ),
    ];
    let remappings = vec![Remapping::new("lib/", "lib/forge-std/")];
    let (db, project_id, snapshot) = setup_db(files, remappings);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_id = file_id(&snapshot, "/workspace/lib/forge-std/Lib.sol");

    resolve_and_verify_def(&db, project_id, main_id, offset, "Lib", lib_id);
}

#[test]
fn resolves_contract_qualified_struct_in_same_file() {
    let (text, offset) = extract_offset(
        r#"
struct Thing {}

contract Foo {
    struct Thing {}
}

contract Bar {
    Foo.Th/*caret*/ing x;
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    resolve_and_verify_def_with_container(
        &db,
        project_id,
        main_id,
        offset,
        "Thing",
        main_id,
        Some("Foo"),
    );
}

#[test]
fn resolves_contract_qualified_struct_via_import_alias() {
    let (main_text, offset) = extract_offset(
        r#"
import { Foo as Bar } from "./Lib.sol";

contract Main {
    Bar.Th/*caret*/ing x;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            r#"
contract Foo {
    struct Thing {}
}
"#
            .to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_id = file_id(&snapshot, "/workspace/src/Lib.sol");

    resolve_and_verify_def_with_container(
        &db,
        project_id,
        main_id,
        offset,
        "Thing",
        lib_id,
        Some("Foo"),
    );
}

#[test]
fn resolves_inherited_state_variable_from_base_contract() {
    let (text, offset) = extract_offset(
        r#"
contract Other {
    uint256 value;
}

contract Base {
    uint256 value;
}

contract Derived is Base {
    function foo() public {
        val/*caret*/ue;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    resolve_and_verify_def_with_container(
        &db,
        project_id,
        main_id,
        offset,
        "value",
        main_id,
        Some("Base"),
    );
}

#[test]
fn resolves_inherited_function_via_c3_linearization() {
    let (text, offset) = extract_offset(
        r#"
contract A {
    function foo() public virtual {}
}

contract B is A {
    function foo() public virtual override {}
}

contract C is A {}

contract D is B, C {
    function bar() public {
        fo/*caret*/o();
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    resolve_and_verify_def_with_container(
        &db,
        project_id,
        main_id,
        offset,
        "foo",
        main_id,
        Some("B"),
    );
}

#[test]
fn resolves_super_call_to_next_override() {
    let (text, offset) = extract_offset(
        r#"
contract A {
    function foo() public virtual {}
}

contract B is A {
    function foo() public virtual override {}
}

contract C is A {
    function foo() public virtual override {}
}

contract D is B, C {
    function foo() public override(B, C) {
        super.fo/*caret*/o();
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    resolve_and_verify_def_with_container(
        &db,
        project_id,
        main_id,
        offset,
        "foo",
        main_id,
        Some("C"),
    );
}

#[test]
fn resolves_overloaded_function_by_argument_type() {
    let (text, offsets) = extract_offsets(
        r#"
contract Overloaded {
    function foo(address value) public {}
    function /*def*/foo(uint256 value) public {}

    function bar() public {
        fo/*caret*/o(1);
    }
}
"#,
        &["/*def*/", "/*caret*/"],
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    let def_offset = offsets[0];
    let call_offset = offsets[1];

    let semantics = Semantics::new(&db, project_id);
    let def = semantics
        .source_to_def(main_id, call_offset)
        .expect("definition");
    let program = sa_hir::lowered_program(&db, project_id);
    let entry = program.def_map().entry(def).expect("def entry");
    let expected_range = sa_span::TextRange::at(def_offset, TextSize::from(3));

    assert_eq!(entry.location().range(), expected_range);
}

#[test]
fn qualified_name_without_alias_does_not_fallback() {
    let (text, offset) = extract_offset(
        r#"
contract Lib {}

contract Main {
    Libs.Li/*caret*/b lib;
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    let semantics = Semantics::new(&db, project_id);
    assert!(semantics.source_to_def(main_id, offset).is_none());
}

#[test]
fn qualified_name_with_ambiguous_alias_does_not_fallback() {
    let (main_text, offset) = extract_offset(
        r#"
import "./LibA.sol" as Libs;
import "./LibB.sol" as Libs;
contract Main {
    Libs.Li/*caret*/b lib;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/LibA.sol"),
            r#"
contract Lib {}
"#
            .to_string(),
        ),
        (
            NormalizedPath::new("/workspace/src/LibB.sol"),
            r#"
contract Lib {}
"#
            .to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    let semantics = Semantics::new(&db, project_id);
    assert!(semantics.source_to_def(main_id, offset).is_none());
}

#[test]
fn resolves_source_to_def_via_source_alias_chain() {
    let (main_text, offset) = extract_offset(
        r#"
import "./Lib.sol" as Libs;

contract Main {
    Libs.Foo.Ba/*caret*/r value;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            r#"
contract Foo {
    struct Bar {}
}
"#
            .to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_id = file_id(&snapshot, "/workspace/src/Lib.sol");

    resolve_and_verify_def_with_container(
        &db,
        project_id,
        main_id,
        offset,
        "Bar",
        lib_id,
        Some("Foo"),
    );
}

#[test]
fn resolves_source_to_def_via_module_alias_chain() {
    let (main_text, offset) = extract_offset(
        r#"
import * as Libs from "./Lib.sol";

contract Main {
    Libs.Foo.Ba/*caret*/r value;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            r#"
contract Foo {
    struct Bar {}
}
"#
            .to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let lib_id = file_id(&snapshot, "/workspace/src/Lib.sol");

    resolve_and_verify_def_with_container(
        &db,
        project_id,
        main_id,
        offset,
        "Bar",
        lib_id,
        Some("Foo"),
    );
}

#[test]
fn source_to_def_falls_back_for_file_outside_workspace() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {}

contract Main {
    Fo/*caret*/o value;
}
"#,
    );
    let files = vec![(NormalizedPath::new("/outside/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/outside/Main.sol");

    resolve_and_verify_def(&db, project_id, main_id, offset, "Foo", main_id);
}

#[test]
fn source_to_def_ignores_missing_imports_when_hir_exists() {
    let (text, offset) = extract_offset(
        r#"
import "missing/Lib.sol";

contract Base {
    function ping() internal {}
}

contract Derived is Base {
    function ping() internal override {
        super.p/*caret*/ing();
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    resolve_and_verify_def_with_container(
        &db,
        project_id,
        main_id,
        offset,
        "ping",
        main_id,
        Some("Base"),
    );
}

#[test]
fn source_to_def_keeps_sema_with_unrelated_parse_errors() {
    let (main_text, offset) = extract_offset(
        r#"
contract Base {
    function ping() internal {}
}

contract Derived is Base {
    function ping() internal override {
        super.p/*caret*/ing();
    }
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Broken.sol"),
            r#"
contract Broken {
    function foo(
"#
            .to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");

    resolve_and_verify_def_with_container(
        &db,
        project_id,
        main_id,
        offset,
        "ping",
        main_id,
        Some("Base"),
    );
}

#[test]
fn source_to_def_resolves_additional_kinds() {
    let (text, offsets) = extract_offsets(
        r#"
type Price is uint256;
struct Thing {}
enum Kind { A }
event Logged(uint256 value);
error Failure();
modifier OnlyOwner() { _; }

contract Main {
    uint256 count;
    function foo() public OnlyOw/*mod*/ner {
        Thi/*struct*/ng memory thing;
        Ki/*enum*/nd kind = Kind.A;
        emit Logg/*event*/ed(1);
        revert Fail/*error*/ure();
        Pri/*udvt*/ce price;
        cou/*var*/nt;
    }
}
"#,
        &[
            "/*mod*/",
            "/*struct*/",
            "/*enum*/",
            "/*event*/",
            "/*error*/",
            "/*udvt*/",
            "/*var*/",
        ],
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/workspace/src/Main.sol");
    let semantics = Semantics::new(&db, project_id);
    let program = sa_hir::lowered_program(&db, project_id);

    let expected = [
        DefKind::Modifier,
        DefKind::Struct,
        DefKind::Enum,
        DefKind::Event,
        DefKind::Error,
        DefKind::Udvt,
        DefKind::Variable,
    ];

    for (offset, expected_kind) in offsets.into_iter().zip(expected) {
        let def = semantics
            .source_to_def(main_id, offset)
            .expect("definition");
        let entry = program.def_map().entry(def).expect("entry");
        assert_eq!(entry.kind(), expected_kind);
    }
}

#[test]
fn source_to_def_fallback_uses_symbol_when_qualifier_is_local() {
    let (text, offset) = extract_offset(
        r#"
contract Target {}

contract Main {
    function run() public {
        Target local = Target();
        local.Ta/*caret*/rget;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/outside/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/outside/Main.sol");

    let semantics = Semantics::new(&db, project_id);
    let def = semantics
        .source_to_def(main_id, offset)
        .expect("definition");
    let program = sa_hir::lowered_program(&db, project_id);
    let entry = program.def_map().entry(def).expect("entry");
    assert_eq!(entry.location().name(), "Target");
}

#[test]
fn source_to_def_fallback_rejects_multi_segment_qualifier() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function run() public {
        Libs.Foo.Bar.Ba/*caret*/z;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/outside/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let main_id = file_id(&snapshot, "/outside/Main.sol");

    let semantics = Semantics::new(&db, project_id);
    assert!(semantics.source_to_def(main_id, offset).is_none());
}
