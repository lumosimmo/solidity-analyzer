use sa_span::TextRange;
use sa_syntax::{ast_utils, parse_file, tokens};

fn slice_range(text: &str, range: TextRange) -> &str {
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    &text[start..end]
}

#[test]
fn contract_name_fixtures() {
    let text = r#"contract Alpha {}
library Beta {}
interface Gamma {}"#;
    let expected = "Alpha\nBeta\nGamma";
    let parse = parse_file(text);
    let names = ast_utils::contract_names(&parse).join("\n");
    assert_eq!(names, expected);
}

#[test]
fn comment_fixtures() {
    let text = r#"// line comment
/// doc line
/* block comment */
/** doc block */
contract Foo {
    // inside
}"#;
    let expected = "line:false:line comment\nline:true:doc line\nblock:false:block comment\nblock:true:doc block\nline:false:inside";
    let comments = tokens::collect_comments(text)
        .into_iter()
        .map(|comment| {
            format!(
                "{}:{}:{}",
                comment.kind_label(),
                comment.is_doc,
                comment.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(comments, expected);
}

#[test]
fn function_signature_fixtures() {
    let text = r#"contract Foo {
    function first(uint256 a) external {}
    function second() public {}
    fallback() external {}
    constructor(uint256 a) {}
}

function topLevel() {}"#;
    let expected =
        "function first(1)\nfunction second(0)\nfallback(0)\nconstructor(1)\nfunction topLevel(0)";
    let parse = parse_file(text);
    let signatures = ast_utils::function_signatures(&parse).join("\n");
    assert_eq!(signatures, expected);
}

#[test]
fn collect_qualified_matches_multi_level() {
    let text = r#"
        prefix Libs.Foo.Bar();
        Libs.Bar();
    "#;
    let ranges = tokens::collect_qualified_ident_ranges(text, "Libs.Foo", "Bar");
    assert_eq!(ranges.len(), 1);
    let range = &ranges[0];
    assert_eq!(
        usize::from(range.qualifier_start),
        text.find("Libs").unwrap()
    );
    assert_eq!(slice_range(text, range.range), "Bar");
}
