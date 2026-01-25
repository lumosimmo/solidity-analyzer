use sa_base_db::{Database, ProjectId};
use sa_hir::{LocalDefKind, Semantics};
use sa_paths::NormalizedPath;
use sa_span::TextRange;
use sa_test_support::{extract_offset, setup_db};

fn resolve_local(
    db: &Database,
    project_id: ProjectId,
    file_id: sa_vfs::FileId,
    offset: sa_span::TextSize,
) -> (LocalDefKind, TextRange, String) {
    let semantics = Semantics::new(db, project_id);
    let local = semantics
        .resolve_local(file_id, offset)
        .expect("local binding");
    (local.kind(), local.range(), local.name().to_string())
}

#[test]
fn resolves_parameter_in_function_body() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar(uint256 value) public returns (uint256) {
        return val/*caret*/ue;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let (kind, range, name) = resolve_local(&db, project_id, file_id, offset);
    assert_eq!(kind, LocalDefKind::Parameter);
    assert_eq!(name, "value");
    let expected = sa_test_support::find_range(&text, "value");
    assert_eq!(range, expected);
}

#[test]
fn resolves_parameter_in_signature_definition() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar(uint256 val/*caret*/ue) public {
        value;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let (kind, range, name) = resolve_local(&db, project_id, file_id, offset);
    assert_eq!(kind, LocalDefKind::Parameter);
    assert_eq!(name, "value");
    let expected = sa_test_support::find_range(&text, "value");
    assert_eq!(range, expected);
}

#[test]
fn resolves_shadowed_parameter_definition() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar(uint256 val/*caret*/ue) public {
        uint256 value = 1;
        {
            uint256 value = 2;
            value;
        }
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let (kind, range, name) = resolve_local(&db, project_id, file_id, offset);
    assert_eq!(kind, LocalDefKind::Parameter);
    assert_eq!(name, "value");
    let expected = sa_test_support::find_range(&text, "value");
    assert_eq!(range, expected);
}

#[test]
fn resolves_named_return_in_function_body() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public returns (uint256 result) {
        res/*caret*/ult = 1;
        return result;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let (kind, range, name) = resolve_local(&db, project_id, file_id, offset);
    assert_eq!(kind, LocalDefKind::NamedReturn);
    assert_eq!(name, "result");
    let expected = sa_test_support::find_range(&text, "result");
    assert_eq!(range, expected);
}

#[test]
fn resolves_named_return_in_signature_definition() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public returns (uint256 res/*caret*/ult) {
        result = 1;
        return result;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let (kind, range, name) = resolve_local(&db, project_id, file_id, offset);
    assert_eq!(kind, LocalDefKind::NamedReturn);
    assert_eq!(name, "result");
    let expected = sa_test_support::find_range(&text, "result");
    assert_eq!(range, expected);
}

#[test]
fn resolves_catch_parameter_in_definition() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function other() public {}

    function bar() public {
        try this.other() {
        } catch Error(string memory reas/*caret*/on) {
            reason;
        }
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let (kind, range, name) = resolve_local(&db, project_id, file_id, offset);
    assert_eq!(kind, LocalDefKind::Parameter);
    assert_eq!(name, "reason");
    let expected = sa_test_support::find_range(&text, "reason");
    assert_eq!(range, expected);
}

#[test]
fn resolves_parameter_in_modifier_argument() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    modifier guard(uint256 amount) {
        _;
    }

    function bar(uint256 value) public guard(val/*caret*/ue) {
        value;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let (kind, range, name) = resolve_local(&db, project_id, file_id, offset);
    assert_eq!(kind, LocalDefKind::Parameter);
    assert_eq!(name, "value");
    let expected = sa_test_support::find_range(&text, "value");
    assert_eq!(range, expected);
}

#[test]
fn ignores_parameter_in_return_type() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar(uint256 Foo) public returns (Foo/*caret*/) {
        return 1;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let semantics = Semantics::new(&db, project_id);
    assert!(semantics.resolve_local(file_id, offset).is_none());
}

#[test]
fn resolves_local_variable_in_function_body() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public {
        uint256 local = 1;
        return loc/*caret*/al;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let (kind, range, name) = resolve_local(&db, project_id, file_id, offset);
    assert_eq!(kind, LocalDefKind::Local);
    assert_eq!(name, "local");
    let expected = sa_test_support::find_range(&text, "local");
    assert_eq!(range, expected);
}

#[test]
fn resolves_shadowed_local_in_nested_scope() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public {
        uint256 value = 1;
        {
            uint256 value = 2;
            return val/*caret*/ue;
        }
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text.clone())];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let (kind, range, name) = resolve_local(&db, project_id, file_id, offset);
    assert_eq!(kind, LocalDefKind::Local);
    assert_eq!(name, "value");
    let positions = text
        .match_indices("value")
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    let inner_start = positions.get(1).copied().expect("inner value");
    let expected = TextRange::new(
        sa_span::TextSize::from(inner_start as u32),
        sa_span::TextSize::from((inner_start + "value".len()) as u32),
    );
    assert_eq!(range, expected);
}

#[test]
fn ignores_local_before_declaration() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public {
        val/*caret*/ue;
        uint256 value = 1;
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let semantics = Semantics::new(&db, project_id);
    assert!(semantics.resolve_local(file_id, offset).is_none());
}

#[test]
fn ignores_local_in_comment() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public {
        uint256 value = 1;
        // val/*caret*/ue
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let semantics = Semantics::new(&db, project_id);
    assert!(semantics.resolve_local(file_id, offset).is_none());
}

#[test]
fn ignores_local_in_string_literal() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public {
        uint256 value = 1;
        string memory label = "val/*caret*/ue";
    }
}
"#,
    );
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");

    let semantics = Semantics::new(&db, project_id);
    assert!(semantics.resolve_local(file_id, offset).is_none());
}
