use std::collections::HashMap;

use sa_base_db::FileId;
use sa_paths::NormalizedPath;
use sa_sema::{ResolveOutcome, ResolvedSymbolKind, SemaSnapshot};
use sa_span::{TextRange, TextSize};
use sa_test_support::{extract_offset, extract_offsets};
use sa_test_utils::{Fixture, FixtureBuilder};

fn snapshot_for_fixture(fixture: &Fixture) -> (SemaSnapshot, HashMap<NormalizedPath, FileId>) {
    let path_to_file_id = fixture
        .vfs_snapshot()
        .iter()
        .map(|(file_id, path)| (path.clone(), file_id))
        .collect::<HashMap<_, _>>();
    let snapshot = SemaSnapshot::new(
        fixture.config(),
        fixture.vfs_snapshot(),
        &path_to_file_id,
        None,
        true,
    )
    .expect("sema snapshot");
    (snapshot, path_to_file_id)
}

fn resolve_at(snapshot: &SemaSnapshot, file_id: FileId, offset: TextSize) -> ResolveOutcome {
    snapshot.resolve_definition(file_id, offset)
}

fn range_from_offset(offset: TextSize, len: usize) -> TextRange {
    let len = TextSize::from(len as u32);
    TextRange::new(offset, offset + len)
}

#[test]
fn resolve_unresolved_for_ambiguous_import_type() {
    let (main_text, offset) = extract_offset(
        r#"
import "./A.sol";
import "./B.sol";

contract Main {
    /*caret*/Foo value;
}
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/A.sol",
            r#"
contract Foo {}
"#,
        )
        .file(
            "src/B.sol",
            r#"
contract Foo {}
"#,
        )
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let outcome = resolve_at(&snapshot, main_file_id, offset);

    match outcome {
        ResolveOutcome::Unresolved { .. } => {}
        other => panic!("expected unresolved, got {other:?}"),
    }
}

#[test]
fn resolve_overload_prefers_literal_arg_types() {
    let (main_text, offsets) = extract_offsets(
        r#"
contract Main {
    function /*def_uint*/foo(uint256 x) public {}
    function /*def_bytes*/foo(bytes32 x) public {}

    function test() public {
        /*call*/foo(1);
    }
}
"#,
        &["/*def_uint*/", "/*def_bytes*/", "/*call*/"],
    );
    let def_uint_range = range_from_offset(offsets[0], "foo".len());
    let call_offset = offsets[2];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let outcome = resolve_at(&snapshot, main_file_id, call_offset);

    let ResolveOutcome::Resolved(symbol) = outcome else {
        panic!("expected resolved outcome");
    };

    assert_eq!(symbol.kind, ResolvedSymbolKind::Function);
    assert_eq!(symbol.definition_range, def_uint_range);
}

#[test]
fn resolve_overload_prefers_c3_order_for_non_literal_args() {
    let (main_text, offsets) = extract_offsets(
        r#"
contract Base {
    function /*base*/foo(uint256 x) public {}
}

contract Derived is Base {
    function /*derived*/foo(uint256 x) public {}

    function test(uint256 x) public {
        /*call*/foo(x);
    }
}
"#,
        &["/*base*/", "/*derived*/", "/*call*/"],
    );
    let derived_range = range_from_offset(offsets[1], "foo".len());
    let call_offset = offsets[2];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let outcome = resolve_at(&snapshot, main_file_id, call_offset);

    let ResolveOutcome::Resolved(symbol) = outcome else {
        panic!("expected resolved outcome");
    };

    assert_eq!(symbol.kind, ResolvedSymbolKind::Function);
    assert_eq!(symbol.container.as_deref(), Some("Derived"));
    assert_eq!(symbol.definition_range, derived_range);
}

#[test]
fn resolve_unresolved_for_ambiguous_module_alias() {
    let (main_text, offset) = extract_offset(
        r#"
import * as Utils from "./A.sol";
import * as Utils from "./B.sol";

contract Main {
    Utils./*caret*/Foo value;
}
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/A.sol",
            r#"
contract Foo {}
"#,
        )
        .file(
            "src/B.sol",
            r#"
contract Foo {}
"#,
        )
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let outcome = resolve_at(&snapshot, main_file_id, offset);

    match outcome {
        ResolveOutcome::Unresolved { .. } => {}
        other => panic!("expected unresolved, got {other:?}"),
    }
}

#[test]
fn resolve_unresolved_for_missing_member_access() {
    let (main_text, offset) = extract_offset(
        r#"
contract Foo {
    uint256 value;
}

contract Main {
    function test() public {
        Foo foo;
        foo./*caret*/missing;
    }
}
"#,
    );
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let outcome = resolve_at(&snapshot, main_file_id, offset);

    match outcome {
        ResolveOutcome::Unresolved { .. } => {}
        other => panic!("expected unresolved, got {other:?}"),
    }
}

#[test]
fn resolve_unresolved_for_ambiguous_literal_overload() {
    let (main_text, offsets) = extract_offsets(
        r#"
contract Main {
    function foo(uint256 x) public {}
    function foo(int256 x) public {}

    function test() public {
        /*call*/foo(0);
    }
}
"#,
        &["/*call*/"],
    );
    let call_offset = offsets[0];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let outcome = resolve_at(&snapshot, main_file_id, call_offset);

    match outcome {
        ResolveOutcome::Unresolved { .. } => {}
        other => panic!("expected unresolved, got {other:?}"),
    }
}

#[test]
fn resolve_super_call_resolves_base_method() {
    let (main_text, offsets) = extract_offsets(
        r#"
contract Base {
    function /*base*/ping() public virtual {}
}

contract Derived is Base {
    function ping() public override {}

    function test() public {
        super./*call*/ping();
    }
}
"#,
        &["/*base*/", "/*call*/"],
    );
    let base_range = range_from_offset(offsets[0], "ping".len());
    let call_offset = offsets[1];

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let outcome = resolve_at(&snapshot, main_file_id, call_offset);

    let ResolveOutcome::Resolved(symbol) = outcome else {
        panic!("expected resolved outcome");
    };

    assert_eq!(symbol.kind, ResolvedSymbolKind::Function);
    assert_eq!(symbol.container.as_deref(), Some("Base"));
    assert_eq!(symbol.definition_range, base_range);
}

#[test]
fn resolve_returns_unavailable_when_offset_has_no_symbol() {
    let (main_text, offset) = extract_offset(
        r#"
contract Main {
    function test() public {
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

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let outcome = resolve_at(&snapshot, main_file_id, offset);

    assert!(matches!(outcome, ResolveOutcome::Unavailable));
}

#[test]
fn resolve_named_args_overload_is_unresolved() {
    let (main_text, offset) = extract_offset(
        r#"
contract Main {
    function foo(uint256 x) public {}
    function foo(address x) public {}

    function test() public {
        /*caret*/foo({x: 1});
    }
}
"#,
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let outcome = resolve_at(&snapshot, main_file_id, offset);

    match outcome {
        ResolveOutcome::Unresolved { .. } => {}
        other => panic!("expected unresolved, got {other:?}"),
    }
}

#[test]
fn resolve_udvt_event_error_and_state_var() {
    let (main_text, offsets) = extract_offsets(
        r#"
pragma solidity ^0.8.20;

type UserId is uint256;

error Boom(uint256 code);
event Ping(uint256 value);

contract Main {
    uint256 value;

    function test(/*udvt*/UserId id) public {
        emit /*event*/Ping(1);
        revert /*error*/Boom(2);
        uint256 local = /*state*/value;
        UserId other = id;
        local = local + UserId.unwrap(other);
    }
}
"#,
        &["/*udvt*/", "/*event*/", "/*error*/", "/*state*/"],
    );

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");

    let outcome = resolve_at(&snapshot, main_file_id, offsets[0]);
    let ResolveOutcome::Resolved(symbol) = outcome else {
        panic!("expected udvt resolved outcome");
    };
    assert_eq!(symbol.kind, ResolvedSymbolKind::Udvt);
    assert_eq!(symbol.name, "UserId");

    let outcome = resolve_at(&snapshot, main_file_id, offsets[1]);
    let ResolveOutcome::Resolved(symbol) = outcome else {
        panic!("expected event resolved outcome");
    };
    assert_eq!(symbol.kind, ResolvedSymbolKind::Event);
    assert_eq!(symbol.name, "Ping");

    let outcome = resolve_at(&snapshot, main_file_id, offsets[2]);
    let ResolveOutcome::Resolved(symbol) = outcome else {
        panic!("expected error resolved outcome");
    };
    assert_eq!(symbol.kind, ResolvedSymbolKind::Error);
    assert_eq!(symbol.name, "Boom");

    let outcome = resolve_at(&snapshot, main_file_id, offsets[3]);
    let ResolveOutcome::Resolved(symbol) = outcome else {
        panic!("expected state var resolved outcome");
    };
    assert_eq!(symbol.kind, ResolvedSymbolKind::Variable);
    assert_eq!(symbol.name, "value");
}
