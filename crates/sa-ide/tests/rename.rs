use sa_paths::NormalizedPath;
use sa_span::{TextRange, TextSize};
use sa_test_support::{extract_offset, extract_offsets, setup_analysis, slice_range};

#[test]
fn rename_produces_edits_for_all_references() {
    let (main_text, offset) =
        extract_offset("import \"./Thing.sol\"; contract Main { /*caret*/Lib lib; }");
    let files = vec![
        (
            NormalizedPath::new("/workspace/src/Main.sol"),
            main_text.clone(),
        ),
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

    let change = analysis
        .rename(main_id, offset, "Renamed")
        .expect("rename changes");
    let edits = change.edits();
    assert_eq!(edits.len(), 2);

    let main_edits = edits
        .iter()
        .find(|entry| entry.file_id == main_id)
        .expect("main edits");
    let thing_edits = edits
        .iter()
        .find(|entry| entry.file_id == thing_id)
        .expect("thing edits");

    let main_text = analysis.file_text(main_id);
    for edit in &main_edits.edits {
        assert_eq!(slice_range(&main_text, edit.range), "Lib");
        assert_eq!(edit.new_text, "Renamed");
    }

    let thing_text = analysis.file_text(thing_id);
    for edit in &thing_edits.edits {
        assert_eq!(slice_range(&thing_text, edit.range), "Lib");
        assert_eq!(edit.new_text, "Renamed");
    }
}

#[test]
fn rename_limits_edits_to_local_scope() {
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

    let change = analysis
        .rename(file_id, offset, "total")
        .expect("rename changes");
    let edits = change.edits();
    assert_eq!(edits.len(), 1);
    let file_edits = &edits[0];

    let mut ranges = file_edits
        .edits
        .iter()
        .map(|edit| edit.range)
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

    for edit in &file_edits.edits {
        assert_eq!(slice_range(&text, edit.range), "count");
        assert_eq!(edit.new_text, "total");
    }
}

#[test]
fn rename_inherited_state_variable_updates_base_and_usage() {
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

    let change = analysis
        .rename(file_id, caret_offset, "total")
        .expect("rename changes");
    let edits = change.edits();
    assert_eq!(edits.len(), 1);
    let file_edits = &edits[0];

    let mut ranges = file_edits
        .edits
        .iter()
        .map(|edit| edit.range)
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    let len = TextSize::from("value".len() as u32);
    let expected = vec![
        TextRange::at(def_offset, len),
        TextRange::at(caret_offset, len),
    ];
    assert_eq!(ranges, expected);

    for edit in &file_edits.edits {
        assert_eq!(slice_range(&text, edit.range), "value");
        assert_eq!(edit.new_text, "total");
    }
}

#[test]
fn rename_overridden_function_in_multiple_inheritance() {
    let (text, offsets) = extract_offsets(
        r#"
contract A {
    function foo() public virtual {}
}

contract B is A {
    function foo() public virtual override {}
}

contract C is A {
    function /*def*/foo() public virtual override {}
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

    let change = analysis
        .rename(file_id, caret_offset, "baz")
        .expect("rename changes");
    let edits = change.edits();
    assert_eq!(edits.len(), 1);
    let file_edits = &edits[0];

    let mut ranges = file_edits
        .edits
        .iter()
        .map(|edit| edit.range)
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    let len = TextSize::from("foo".len() as u32);
    let expected = vec![
        TextRange::at(def_offset, len),
        TextRange::at(caret_offset, len),
    ];
    assert_eq!(ranges, expected);

    for edit in &file_edits.edits {
        assert_eq!(slice_range(&text, edit.range), "foo");
        assert_eq!(edit.new_text, "baz");
    }
}

#[test]
fn rename_overloaded_function_calls() {
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

    let change = analysis
        .rename(file_id, caret_offset, "alias")
        .expect("rename changes");
    let edits = change.edits();
    assert_eq!(edits.len(), 1);
    let file_edits = &edits[0];

    let mut ranges = file_edits
        .edits
        .iter()
        .map(|edit| edit.range)
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    let len = TextSize::from("foo".len() as u32);
    let expected = vec![
        TextRange::at(def_offset, len),
        TextRange::at(caret_offset, len),
    ];
    assert_eq!(ranges, expected);

    for edit in &file_edits.edits {
        assert_eq!(slice_range(&text, edit.range), "foo");
        assert_eq!(edit.new_text, "alias");
    }
}
