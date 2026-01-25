use sa_ide::HoverResult;
use sa_paths::NormalizedPath;
use sa_span::{TextRange, TextSize};
use sa_test_support::{extract_offset, find_range, setup_analysis};

#[test]
fn hover_includes_contract_docs_and_label() {
    let (text, offset) = extract_offset(
        "/// Main contract docs\ncontract Foo {}\ncontract Main { /*caret*/Foo foo; }",
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.clone())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let result = analysis.hover(file_id, offset).expect("hover result");

    let name_start = text.find("Foo foo").expect("Foo usage");
    let expected_range = TextRange::new(
        TextSize::from(name_start as u32),
        TextSize::from((name_start + 3) as u32),
    );
    assert_eq!(result.range, expected_range);
    assert_eq!(
        result.contents,
        "```solidity\ncontract Foo\n```\n\nMain contract docs"
    );
}

#[test]
fn hover_range_matches_cross_file_reference_token() {
    let (main_text, offset) = extract_offset(
        r#"
import "./Other.sol";

contract Main {
    function run() public {
        Fo/*caret*/o value;
    }
}
"#,
    );
    let other_text = r#"
// padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding
// padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding
contract Foo {}
"#;
    let main_path = NormalizedPath::new("/workspace/src/Main.sol");
    let other_path = NormalizedPath::new("/workspace/src/Other.sol");
    let (analysis, snapshot) = setup_analysis(
        vec![
            (main_path.clone(), main_text.clone()),
            (other_path.clone(), other_text.to_string()),
        ],
        vec![],
    );
    let file_id = snapshot.file_id(&main_path).expect("file id");

    let result = analysis.hover(file_id, offset).expect("hover result");

    let expected_range = find_range(&main_text, "Foo");
    assert_eq!(result.range, expected_range);
}

#[test]
fn hover_includes_function_signature_and_docs() {
    let (text, offset) = extract_offset(
        r#"contract Foo {
    /// Adds two values.
    function add(uint256 left, uint256 right) public returns (uint256) { return left + right; }
}
contract Main { function run() public { Foo foo = new Foo(); foo.ad/*caret*/d(1, 2); } }"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.clone())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let HoverResult { contents, .. } = analysis.hover(file_id, offset).expect("hover result");
    assert!(
        contents.contains(
            "```solidity\nfunction add(uint256 left, uint256 right) returns (uint256)\n```"
        )
    );
    assert!(contents.contains("Adds two values."));
}

#[test]
fn hover_includes_local_binding_label() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo(uint256 value) public {
        val/*caret*/ue;
    }
}
"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.clone())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let result = analysis.hover(file_id, offset).expect("hover result");

    let name_start = text.rfind("value").expect("value usage");
    let expected_range = TextRange::new(
        TextSize::from(name_start as u32),
        TextSize::from((name_start + 5) as u32),
    );
    assert_eq!(result.range, expected_range);
    assert!(result.contents.contains("```solidity\nparameter"));
    assert!(result.contents.contains("uint256 value"));
}

#[test]
fn hover_renders_natspec_sections() {
    let (text, offset) = extract_offset(
        r#"contract Foo {
    /// @notice Adds two values.
    /// @param left The left value.
    /// @param right The right value.
    /// @return sum The sum.
    function add(uint256 left, uint256 right) public returns (uint256 sum) {
        return left + right;
    }
}
contract Main { function run() public { Foo foo = new Foo(); foo.ad/*caret*/d(1, 2); } }"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.clone())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let result = analysis.hover(file_id, offset).expect("hover result");
    assert!(result.contents.contains(
        "```solidity\nfunction add(uint256 left, uint256 right) returns (uint256 sum)\n```"
    ));
    assert!(result.contents.contains("**Notice**"));
    assert!(result.contents.contains("**Parameters**"));
    assert!(result.contents.contains("- `left`: The left value."));
    assert!(result.contents.contains("- `right`: The right value."));
    assert!(result.contents.contains("**Returns**"));
    assert!(result.contents.contains("- `sum`: The sum."));
}

#[test]
fn hover_renders_block_natspec_sections() {
    let (text, offset) = extract_offset(
        r#"contract Foo {
    /**
     * @notice Adds two values.
     * @param left The left value.
     * @return sum The sum.
     */
    function add(uint256 left) public returns (uint256 sum) {
        return left + 1;
    }
}
contract Main { function run() public { Foo foo = new Foo(); foo.ad/*caret*/d(1); } }"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.clone())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let result = analysis.hover(file_id, offset).expect("hover result");
    assert!(result.contents.contains("**Notice**"));
    assert!(result.contents.contains("Adds two values."));
    assert!(result.contents.contains("**Parameters**"));
    assert!(result.contents.contains("- `left`: The left value."));
    assert!(result.contents.contains("**Returns**"));
    assert!(result.contents.contains("- `sum`: The sum."));
}

#[test]
fn hover_includes_state_variable_label() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    uint256 public count;
    function run() public {
        cou/*caret*/nt;
    }
}
"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.clone())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let result = analysis.hover(file_id, offset).expect("hover result");

    assert_eq!(result.contents, "```solidity\nuint256 count\n```");
}

#[test]
fn hover_uses_def_kind_labels_for_non_function_items() {
    let text = r#"
type Price is uint256;

struct Data { uint256 value; }
enum Choice { A, B }
event Logged(uint256 value);
error Failure();
modifier OnlyOwner() { _; }

contract Main {
    function run() public OnlyOwner {
        Data memory data;
        Choice choice = Choice.A;
        emit Logged(1);
        revert Failure();
        Price price = Price.wrap(1);
    }
}
"#;
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let data_offset = find_range(text, "Data memory data").start();
    let choice_offset = find_range(text, "Choice choice").start();
    let event_offset = find_range(text, "Logged(1)").start();
    let modifier_offset = find_range(text, "OnlyOwner {").start();
    let price_offset = find_range(text, "Price price").start();
    let error_start = text.find("revert Failure();").expect("error usage");
    let error_offset = TextSize::from((error_start + "revert ".len()) as u32);

    let data_hover = analysis.hover(file_id, data_offset).expect("struct hover");
    let choice_hover = analysis.hover(file_id, choice_offset).expect("enum hover");
    let event_hover = analysis.hover(file_id, event_offset).expect("event hover");
    let modifier_hover = analysis
        .hover(file_id, modifier_offset)
        .expect("modifier hover");
    let price_hover = analysis.hover(file_id, price_offset).expect("udvt hover");
    let error_hover = analysis.hover(file_id, error_offset).expect("error hover");

    assert_eq!(data_hover.contents, "```solidity\nstruct Data\n```");
    assert_eq!(choice_hover.contents, "```solidity\nenum Choice\n```");
    assert_eq!(event_hover.contents, "```solidity\nevent Logged\n```");
    assert_eq!(
        modifier_hover.contents,
        "```solidity\nmodifier OnlyOwner()\n```"
    );
    assert_eq!(price_hover.contents, "```solidity\ntype Price\n```");
    assert_eq!(error_hover.contents, "```solidity\nerror Failure\n```");
}

#[test]
fn hover_includes_named_return_label() {
    let (text, offset) = extract_offset(
        r#"
struct Dummy { }

contract Main {
    function foo() public returns (uint256 sum) {
        su/*caret*/m = 1;
        return sum;
    }
}
"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.clone())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let result = analysis.hover(file_id, offset).expect("hover result");

    assert!(
        result
            .contents
            .contains("```solidity\nreturn uint256 sum\n```")
    );
}

#[test]
fn hover_resolves_local_in_scoped_statements() {
    let text = r#"
contract Main {
    function foo() public {
        for (uint256 loopIdx = 0; loopIdx < 1; loopIdx++) {
            loopIdx;
        }
        if (true) { } else {
            uint256 elseValue = 1;
            elseValue;
        }
        while (false) {
            uint256 whileValue = 1;
            whileValue;
        }
        do {
            uint256 doValue = 1;
            doValue;
        } while (false);
    }
}
"#;
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let loop_offset = find_range(text, "loopIdx;").start();
    let else_offset = find_range(text, "elseValue;").start();
    let while_offset = find_range(text, "whileValue;").start();
    let do_offset = find_range(text, "doValue;").start();

    let loop_hover = analysis.hover(file_id, loop_offset).expect("for hover");
    let else_hover = analysis.hover(file_id, else_offset).expect("else hover");
    let while_hover = analysis.hover(file_id, while_offset).expect("while hover");
    let do_hover = analysis.hover(file_id, do_offset).expect("do hover");

    assert!(loop_hover.contents.contains("local uint256 loopIdx"));
    assert!(else_hover.contents.contains("local uint256 elseValue"));
    assert!(while_hover.contents.contains("local uint256 whileValue"));
    assert!(do_hover.contents.contains("local uint256 doValue"));
}

#[test]
fn hover_resolves_local_in_blocks_and_tuple_decl() {
    let text = r#"
contract Main {
    function foo() public {
        {
            uint256 blockValue = 1;
            blockValue;
        }
        unchecked {
            uint256 uncheckedValue = 1;
            uncheckedValue;
        }
        (uint256 left, uint256 right) = (1, 2);
        right;
    }
}
"#;
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let block_offset = find_range(text, "blockValue;").start();
    let unchecked_offset = find_range(text, "uncheckedValue;").start();
    let tuple_offset = find_range(text, "right;").start();

    let block_hover = analysis.hover(file_id, block_offset).expect("block hover");
    let unchecked_hover = analysis
        .hover(file_id, unchecked_offset)
        .expect("unchecked hover");
    let tuple_hover = analysis.hover(file_id, tuple_offset).expect("tuple hover");

    assert!(block_hover.contents.contains("local uint256 blockValue"));
    assert!(
        unchecked_hover
            .contents
            .contains("local uint256 uncheckedValue")
    );
    assert!(tuple_hover.contents.contains("local uint256 right"));
}

#[test]
fn hover_resolves_local_in_try_clause() {
    let text = r#"
contract Main {
    function foo() public {
        try this.bar() returns (uint256 value) {
            value;
        } catch Error(string memory reason) {
            uint256 catchValue = 1;
            catchValue;
            reason;
        }
    }

    function bar() public pure returns (uint256) {
        return 1;
    }
}
"#;
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let reason_offset = find_range(text, "catchValue;").start();
    let result = analysis
        .hover(file_id, reason_offset)
        .expect("hover result");

    assert!(result.contents.contains("local uint256 catchValue"));
}
