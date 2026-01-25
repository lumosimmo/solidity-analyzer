use sa_ide_completion::CompletionItemKind;
use sa_paths::NormalizedPath;
use sa_test_support::{extract_offset, setup_db};

fn completion_labels(items: &[sa_ide_completion::CompletionItem]) -> Vec<&str> {
    items.iter().map(|item| item.label.as_str()).collect()
}

fn completions_for_main(text_with_caret: &str) -> Vec<sa_ide_completion::CompletionItem> {
    let (text, offset) = extract_offset(text_with_caret.trim());
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    sa_ide_completion::completions(&db, project_id, file_id, offset)
}

fn completions_for_main_with_deps(
    text_with_caret: &str,
    deps: Vec<(NormalizedPath, String)>,
) -> Vec<sa_ide_completion::CompletionItem> {
    let (text, offset) = extract_offset(text_with_caret.trim());
    let mut files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    files.extend(deps);
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    sa_ide_completion::completions(&db, project_id, file_id, offset)
}

#[test]
fn completes_identifiers_sorted_and_filtered() {
    let (text, offset) = extract_offset(
        r#"
contract Alpha {}
contract Beta {}
contract Main { Al/*caret*/pha value; }
"#
        .trim(),
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"Alpha"));
    let mut sorted = labels.clone();
    sorted.sort();
    assert_eq!(labels, sorted);

    // Verify kind for Alpha is Contract
    let alpha_item = completions
        .iter()
        .find(|item| item.label == "Alpha")
        .expect("Alpha completion item");
    assert_eq!(alpha_item.kind, CompletionItemKind::Contract);
}

#[test]
fn scoped_identifier_completion_excludes_unrelated_contract_members() {
    let (text, offset) = extract_offset(
        r#"
import "./Dep.sol";

contract Alpha {
    uint256 numberA;

    function test(uint256 paramA) public returns (uint256 namedReturn) {
        uint256 localA = 1;
        /*caret*/
    }
}
"#
        .trim(),
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), text),
        (
            NormalizedPath::new("/workspace/src/Dep.sol"),
            r#"
contract Beta {
    uint256 numberB;
}
"#
            .trim()
            .to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let labels = completions
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"numberA"));
    assert!(labels.contains(&"paramA"));
    assert!(labels.contains(&"namedReturn"));
    assert!(labels.contains(&"localA"));
    assert!(!labels.contains(&"numberB"));
}

#[test]
fn scoped_identifier_completion_skips_locals_after_caret() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function test() public {
        uint256 beforeCaret = 1;
        /*caret*/
        uint256 afterCaret = 2;
    }
}
"#
        .trim(),
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let labels = completions
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"beforeCaret"));
    assert!(!labels.contains(&"afterCaret"));
}

#[test]
fn scoped_identifier_completion_scopes_when_sema_unavailable() {
    let (text, offset) = extract_offset(
        r#"
import "./Dep.sol";

contract Alpha {
    uint256 numberA;

    function test() public {
        /*caret*/
    }
}
"#
        .trim(),
    );
    let files = vec![
        (NormalizedPath::new("/external/Main.sol"), text),
        (
            NormalizedPath::new("/external/Dep.sol"),
            r#"
contract Beta {
    uint256 numberB;
}
"#
            .trim()
            .to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/external/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"numberA"));
    assert!(!labels.contains(&"numberB"));
}

#[test]
fn completes_inherited_members_when_sema_unavailable() {
    let (text, offset) = extract_offset(
        r#"
import "./Base.sol";

contract Derived is Base {
    function test() public {
        ba/*caret*/;
    }
}
"#
        .trim(),
    );
    let files = vec![
        (NormalizedPath::new("/external/Main.sol"), text),
        (
            NormalizedPath::new("/external/Base.sol"),
            r#"
contract Base {
    uint256 public baseValue;
    function basePing() public {}
}
"#
            .trim()
            .to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/external/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"baseValue"));
    assert!(labels.contains(&"basePing"));
}

#[test]
fn completes_contract_members_after_dot() {
    let (text, offset) = extract_offset(
        r#"
contract Foo { function bar() external {} uint256 value; }
contract Main { function test() public { Foo./*caret*/ } }
"#
        .trim(),
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let labels = completions
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"bar"));
    assert!(labels.contains(&"value"));

    // Verify kinds for bar (Function) and value (Variable)
    let bar_item = completions
        .iter()
        .find(|item| item.label == "bar")
        .expect("bar completion item");
    assert_eq!(bar_item.kind, CompletionItemKind::Function);

    let value_item = completions
        .iter()
        .find(|item| item.label == "value")
        .expect("value completion item");
    assert_eq!(value_item.kind, CompletionItemKind::Variable);
}

#[test]
fn completes_import_paths() {
    let (main_text, offset) = extract_offset(
        r#"
import "src/D/*caret*/";
contract Main {}
"#
        .trim(),
    );
    let files = vec![
        (NormalizedPath::new("/workspace/src/Main.sol"), main_text),
        (
            NormalizedPath::new("/workspace/src/Dep.sol"),
            "contract Dep {}".to_string(),
        ),
    ];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let labels = completions
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"src/Dep.sol"));

    // Verify kind for the file completion is File
    let dep_item = completions
        .iter()
        .find(|item| item.label == "src/Dep.sol")
        .expect("src/Dep.sol completion item");
    assert_eq!(dep_item.kind, CompletionItemKind::File);
}

#[test]
fn completes_inherited_contract_members() {
    let (text, offset) = extract_offset(
        r#"
contract Base { function ping() public {} uint256 public baseValue; }
contract Derived is Base { function derived() public {} }
contract Main {
    function test() public {
        Derived d;
        d.p/*caret*/;
    }
}
"#
        .trim(),
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let labels = completions
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"ping"));

    let ping_item = completions
        .iter()
        .find(|item| item.label == "ping")
        .expect("ping completion item");
    assert_eq!(ping_item.kind, CompletionItemKind::Function);
}

#[test]
fn completes_super_members() {
    let (text, offset) = extract_offset(
        r#"
contract Base { function ping() internal {} function pong() internal {} }
contract Mid is Base { function ping() internal {} }
contract Derived is Mid {
    function test() internal {
        super.p/*caret*/;
    }
}
"#
        .trim(),
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let labels = completions
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"ping"));
    assert!(labels.contains(&"pong"));
}

#[test]
fn completes_struct_members() {
    let completions = completions_for_main(
        r#"
struct Data { uint256 value; address owner; }
contract Main {
    function test() public {
        Data memory data;
        data.v/*caret*/;
    }
}
"#,
    );
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"value"));
}

#[test]
fn completes_address_members() {
    let completions = completions_for_main(
        r#"
contract Main {
    function test() public {
        address target = address(this);
        target.b/*caret*/;
    }
}
"#,
    );
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"balance"));
}

#[test]
fn completes_array_members() {
    let completions = completions_for_main(
        r#"
contract Main {
    function test() public {
        uint256[] memory values;
        values.l/*caret*/;
    }
}
"#,
    );
    let labels = completions
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"length"));
}

#[test]
fn dedupes_overloaded_member_names() {
    let (text, offset) = extract_offset(
        r#"
contract Base {
    function foo(uint256 value) public {}
    function foo(bytes32 value) public {}
}
contract Main {
    function test() public {
        Base b;
        b.f/*caret*/;
    }
}
"#
        .trim(),
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let completions = sa_ide_completion::completions(&db, project_id, file_id, offset);
    let foo_count = completions
        .iter()
        .filter(|item| item.label == "foo")
        .count();
    assert_eq!(foo_count, 1);
}

#[test]
fn completes_imported_identifier_with_parse_errors() {
    let completions = completions_for_main_with_deps(
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Math} from "./Math.sol";

contract A {
    function doSomething() public pure returns (uint256) {
        return M/*caret*/
    }
}
"#,
        vec![(
            NormalizedPath::new("/workspace/src/Math.sol"),
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

library Math {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#
            .trim()
            .to_string(),
        )],
    );
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"Math"));
}

#[test]
fn completes_local_identifier_with_parse_errors() {
    let completions = completions_for_main_with_deps(
        r#"
pragma solidity ^0.8.13;

contract A {
    function doSomething() public pure returns (uint256) {
        uint256 myValue = 42;
        return my/*caret*/
    }
}
"#,
        vec![],
    );
    let labels = completion_labels(&completions);

    assert!(labels.contains(&"myValue"));
}
