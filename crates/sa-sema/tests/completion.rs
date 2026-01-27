use std::collections::HashMap;

use sa_base_db::FileId;
use sa_paths::NormalizedPath;
use sa_sema::{SemaCompletionKind, SemaSnapshot};
use sa_span::TextRange;
use sa_test_support::{extract_offset, extract_offsets};
use sa_test_utils::{Fixture, FixtureBuilder};

fn snapshot_for_fixture(fixture: &Fixture) -> SemaSnapshot {
    let path_to_file_id = fixture
        .vfs_snapshot()
        .iter()
        .map(|(file_id, path)| (path.clone(), file_id))
        .collect::<HashMap<NormalizedPath, FileId>>();
    SemaSnapshot::new(
        fixture.config(),
        fixture.vfs_snapshot(),
        &path_to_file_id,
        None,
        true,
    )
    .expect("sema snapshot")
}

fn completion_labels(items: &[sa_sema::SemaCompletionItem]) -> Vec<&str> {
    items.iter().map(|item| item.label.as_str()).collect()
}

#[test]
fn identifier_completions_include_import_variants_and_dedupe() {
    let (main_text, offset) = extract_offset(
        r#"
pragma solidity ^0.8.20;

import "./Dep.sol";
import "./Dep.sol";
import "./Dep.sol" as DepAlias;
import * as Glob from "./Dep.sol";
import {Foo as Renamed} from "./Dep.sol";

contract Main {
    function test() public {
        /*caret*/
    }
}
"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Dep.sol",
            r#"
pragma solidity ^0.8.20;

contract Foo {}
"#,
        )
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let snapshot = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let items = snapshot
        .identifier_completions(main_file_id, offset)
        .expect("completions");

    let foo_count = items
        .iter()
        .filter(|item| item.label == "Foo" && item.kind == SemaCompletionKind::Contract)
        .count();
    assert_eq!(foo_count, 1, "expected Foo to be deduped");

    assert!(
        items
            .iter()
            .any(|item| item.label == "DepAlias" && item.kind == SemaCompletionKind::Type),
        "expected module alias import completion"
    );
    assert!(
        items
            .iter()
            .any(|item| item.label == "Glob" && item.kind == SemaCompletionKind::Type),
        "expected glob import completion"
    );
    assert!(
        items
            .iter()
            .any(|item| item.label == "Renamed" && item.kind == SemaCompletionKind::Contract),
        "expected renamed import completion"
    );
}

#[test]
fn identifier_completions_respect_local_scope_ordering() {
    let (main_text, offset) = extract_offset(
        r#"
pragma solidity ^0.8.20;

contract Main {
    uint256 stateValue;

    function test(uint256 param) public {
        uint256 before = 1;
        /*caret*/
        uint256 later = 2;
    }
}
"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let snapshot = snapshot_for_fixture(&fixture);
    let file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let items = snapshot
        .identifier_completions(file_id, offset)
        .expect("completions");
    let labels = completion_labels(&items);

    assert!(labels.contains(&"stateValue"));
    assert!(labels.contains(&"param"));
    assert!(labels.contains(&"before"));
    assert!(!labels.contains(&"later"));
}

#[test]
fn member_completions_resolve_contract_and_variable_receivers() {
    let (main_text, offsets) = extract_offsets(
        r#"
pragma solidity ^0.8.20;

contract Foo { function bar() public {} uint256 public value; }

contract Main {
    Foo foo;

    function test() public {
        /*type_start*/Foo/*type_end*/./*type_caret*/bar();
        /*var_start*/foo/*var_end*/./*var_caret*/bar();
    }
}
"#,
        &[
            "/*type_start*/",
            "/*type_end*/",
            "/*type_caret*/",
            "/*var_start*/",
            "/*var_end*/",
            "/*var_caret*/",
        ],
    );

    let type_range = TextRange::new(offsets[0], offsets[1]);
    let type_offset = offsets[2];
    let var_range = TextRange::new(offsets[3], offsets[4]);
    let var_offset = offsets[5];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let snapshot = snapshot_for_fixture(&fixture);
    let file_id = fixture.file_id("src/Main.sol").expect("main file id");

    let type_items = snapshot
        .member_completions(file_id, type_offset, type_range, "Foo")
        .expect("type member completions");
    let type_labels = completion_labels(&type_items);
    assert!(type_labels.contains(&"bar"));
    assert!(type_labels.contains(&"value"));

    let var_items = snapshot
        .member_completions(file_id, var_offset, var_range, "foo")
        .expect("variable member completions");
    let var_labels = completion_labels(&var_items);
    assert!(var_labels.contains(&"bar"));
    assert!(var_labels.contains(&"value"));
}

#[test]
fn member_completions_handle_super_and_this_receivers() {
    let (main_text, offsets) = extract_offsets(
        r#"
pragma solidity ^0.8.20;

contract Base {
    function baseFn() public {}
    function hidden() private {}
    modifier OnlyOwner() { _; }
}

contract Child is Base {
    function childFn() public {}

    function test() public {
        /*super_start*/super/*super_end*/./*super_caret*/baseFn();
        /*this_start*/this/*this_end*/./*this_caret*/childFn();
    }
}
"#,
        &[
            "/*super_start*/",
            "/*super_end*/",
            "/*super_caret*/",
            "/*this_start*/",
            "/*this_end*/",
            "/*this_caret*/",
        ],
    );

    let super_range = TextRange::new(offsets[0], offsets[1]);
    let super_offset = offsets[2];
    let this_range = TextRange::new(offsets[3], offsets[4]);
    let this_offset = offsets[5];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let snapshot = snapshot_for_fixture(&fixture);
    let file_id = fixture.file_id("src/Main.sol").expect("main file id");

    let super_items = snapshot
        .member_completions(file_id, super_offset, super_range, "super")
        .expect("super completions");
    let super_labels = completion_labels(&super_items);
    assert!(super_labels.contains(&"baseFn"));
    assert!(!super_labels.contains(&"OnlyOwner"));

    let this_items = snapshot
        .member_completions(file_id, this_offset, this_range, "this")
        .expect("this completions");
    let this_labels = completion_labels(&this_items);
    assert!(this_labels.contains(&"childFn"));
}

#[test]
fn member_completions_include_non_contract_type_members() {
    let (main_text, offsets) = extract_offsets(
        r#"
pragma solidity ^0.8.20;

contract Main {
    function test() public {
        address target = address(this);
        /*recv_start*/target/*recv_end*/./*caret*/balance;
    }
}
"#,
        &["/*recv_start*/", "/*recv_end*/", "/*caret*/"],
    );

    let recv_range = TextRange::new(offsets[0], offsets[1]);
    let caret_offset = offsets[2];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let snapshot = snapshot_for_fixture(&fixture);
    let file_id = fixture.file_id("src/Main.sol").expect("main file id");

    let items = snapshot
        .member_completions(file_id, caret_offset, recv_range, "target")
        .expect("address completions");
    let labels = completion_labels(&items);
    assert!(labels.contains(&"balance"));
}

#[test]
fn member_completions_fallback_to_contract_by_name() {
    let (main_text, offset) = extract_offset(
        r#"
pragma solidity ^0.8.20;

contract Foo { function ping() public {} }

contract Main {
    function test() public {
        uint256 value = 1;
        /*caret*/
    }
}
"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let snapshot = snapshot_for_fixture(&fixture);
    let file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let receiver_range = TextRange::new(offset, offset);

    let items = snapshot
        .member_completions(file_id, offset, receiver_range, "Foo")
        .expect("contract fallback completions");
    let labels = completion_labels(&items);
    assert!(labels.contains(&"ping"));
}

#[test]
fn member_completions_respect_visibility_and_constants_on_contract_types() {
    let (main_text, offsets) = extract_offsets(
        r#"
pragma solidity ^0.8.20;

contract Base {
    uint256 public pubValue;
    uint256 internal internalValue;
    uint256 public constant CONST = 1;

    function pubFn() public {}
    function extFn() external {}
    function internalFn() internal {}
    function privFn() private {}
}

contract Child is Base {
    function test() public {
        Base base;
        /*inst_start*/base/*inst_end*/./*inst_caret*/pubFn();
        /*type_start*/Base/*type_end*/./*type_caret*/pubFn();
    }
}
"#,
        &[
            "/*inst_start*/",
            "/*inst_end*/",
            "/*inst_caret*/",
            "/*type_start*/",
            "/*type_end*/",
            "/*type_caret*/",
        ],
    );

    let inst_range = TextRange::new(offsets[0], offsets[1]);
    let inst_offset = offsets[2];
    let type_range = TextRange::new(offsets[3], offsets[4]);
    let type_offset = offsets[5];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let snapshot = snapshot_for_fixture(&fixture);
    let file_id = fixture.file_id("src/Main.sol").expect("main file id");

    let inst_items = snapshot
        .member_completions(file_id, inst_offset, inst_range, "base")
        .expect("instance completions");
    let inst_labels = completion_labels(&inst_items);
    assert!(inst_labels.contains(&"pubFn"));
    assert!(inst_labels.contains(&"extFn"));
    assert!(inst_labels.contains(&"pubValue"));
    assert!(!inst_labels.contains(&"internalFn"));
    assert!(!inst_labels.contains(&"privFn"));
    assert!(!inst_labels.contains(&"internalValue"));

    let type_items = snapshot
        .member_completions(file_id, type_offset, type_range, "Base")
        .expect("type completions");
    let type_labels = completion_labels(&type_items);
    assert!(type_labels.contains(&"pubFn"));
    assert!(type_labels.contains(&"extFn"));
    assert!(type_labels.contains(&"internalFn"));
    assert!(type_labels.contains(&"CONST"));
    assert!(!type_labels.contains(&"privFn"));
    assert!(!type_labels.contains(&"internalValue"));
}

#[test]
fn member_completions_include_interface_members() {
    let (main_text, offsets) = extract_offsets(
        r#"
pragma solidity ^0.8.20;

interface IFoo {
    function ping() external;
    function pong() external;
}

contract Main {
    IFoo foo;

    function test() public {
        /*recv_start*/foo/*recv_end*/./*caret*/ping();
    }
}
"#,
        &["/*recv_start*/", "/*recv_end*/", "/*caret*/"],
    );

    let recv_range = TextRange::new(offsets[0], offsets[1]);
    let caret_offset = offsets[2];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let snapshot = snapshot_for_fixture(&fixture);
    let file_id = fixture.file_id("src/Main.sol").expect("main file id");

    let items = snapshot
        .member_completions(file_id, caret_offset, recv_range, "foo")
        .expect("interface completions");
    let labels = completion_labels(&items);
    assert!(labels.contains(&"ping"));
    assert!(labels.contains(&"pong"));
}

#[test]
fn member_completions_include_library_functions() {
    let (main_text, offsets) = extract_offsets(
        r#"
pragma solidity ^0.8.20;

library Lib {
    function pubFn() public {}
    function internalFn() internal {}
    function privFn() private {}
}

contract Main {
    function test() public {
        /*lib_start*/Lib/*lib_end*/./*lib_caret*/pubFn();
    }
}
"#,
        &["/*lib_start*/", "/*lib_end*/", "/*lib_caret*/"],
    );

    let lib_range = TextRange::new(offsets[0], offsets[1]);
    let lib_offset = offsets[2];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let snapshot = snapshot_for_fixture(&fixture);
    let file_id = fixture.file_id("src/Main.sol").expect("main file id");

    let items = snapshot
        .member_completions(file_id, lib_offset, lib_range, "Lib")
        .expect("library completions");
    let labels = completion_labels(&items);
    assert!(labels.contains(&"pubFn"));
    assert!(labels.contains(&"internalFn"));
    assert!(!labels.contains(&"privFn"));
}
