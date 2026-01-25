use sa_ide::{CodeActionDiagnostic, CodeActionKind};
use sa_paths::NormalizedPath;
use sa_test_support::{find_range, setup_analysis};

#[test]
fn quick_fix_mixed_case_variable() {
    let text = r#"
contract Main {
    uint256 FooBar;
}
"#
    .trim();
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let diag = CodeActionDiagnostic {
        range: find_range(text, "FooBar"),
        code: "mixed-case-variable".to_string(),
    };

    let actions = analysis.code_actions(file_id, &[diag]);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].kind, CodeActionKind::QuickFix);

    let edits = actions[0].edit.edits();
    assert_eq!(edits.len(), 1);
    let file_edit = &edits[0];
    assert_eq!(file_edit.file_id, file_id);
    assert_eq!(file_edit.edits.len(), 1);
    assert_eq!(file_edit.edits[0].new_text, "fooBar");
}

#[test]
fn quick_fix_mixed_case_function() {
    let text = r#"
contract Main {
    function Bad_Name() public {}
}
"#
    .trim();
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let diag = CodeActionDiagnostic {
        range: find_range(text, "Bad_Name"),
        code: "mixed-case-function".to_string(),
    };

    let actions = analysis.code_actions(file_id, &[diag]);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].kind, CodeActionKind::QuickFix);

    let edits = actions[0].edit.edits();
    assert_eq!(edits.len(), 1);
    let file_edit = &edits[0];
    assert_eq!(file_edit.edits.len(), 1);
    assert_eq!(file_edit.edits[0].new_text, "badName");
}

#[test]
fn quick_fix_pascal_case_struct() {
    let text = r#"
contract Main {
    struct bad_struct {
        uint256 value;
    }
}
"#
    .trim();
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let diag = CodeActionDiagnostic {
        range: find_range(text, "bad_struct"),
        code: "pascal-case-struct".to_string(),
    };

    let actions = analysis.code_actions(file_id, &[diag]);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].kind, CodeActionKind::QuickFix);

    let edits = actions[0].edit.edits();
    assert_eq!(edits.len(), 1);
    let file_edit = &edits[0];
    assert_eq!(file_edit.edits.len(), 1);
    assert_eq!(file_edit.edits[0].new_text, "BadStruct");
}
