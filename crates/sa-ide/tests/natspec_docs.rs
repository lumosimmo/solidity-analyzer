use sa_ide::docs_for_item;
use sa_syntax::{
    Parse,
    ast::{Item, ItemFunction, ItemKind},
};

#[test]
fn natspec_docs_render_markdown_sections() {
    let text = r#"contract Foo {
    /// @notice Adds two values.
    /// @dev Used for demos.
    /// @param left The left value.
    /// @param right The right value.
    /// @return sum The sum.
    function add(uint256 left, uint256 right) public returns (uint256 sum) {
        return left + right;
    }
}"#;
    let docs = docs_for_function(text, "add").expect("docs");

    assert!(docs.contains("**Notice**"));
    assert!(docs.contains("Adds two values."));
    assert!(docs.contains("**Dev**"));
    assert!(docs.contains("Used for demos."));
    assert!(docs.contains("**Parameters**"));
    assert!(docs.contains("- `left`: The left value."));
    assert!(docs.contains("- `right`: The right value."));
    assert!(docs.contains("**Returns**"));
    assert!(docs.contains("- `sum`: The sum."));
}

#[test]
fn natspec_docs_falls_back_to_plain_docs_without_tags() {
    let text = r#"contract Foo {
    /// Plain docs line one.
    /// Plain docs line two.
    function ping() public {}
}"#;
    let docs = docs_for_function(text, "ping").expect("docs");

    assert_eq!(docs, "Plain docs line one.\nPlain docs line two.");
}

#[test]
fn natspec_docs_concatenates_continuation_lines() {
    let text = r#"contract Foo {
    /// @notice A modifier that allows only the address recorded as an owner of the address prefix to call the function.
    /// @dev The owner of an address prefix is an address that matches the address that has previously been recorded (or
    /// will be) as an owner in the ownerLookup.
    /// @param addressPrefix The address prefix for which it is checked whether the caller is the owner.
    function check(bytes4 addressPrefix) public {}
}"#;
    let docs = docs_for_function(text, "check").expect("docs");

    let expected = r#"**Notice**
A modifier that allows only the address recorded as an owner of the address prefix to call the function.

**Dev**
The owner of an address prefix is an address that matches the address that has previously been recorded (or will be) as an owner in the ownerLookup.

**Parameters**
- `addressPrefix`: The address prefix for which it is checked whether the caller is the owner."#;
    assert_eq!(docs, expected);
}

#[test]
fn natspec_docs_none_when_no_documentation() {
    let text = r#"contract Foo {
    function foo() public {}
}"#;
    let docs = docs_for_function(text, "foo");

    assert!(docs.is_none());
}

#[test]
fn natspec_docs_mixed_plain_and_tagged_lines() {
    let text = r#"contract Foo {
    /// Summary line.
    /// @notice Adds two values.
    /// @param left The left value.
    function add(uint256 left) public {}
}"#;
    let docs = docs_for_function(text, "add").expect("docs");

    assert!(docs.contains("Summary line."));
    assert!(docs.contains("**Notice**"));
    assert!(docs.contains("Adds two values."));
    assert!(docs.contains("**Parameters**"));
    assert!(docs.contains("- `left`: The left value."));
}

#[test]
fn natspec_docs_malformed_tags_are_plain_text() {
    let text = r#"contract Foo {
    /// @param
    function foo(uint256 x) public {}
}"#;
    let docs = docs_for_function(text, "foo").expect("docs");

    assert_eq!(docs, "@param");
}

#[test]
fn natspec_docs_renders_empty_param_description() {
    let text = r#"contract Foo {
    /// @param x
    function foo(uint256 x) public {}
}"#;
    let docs = docs_for_function(text, "foo").expect("docs");

    assert_eq!(docs, "**Parameters**\n- `x`");
}

#[test]
fn natspec_docs_renders_multiple_authors() {
    let text = r#"contract Foo {
    /// @author Alice
    /// @author Bob
    function foo() public {}
}"#;
    let docs = docs_for_function(text, "foo").expect("docs");

    assert_eq!(docs, "**Author**\nAlice\n\nBob");
}

#[test]
fn natspec_docs_renders_custom_tags() {
    let text = r#"contract Foo {
    /// @custom:security OnlyOwner
    function foo() public {}
}"#;
    let docs = docs_for_function(text, "foo").expect("docs");

    assert_eq!(docs, "**Custom**\n- `security`: OnlyOwner");
}

#[test]
fn natspec_docs_renders_inheritdoc_tags() {
    let text = r#"contract Base {
    function ping() public {}
}
contract Derived is Base {
    /// @inheritdoc Base
    function foo() public {}
}"#;
    let docs = docs_for_function(text, "foo").expect("docs");

    assert_eq!(docs, "**Inheritdoc**\n- `Base`");
}

#[test]
fn natspec_docs_renders_block_comment_tags() {
    let text = r#"contract Foo {
    /**
     * @notice Adds two values.
     * @param left The left value.
     * @return sum The sum.
     */
    function add(uint256 left) public returns (uint256 sum) {
        return left + 1;
    }
}"#;
    let docs = docs_for_function(text, "add").expect("docs");

    assert!(docs.contains("**Notice**"));
    assert!(docs.contains("Adds two values."));
    assert!(docs.contains("**Parameters**"));
    assert!(docs.contains("- `left`: The left value."));
    assert!(docs.contains("**Returns**"));
    assert!(docs.contains("- `sum`: The sum."));
}

#[test]
fn natspec_docs_renders_block_comment_plain_text() {
    let text = r#"contract Foo {
    /**
     * Plain docs line one.
     * Plain docs line two.
     */
    function ping() public {}
}"#;
    let docs = docs_for_function(text, "ping").expect("docs");

    assert_eq!(docs, "Plain docs line one.\nPlain docs line two.");
}

#[test]
fn natspec_docs_preserves_markdown_in_dev_section() {
    let text = r#"contract Foo {
    /**
     * @dev Helper library packing and unpacking multiple values into bytes.
     *
     * Example usage:
     *
     * ```solidity
     * library MyPacker {
     *     function pack() public {}
     * }
     * ```
     *
     * _Available since v5.1._
     */
    function pack() public {}
}"#;
    let docs = docs_for_function(text, "pack").expect("docs");

    assert!(docs.contains("**Dev**"));
    assert!(docs.contains("Example usage:"));
    assert!(docs.contains("```solidity"));
    assert!(docs.contains("library MyPacker"));
    assert!(docs.contains("_Available since v5.1._"));
}

#[test]
fn natspec_docs_preserves_code_block_indentation() {
    let text = r#"contract Foo {
    /**
     * @dev Example:
     *
     * ```solidity
     * library MyPacker {
     *     function pack() public {}
     * }
     * ```
     */
    function pack() public {}
}"#;
    let docs = docs_for_function(text, "pack").expect("docs");

    let expected = r#"```solidity
library MyPacker {
    function pack() public {}
}
```"#;
    assert!(docs.contains(expected));
}

#[test]
fn natspec_docs_renders_code_block_inside_param_list() {
    let text = r#"contract Foo {
    /// @param data Encoded bytes:
    /// ```solidity
    /// bytes memory data = abi.encode(foo);
    /// ```
    function foo(bytes data) public {}
}"#;
    let docs = docs_for_function(text, "foo").expect("docs");

    let expected = r#"**Parameters**
- `data`: Encoded bytes:
  ```solidity
  bytes memory data = abi.encode(foo);
  ```"#;
    assert!(docs.contains(expected));
}

fn docs_for_function(text: &str, name: &str) -> Option<String> {
    let parse = sa_syntax::parse_file(text);
    let item = find_function_item(&parse, name);
    docs_for_item(&parse, item)
}

fn find_function_item<'a>(parse: &'a Parse, name: &str) -> &'a Item<'static> {
    parse
        .with_session(|| {
            parse.tree().items.iter().find_map(|item| match &item.kind {
                ItemKind::Function(function) => matches_name(function, name).then_some(item),
                ItemKind::Contract(contract) => contract.body.iter().find(|inner| {
                    if let ItemKind::Function(function) = &inner.kind {
                        matches_name(function, name)
                    } else {
                        false
                    }
                }),
                _ => None,
            })
        })
        .expect("function item")
}

fn matches_name(function: &ItemFunction<'_>, name: &str) -> bool {
    function
        .header
        .name
        .map(|ident| ident.to_string())
        .as_deref()
        == Some(name)
}
