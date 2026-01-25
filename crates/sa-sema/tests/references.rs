use std::collections::HashMap;

use sa_base_db::FileId;
use sa_paths::NormalizedPath;
use sa_sema::{SemaReference, SemaSnapshot};
use sa_span::{TextRange, TextSize};
use sa_test_support::{extract_offsets, find_range};
use sa_test_utils::{Fixture, FixtureBuilder};
use sa_vfs::VfsSnapshot;

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

fn file_text(snapshot: &VfsSnapshot, file_id: FileId) -> String {
    snapshot.file_text(file_id).expect("file text").to_string()
}

fn references_for_definition(
    snapshot: &SemaSnapshot,
    file_id: FileId,
    file_text: &str,
    needle: &str,
) -> Vec<SemaReference> {
    let definition_range = find_range(file_text, needle);
    snapshot
        .references_for_definition(file_id, definition_range)
        .unwrap_or(&[])
        .to_vec()
}

fn range_from_offset(offset: TextSize, len: usize) -> TextRange {
    let len = TextSize::from(len as u32);
    TextRange::new(offset, offset + len)
}

#[test]
fn references_skip_ambiguous_import_names() {
    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/A.sol",
            r#"
uint256 constant VALUE = 1;
"#,
        )
        .file(
            "src/B.sol",
            r#"
uint256 constant VALUE = 2;
"#,
        )
        .file(
            "src/Main.sol",
            r#"
import "./A.sol";
import "./B.sol";

contract Main {
    function value() public pure returns (uint256) {
        return VALUE;
    }
}
"#,
        )
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let vfs = fixture.vfs_snapshot();
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let a_file_id = fixture.file_id("src/A.sol").expect("A file id");
    let b_file_id = fixture.file_id("src/B.sol").expect("B file id");

    let a_text = file_text(vfs, a_file_id);
    let b_text = file_text(vfs, b_file_id);

    let a_refs = references_for_definition(&snapshot, a_file_id, &a_text, "VALUE");
    assert!(
        a_refs
            .iter()
            .any(|reference| reference.file_id() == a_file_id)
    );
    assert!(
        !a_refs
            .iter()
            .any(|reference| reference.file_id() == main_file_id)
    );

    let b_refs = references_for_definition(&snapshot, b_file_id, &b_text, "VALUE");
    assert!(
        b_refs
            .iter()
            .any(|reference| reference.file_id() == b_file_id)
    );
    assert!(
        !b_refs
            .iter()
            .any(|reference| reference.file_id() == main_file_id)
    );
}

#[test]
fn references_skip_ambiguous_source_alias() {
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
        .file(
            "src/Main.sol",
            r#"
import * as Lib from "./A.sol";
import * as Lib from "./B.sol";

contract Main {
    Lib.Foo foo;
}
"#,
        )
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let vfs = fixture.vfs_snapshot();
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let a_file_id = fixture.file_id("src/A.sol").expect("A file id");
    let b_file_id = fixture.file_id("src/B.sol").expect("B file id");

    let a_text = file_text(vfs, a_file_id);
    let b_text = file_text(vfs, b_file_id);

    let a_refs = references_for_definition(&snapshot, a_file_id, &a_text, "Foo");
    assert!(
        a_refs
            .iter()
            .any(|reference| reference.file_id() == a_file_id)
    );
    assert!(
        !a_refs
            .iter()
            .any(|reference| reference.file_id() == main_file_id)
    );

    let b_refs = references_for_definition(&snapshot, b_file_id, &b_text, "Foo");
    assert!(
        b_refs
            .iter()
            .any(|reference| reference.file_id() == b_file_id)
    );
    assert!(
        !b_refs
            .iter()
            .any(|reference| reference.file_id() == main_file_id)
    );
}

#[test]
fn references_include_import_aliases() {
    let (main_text, offsets) = extract_offsets(
        r#"
import {Foo as /*alias*/FooAlias} from "./A.sol";

contract Main {
    /*usage*/FooAlias foo;
}
"#,
        &["/*alias*/", "/*usage*/"],
    );
    let alias_range = range_from_offset(offsets[0], "FooAlias".len());
    let usage_range = range_from_offset(offsets[1], "FooAlias".len());

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/A.sol",
            r#"
contract Foo {}
"#,
        )
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let vfs = fixture.vfs_snapshot();
    let main_file_id = fixture.file_id("src/Main.sol").expect("main file id");
    let a_file_id = fixture.file_id("src/A.sol").expect("A file id");
    let a_text = file_text(vfs, a_file_id);

    let references = references_for_definition(&snapshot, a_file_id, &a_text, "Foo");
    assert!(
        references.iter().any(
            |reference| reference.file_id() == main_file_id && reference.range() == alias_range
        )
    );
    assert!(
        references.iter().any(
            |reference| reference.file_id() == main_file_id && reference.range() == usage_range
        )
    );
}

#[test]
fn references_include_event_error_udvt_and_state_member() {
    let (main_text, offsets) = extract_offsets(
        r#"
pragma solidity ^0.8.20;

type /*def_udvt*/UserId is uint256;

error /*def_error*/Boom(uint256 code);
event /*def_event*/Ping(uint256 value);

contract Main {
    uint256 /*def_state*/value;

    function test(/*use_udvt*/UserId id) public returns (uint256) {
        emit /*use_event*/Ping(1);
        revert /*use_error*/Boom(2);
        uint256 local = /*use_state*/value;
        this.value;
        UserId next = id;
        local = local + 1;
        return UserId.unwrap(next) + local;
    }

    function exercise(uint256 input) public returns (uint256) {
        uint256[] memory arr = new uint256[](2);
        arr[0] = input;
        arr[1] = input + 1;
        uint256 sum = arr[0] + arr[1];
        if (sum > 0) {
            sum = sum > 1 ? sum : 1;
        } else {
            sum = 0;
        }
        (uint256 a, uint256 b) = (sum, 1);
        for (uint256 i = 0; i < 1; i++) {
            if (i == 0) {
                continue;
            }
            break;
        }
        try this.test(UserId.wrap(sum)) returns (uint256 r) {
            sum = r;
        } catch Error(string memory) {
            revert Boom(sum);
        } catch (bytes memory) {
            revert Boom(sum);
        }
        string memory name = type(Main).name;
        address payable pay = payable(address(this));
        emit Ping(bytes(name).length + uint256(uint160(pay)));
        return a + b + sum;
    }
}
"#,
        &[
            "/*def_udvt*/",
            "/*def_error*/",
            "/*def_event*/",
            "/*def_state*/",
            "/*use_udvt*/",
            "/*use_event*/",
            "/*use_error*/",
            "/*use_state*/",
        ],
    );

    let def_udvt_range = range_from_offset(offsets[0], "UserId".len());
    let def_error_range = range_from_offset(offsets[1], "Boom".len());
    let def_event_range = range_from_offset(offsets[2], "Ping".len());
    let def_state_range = range_from_offset(offsets[3], "value".len());
    let use_udvt_range = range_from_offset(offsets[4], "UserId".len());
    let use_event_range = range_from_offset(offsets[5], "Ping".len());
    let use_error_range = range_from_offset(offsets[6], "Boom".len());
    let use_state_range = range_from_offset(offsets[7], "value".len());

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file("src/Main.sol", main_text)
        .build()
        .expect("fixture");

    let (snapshot, _) = snapshot_for_fixture(&fixture);
    let file_id = fixture.file_id("src/Main.sol").expect("main file id");

    let udvt_refs = snapshot
        .references_for_definition(file_id, def_udvt_range)
        .unwrap_or(&[]);
    assert!(
        udvt_refs
            .iter()
            .any(|reference| reference.range() == use_udvt_range)
    );

    let event_refs = snapshot
        .references_for_definition(file_id, def_event_range)
        .unwrap_or(&[]);
    assert!(
        event_refs
            .iter()
            .any(|reference| reference.range() == use_event_range)
    );

    let error_refs = snapshot
        .references_for_definition(file_id, def_error_range)
        .unwrap_or(&[]);
    assert!(
        error_refs
            .iter()
            .any(|reference| reference.range() == use_error_range)
    );

    let state_refs = snapshot
        .references_for_definition(file_id, def_state_range)
        .unwrap_or(&[]);
    assert!(
        state_refs
            .iter()
            .any(|reference| reference.range() == use_state_range)
    );
}
