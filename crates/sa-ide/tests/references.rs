use sa_base_db::FileId;
use sa_paths::NormalizedPath;
use sa_sema::sema_snapshot_for_project;
use sa_span::{TextRange, TextSize};
use sa_test_support::{extract_offset, extract_offsets, setup_analysis, setup_db};

fn assert_sema_reference_ranges(
    references: Option<&[sa_sema::SemaReference]>,
    file_id: FileId,
    mut expected: Vec<TextRange>,
) {
    let references = references.expect("references");
    let mut ranges = references
        .iter()
        .filter(|reference| reference.file_id() == file_id)
        .map(|reference| reference.range())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    expected.sort_by_key(|range| range.start());
    assert_eq!(ranges, expected);
}

fn assert_reference_ranges(references: Vec<sa_ide::Reference>, mut expected: Vec<TextRange>) {
    let mut ranges = references
        .iter()
        .map(|reference| reference.range())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    expected.sort_by_key(|range| range.start());
    assert_eq!(ranges, expected);
}

fn setup_single_file_analysis(text: String) -> (sa_ide::Analysis, FileId) {
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text)], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    (analysis, file_id)
}

#[test]
fn references_find_across_files() {
    let (main_text, offset) =
        extract_offset("import \"./Thing.sol\"; contract Main { /*caret*/Lib lib; }");
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Thing.sol"),
            "contract Lib {}".to_string(),
        ),
    ];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");
    let thing_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Thing.sol"))
        .expect("thing file id");

    let refs = analysis.find_references(main_id, offset);
    assert!(refs.iter().any(|reference| reference.file_id() == main_id));
    assert!(refs.iter().any(|reference| reference.file_id() == thing_id));
}

#[test]
fn references_respect_local_shadowing() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo(uint256 value) public {
        uint256 count = value;
        {
            uint256 count = 2;
            count;
        }
        co/*caret*/unt;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let refs = analysis.find_references(file_id, offset);
    let mut ranges = refs
        .iter()
        .map(|reference| reference.range())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    let positions = text
        .match_indices("count")
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    let expected = vec![
        TextRange::new(
            TextSize::from(positions[0] as u32),
            TextSize::from((positions[0] + "count".len()) as u32),
        ),
        TextRange::new(
            TextSize::from(positions[3] as u32),
            TextSize::from((positions[3] + "count".len()) as u32),
        ),
    ];

    assert_eq!(ranges, expected);
}

#[test]
fn references_handle_struct_constructor_member_access() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    struct Data {
        uint256 value;
    }

    uint256 count;

    function foo() public {
        Data memory data = Data(1);
        count = data.value;
        Data(2).value;
        co/*caret*/unt;
    }
}
"#,
    );
    let positions = text
        .match_indices("count")
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    let expected = positions
        .iter()
        .map(|start| {
            TextRange::new(
                TextSize::from(*start as u32),
                TextSize::from((*start + "count".len()) as u32),
            )
        })
        .collect::<Vec<_>>();

    let (analysis, file_id) = setup_single_file_analysis(text);
    let refs = analysis.find_references(file_id, offset);

    assert_reference_ranges(refs, expected);
}

#[test]
fn references_from_parameter_definition_include_header_usages() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    modifier guard(uint256 amount) {
        _;
    }

    function foo(uint256 val/*caret*/ue) public guard(value) {
        value;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let refs = analysis.find_references(file_id, offset);
    let mut ranges = refs
        .iter()
        .map(|reference| reference.range())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    let positions = text
        .match_indices("value")
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    let expected = positions
        .iter()
        .map(|start| {
            TextRange::new(
                TextSize::from(*start as u32),
                TextSize::from((*start + "value".len()) as u32),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(ranges, expected);
}

#[test]
fn references_ignore_comments_and_strings() {
    let (main_text, offset) = extract_offset(
        r#"
import "./Lib.sol";
contract Main {
    // Lib should not be treated as a reference.
    string constant NAME = "Lib";
    Lib lib;
    Li/*caret*/b other;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            "contract Lib {}".to_string(),
        ),
    ];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");

    let refs = analysis.find_references(main_id, offset);
    let main_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == main_id)
        .count();

    assert_eq!(refs.len(), 3);
    assert_eq!(main_refs, 2);
}

#[test]
fn references_include_import_aliases() {
    let (main_text, offset) = extract_offset(
        r#"
import { Lib as Other } from "./Lib.sol";
contract Main {
    Ot/*caret*/her other;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            "contract Lib {}".to_string(),
        ),
    ];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");
    let lib_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Lib.sol"))
        .expect("lib file id");

    let refs = analysis.find_references(main_id, offset);
    let main_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == main_id)
        .count();
    let lib_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == lib_id)
        .count();

    assert_eq!(main_refs, 2);
    assert_eq!(lib_refs, 1);
}

#[test]
fn references_skip_ambiguous_imports() {
    let (main_text, offset) = extract_offset(
        r#"
import "./LibA.sol";
import "./LibB.sol";
contract Main {
    Li/*caret*/b lib;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/LibA.sol"),
            "contract Lib {}".to_string(),
        ),
        (
            NormalizedPath::new("/workspace/src/LibB.sol"),
            "contract Lib {}".to_string(),
        ),
    ];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");
    let liba_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/LibA.sol"))
        .expect("liba file id");

    let refs = analysis.find_references(main_id, offset);
    let main_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == main_id)
        .count();
    let liba_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == liba_id)
        .count();

    assert_eq!(main_refs, 0);
    assert_eq!(liba_refs, 0);
}

#[test]
fn references_include_source_alias_imports() {
    let (main_text, offset) = extract_offset(
        r#"
import "./Lib.sol" as Libs;
contract Main {
    Libs.Lib lib;
    Libs.Li/*caret*/b other;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            "contract Lib {}".to_string(),
        ),
    ];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");
    let lib_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Lib.sol"))
        .expect("lib file id");

    let refs = analysis.find_references(main_id, offset);
    let main_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == main_id)
        .count();
    let lib_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == lib_id)
        .count();

    assert_eq!(main_refs, 2);
    assert_eq!(lib_refs, 1);
}

#[test]
fn references_include_module_alias_imports() {
    let (main_text, offset) = extract_offset(
        r#"
import * as Libs from "./Lib.sol";
contract Main {
    Libs.Lib lib;
    Libs.Li/*caret*/b other;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            "contract Lib {}".to_string(),
        ),
    ];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");
    let lib_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Lib.sol"))
        .expect("lib file id");

    let refs = analysis.find_references(main_id, offset);
    let main_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == main_id)
        .count();
    let lib_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == lib_id)
        .count();

    assert_eq!(main_refs, 2);
    assert_eq!(lib_refs, 1);
}

#[test]
fn references_include_contract_alias_qualified_refs() {
    let (main_text, offset) = extract_offset(
        r#"
import { Foo as Bar } from "./Lib.sol";
contract Main {
    Bar.Th/*caret*/ing value;
}
"#,
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Lib.sol"),
            "contract Foo { struct Thing {} }".to_string(),
        ),
    ];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let main_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("main file id");
    let lib_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Lib.sol"))
        .expect("lib file id");

    let refs = analysis.find_references(main_id, offset);
    let main_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == main_id)
        .count();
    let lib_refs = refs
        .iter()
        .filter(|reference| reference.file_id() == lib_id)
        .count();

    assert_eq!(main_refs, 1);
    assert_eq!(lib_refs, 1);
}

#[test]
fn references_resolve_inherited_state_variable() {
    let (text, offsets) = extract_offsets(
        r#"
contract Other {
    uint256 value;
}

contract Base {
    uint256 /*def*/value;
}

contract Derived is Base {
    function foo() public {
        /*caret*/value;
    }
}
"#,
        &["/*def*/", "/*caret*/"],
    );
    let def_offset = offsets[0];
    let caret_offset = offsets[1];
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let refs = analysis.find_references(file_id, caret_offset);
    let mut ranges = refs
        .iter()
        .map(|reference| reference.range())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    let len = TextSize::from("value".len() as u32);
    let expected = vec![
        TextRange::at(def_offset, len),
        TextRange::at(caret_offset, len),
    ];

    assert_eq!(ranges, expected);
}

#[test]
fn references_skip_ambiguous_member_without_args() {
    let (text, offsets) = extract_offsets(
        r#"
contract Foo {
    function /*def1*/bar(uint256) external {}
    function /*def2*/bar(address) external {}
}

contract Main {
    Foo foo;

    function test() external {
        foo./*call1*/bar(1);
        foo./*call2*/bar(0x0000000000000000000000000000000000000000);
        function(uint256) external f = foo.bar;
    }
}
"#,
        &["/*def1*/", "/*def2*/", "/*call1*/", "/*call2*/"],
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");
    let project = db.project_input(project_id);
    let sema_snapshot = sema_snapshot_for_project(&db, project);
    let sema_snapshot = sema_snapshot.for_file(file_id).expect("sema snapshot");

    let name_len = TextSize::from("bar".len() as u32);
    let def1_range = TextRange::at(offsets[0], name_len);
    let def2_range = TextRange::at(offsets[1], name_len);
    let call1_range = TextRange::at(offsets[2], name_len);
    let call2_range = TextRange::at(offsets[3], name_len);

    assert_sema_reference_ranges(
        sema_snapshot.references_for_definition(file_id, def1_range),
        file_id,
        vec![def1_range, call1_range],
    );
    assert_sema_reference_ranges(
        sema_snapshot.references_for_definition(file_id, def2_range),
        file_id,
        vec![def2_range, call2_range],
    );
}

#[test]
fn references_resolve_overridden_function_in_multiple_inheritance() {
    let (text, offsets) = extract_offsets(
        r#"
contract A {
    function foo() public virtual {}
}

contract B is A {
    function /*def*/foo() public virtual override {}
}

contract C is A {
    function foo() public virtual override {}
}

contract D is B, C {
    function bar() public {
        /*caret*/foo();
    }
}
"#,
        &["/*def*/", "/*caret*/"],
    );
    let def_offset = offsets[0];
    let caret_offset = offsets[1];
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let refs = analysis.find_references(file_id, caret_offset);
    let mut ranges = refs
        .iter()
        .map(|reference| reference.range())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    let len = TextSize::from("foo".len() as u32);
    let expected = vec![
        TextRange::at(def_offset, len),
        TextRange::at(caret_offset, len),
    ];

    assert_eq!(ranges, expected);
}

#[test]
fn references_resolve_overloaded_function_calls() {
    let (text, offsets) = extract_offsets(
        r#"
contract Overloaded {
    function foo(address value) public {}
    function /*def*/foo(uint256 value) public {}

    function bar() public {
        /*caret*/foo(1);
    }
}
"#,
        &["/*def*/", "/*caret*/"],
    );
    let def_offset = offsets[0];
    let caret_offset = offsets[1];
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (analysis, snapshot) = setup_analysis(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let refs = analysis.find_references(file_id, caret_offset);
    let mut ranges = refs
        .iter()
        .map(|reference| reference.range())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    let len = TextSize::from("foo".len() as u32);
    let expected = vec![
        TextRange::at(def_offset, len),
        TextRange::at(caret_offset, len),
    ];

    assert_eq!(ranges, expected);
}

#[test]
fn references_skip_inherited_contract_type_member() {
    let (text, offsets) = extract_offsets(
        r#"
contract A {
    function /*def*/foo() public {}
}

contract B is A {}

contract Main {
    function bar() public {
        A./*use*/foo.selector;
        B./*caret*/foo.selector;
    }
}
"#,
        &["/*def*/", "/*use*/", "/*caret*/"],
    );
    let def_offset = offsets[0];
    let use_offset = offsets[1];
    let caret_offset = offsets[2];
    let (analysis, file_id) = setup_single_file_analysis(text.clone());

    let name_len = TextSize::from("foo".len() as u32);
    let expected = vec![
        TextRange::at(def_offset, name_len),
        TextRange::at(use_offset, name_len),
    ];
    assert_reference_ranges(analysis.find_references(file_id, def_offset), expected);
    assert!(analysis.find_references(file_id, caret_offset).is_empty());
}
