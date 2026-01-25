use sa_base_db::Database;
use sa_hir::{LocalDef, LocalDefKind, Semantics, local_references};
use sa_paths::NormalizedPath;
use sa_span::{TextRange, TextSize, is_ident_byte};
use sa_test_support::{extract_offset, setup_db};

fn setup_file(text: String) -> (Database, sa_base_db::ProjectId, sa_vfs::FileId) {
    let files = vec![(NormalizedPath::new("/workspace/src/Main.sol"), text)];
    let (db, project_id, snapshot) = setup_db(files, vec![]);
    let file_id = snapshot
        .file_id(&NormalizedPath::new("/workspace/src/Main.sol"))
        .expect("file id");
    (db, project_id, file_id)
}

fn resolve_local_at(
    db: &Database,
    project_id: sa_base_db::ProjectId,
    file_id: sa_vfs::FileId,
    offset: TextSize,
) -> LocalDef {
    let semantics = Semantics::new(db, project_id);
    semantics
        .resolve_local(file_id, offset)
        .expect("local binding")
}

fn ident_ranges(text: &str, ident: &str) -> Vec<TextRange> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    for (idx, _) in text.match_indices(ident) {
        let start = idx;
        let end = idx + ident.len();
        let before = start.checked_sub(1).and_then(|i| bytes.get(i));
        let after = bytes.get(end);
        if before.is_none_or(|b| !is_ident_byte(*b)) && after.is_none_or(|b| !is_ident_byte(*b)) {
            ranges.push(TextRange::new(
                TextSize::from(start as u32),
                TextSize::from(end as u32),
            ));
        }
    }
    ranges.sort_by_key(|range| (u32::from(range.start()), u32::from(range.end())));
    ranges
}

#[test]
fn local_references_collect_various_exprs() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar(uint256 target, uint256 other) public returns (uint256) {
        uint256 value = target + other;
        target = target + 1;
        delete target;
        foo(target);
        foo{gas: target}(target);
        uint256[2] memory arr = [target, other];
        arr[target] = other;
        bytes memory data = "hello";
        data[target:other];
        bool flag = target > 0;
        uint256 result = flag ? target : other;
        (target, other) = (other, target);
        -target;
        return tar/*caret*/get;
    }

    function foo(uint256 val) internal {}
}
"#,
    );

    let (db, project_id, file_id) = setup_file(text.clone());
    let local = resolve_local_at(&db, project_id, file_id, offset);
    assert_eq!(local.kind(), LocalDefKind::Parameter);

    let ranges = local_references(&db, file_id, &local);
    let expected = ident_ranges(&text, "target");
    assert_eq!(ranges, expected);
}

#[test]
fn local_references_include_member_and_payable() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public {
        address addr = address(this);
        payable(ad/*caret*/dr);
        addr.balance;
    }
}
"#,
    );

    let (db, project_id, file_id) = setup_file(text.clone());
    let local = resolve_local_at(&db, project_id, file_id, offset);
    assert_eq!(local.kind(), LocalDefKind::Local);
    assert_eq!(local.name(), "addr");

    let ranges = local_references(&db, file_id, &local);
    let expected = ident_ranges(&text, "addr");
    assert_eq!(ranges, expected);
}

#[test]
fn resolves_locals_in_for_and_tuple_decl() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public {
        for (uint256 idx = 0; idx < 1; idx++) {
            idx/*caret*/;
        }
        (uint256 a, uint256 b) = (1, 2);
        a;
    }
}
 "#,
    );

    let (db, project_id, file_id) = setup_file(text);
    let local = resolve_local_at(&db, project_id, file_id, offset);
    assert_eq!(local.kind(), LocalDefKind::Local);
    assert_eq!(local.name(), "idx");
}

#[test]
fn resolves_local_in_if_scope() {
    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar() public {
        if (true) {
            uint256 inner = 1;
            inn/*caret*/er;
        }
    }
}
"#,
    );

    let (db, project_id, file_id) = setup_file(text);
    let local = resolve_local_at(&db, project_id, file_id, offset);
    assert_eq!(local.kind(), LocalDefKind::Local);
    assert_eq!(local.name(), "inner");
}
