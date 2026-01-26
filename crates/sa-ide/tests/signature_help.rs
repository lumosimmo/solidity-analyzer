use sa_ide::SignatureHelp;
use sa_paths::NormalizedPath;
use sa_test_support::{extract_offset, setup_analysis};

#[test]
fn signature_help_returns_function_signature_and_docs() {
    let (text, offset) = extract_offset(
        r#"contract Foo {
    /// Multiplies values.
    function mul(uint256 left, uint256 right) public returns (uint256) { return left * right; }

    function test() public { mul(1, /*caret*/2); }
}"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text)], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let SignatureHelp {
        signatures,
        active_parameter,
        ..
    } = analysis
        .signature_help(file_id, offset)
        .expect("signature help");
    assert_eq!(signatures.len(), 1);
    assert_eq!(active_parameter, Some(1));
    let signature = &signatures[0];
    assert_eq!(
        signature.label,
        "function mul(uint256 left, uint256 right) returns (uint256)"
    );
    assert!(
        signature
            .documentation
            .as_ref()
            .is_some_and(|doc| doc.contains("Multiplies values."))
    );
    assert_eq!(signature.parameters.len(), 2);
    assert_eq!(signature.parameters[0].label, "uint256 left");
}

#[test]
fn signature_help_uses_sema_type_printer() {
    let (text, offset) = extract_offset(
        r#"contract Foo {
    function cast(uint value) public returns (uint) { return value; }

    function test() public { cast(/*caret*/1); }
}"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text)], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let SignatureHelp { signatures, .. } = analysis
        .signature_help(file_id, offset)
        .expect("signature help");

    let signature = &signatures[0];
    assert_eq!(
        signature.label,
        "function cast(uint256 value) returns (uint256)"
    );
    assert_eq!(signature.parameters[0].label, "uint256 value");
}

#[test]
fn signature_help_clamps_active_parameter_to_last() {
    // Test that when cursor is past the last argument, active_parameter is clamped
    let (text, offset) = extract_offset(
        r#"contract Foo {
    function add(uint256 a) public returns (uint256) { return a; }

    function test() public { add(1, 2, /*caret*/3); }
}"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text)], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let SignatureHelp {
        active_parameter, ..
    } = analysis
        .signature_help(file_id, offset)
        .expect("signature help");
    // Function has 1 parameter (index 0), active should be clamped to 0
    assert_eq!(active_parameter, Some(0));
}

#[test]
fn signature_help_handles_arrays_in_arguments() {
    // Test that commas inside inline array literals are not counted as argument separators
    let (text, offset) = extract_offset(
        r#"contract Foo {
    function process(uint256[3] memory arr, uint256 x) public {}

    function test() public {
        process([1, 2, 3], /*caret*/5);
    }
}"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text)], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let SignatureHelp {
        active_parameter, ..
    } = analysis
        .signature_help(file_id, offset)
        .expect("signature help");
    // Commas inside [1, 2, 3] should not be counted; we're on parameter 1
    assert_eq!(active_parameter, Some(1));
}

#[test]
fn signature_help_resolves_same_name_functions_in_different_contracts() {
    // Test that when two contracts have functions with the same name,
    // we resolve to the correct one based on the call site (scoped to Beta)
    let (text, offset) = extract_offset(
        r#"contract Alpha {
    /// Alpha's compute function with 3 params
    function compute(uint256 a, uint256 b, uint256 c) public returns (uint256) {
        return a + b + c;
    }
}

contract Beta {
    /// Beta's compute function with 2 params
    function compute(uint256 x, uint256 y) public returns (uint256) {
        return x * y;
    }

    function test() public {
        compute(1, /*caret*/2);
    }
}"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text)], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let result = analysis.signature_help(file_id, offset);
    assert!(result.is_some());
    let SignatureHelp { signatures, .. } = result.unwrap();
    assert_eq!(signatures.len(), 1);
    // Should resolve to Beta.compute which has 2 parameters (not Alpha's 3)
    assert_eq!(
        signatures[0].parameters.len(),
        2,
        "Expected Beta.compute with 2 params, not Alpha.compute with 3"
    );
}

#[test]
fn signature_help_includes_natspec_sections() {
    let (text, offset) = extract_offset(
        r#"contract Foo {
    /// @notice Multiplies values.
    /// @param left The left value.
    /// @param right The right value.
    /// @return product The product.
    function mul(uint256 left, uint256 right) public returns (uint256 product) {
        return left * right;
    }

    function test() public { mul(1, /*caret*/2); }
}"#,
    );
    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text)], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let SignatureHelp { signatures, .. } = analysis
        .signature_help(file_id, offset)
        .expect("signature help");
    let docs = signatures[0].documentation.as_ref().expect("documentation");
    assert!(docs.contains("**Notice**"));
    assert!(docs.contains("**Parameters**"));
    assert!(docs.contains("- `left`: The left value."));
    assert!(docs.contains("- `right`: The right value."));
    assert!(docs.contains("**Returns**"));
    assert!(docs.contains("- `product`: The product."));
}
