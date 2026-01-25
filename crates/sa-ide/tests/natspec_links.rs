use sa_paths::NormalizedPath;
use sa_test_support::{extract_offset, setup_analysis};

fn hover_contents(text: &str, path: &NormalizedPath) -> String {
    let (text, offset) = extract_offset(text);
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.clone())], vec![]);
    let file_id = snapshot.file_id(path).expect("file id");
    let result = analysis.hover(file_id, offset).expect("hover result");
    result.contents
}

#[test]
fn hover_links_natspec_member_references() {
    let text = r#"
contract Governor {
    /// @notice Uses {quorum}, {Governor.quorum}, and {Governor-quorum}.
    function quorum(uint256 timepoint) public view returns (uint256) {
        return timepoint;
    }
}

contract Main {
    function run() public {
        Governor governor = new Governor();
        governor.quo/*caret*/rum(1);
    }
}
"#;
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let contents = hover_contents(text, &path);

    assert!(contents.contains("[`{quorum}`](file://"));
    assert!(contents.contains("[`{Governor.quorum}`](file://"));
    assert!(contents.contains("[`{Governor-quorum}`](file://"));
}

#[test]
fn hover_skips_links_in_code_fences_and_inline_code() {
    let text = r#"
contract Governor {
    /**
     * @dev Example:
     * ```solidity
     * {quorum}
     * ```
     * Inline `{quorum}` should stay plain.
     * But {quorum} should link.
     */
    function quorum(uint256 timepoint) public view returns (uint256) {
        return timepoint;
    }
}

contract Main {
    function run() public {
        Governor governor = new Governor();
        governor.quo/*caret*/rum(1);
    }
}
"#;
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let contents = hover_contents(text, &path);

    assert!(contents.contains("```solidity\n{quorum}\n```"));
    assert!(contents.contains("`{quorum}`"));
    assert!(contents.contains("[`{quorum}`](file://"));
}
