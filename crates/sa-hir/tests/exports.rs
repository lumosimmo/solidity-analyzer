use sa_hir::lowered_program;
use sa_paths::NormalizedPath;
use sa_test_support::setup_db;

#[test]
fn resolves_reexported_symbol() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Base.sol"),
            r#"
contract Base {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Intermediate.sol"),
            r#"
import {Base} from "./Base.sol";

contract Intermediate is Base {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"
import {Intermediate, Base} from "./Intermediate.sol";

contract Main is Intermediate {
    Base value;
}
"#,
        ),
    ];

    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let program = lowered_program(&db, project_id);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");
    let base_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Base.sol"))
        .expect("base file id");

    let def_id = program.resolve_symbol(main_id, "Base").expect("definition");
    let entry = program.def_map().entry(def_id).expect("entry");
    assert_eq!(entry.location().file_id(), base_id);
}

#[test]
fn resolves_reexported_alias() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Base.sol"),
            r#"
contract Base {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Intermediate.sol"),
            r#"
import {Base as AliasBase} from "./Base.sol";

contract Intermediate is AliasBase {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            r#"
import {Intermediate, AliasBase} from "./Intermediate.sol";

contract Main is Intermediate {
    AliasBase value;
}
"#,
        ),
    ];

    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let program = lowered_program(&db, project_id);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");
    let base_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Base.sol"))
        .expect("base file id");

    let def_id = program
        .resolve_symbol(main_id, "AliasBase")
        .expect("definition");
    let entry = program.def_map().entry(def_id).expect("entry");
    assert_eq!(entry.location().file_id(), base_id);
}

#[test]
fn visible_definitions_handle_cycles() {
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/A.sol"),
            r#"
import {B} from "./B.sol";

contract A {}
"#,
        ),
        (
            NormalizedPath::new("/workspace/src/B.sol"),
            r#"
import {A} from "./A.sol";

contract B {}
"#,
        ),
    ];

    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let program = lowered_program(&db, project_id);
    let a_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/A.sol"))
        .expect("a file id");

    let defs = program.visible_definitions_in_file(a_id);
    let names = defs.iter().map(|def| def.name()).collect::<Vec<_>>();
    assert!(names.contains(&"A"));
    assert!(names.contains(&"B"));
}
