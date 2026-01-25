use forge_fmt::FormatterConfig;
use sa_paths::NormalizedPath;
use sa_span::{TextRange, TextSize};
use sa_test_support::setup_analysis;

#[test]
fn format_document_basic_contract() {
    let text = r#"
contract Foo{function bar()public returns(uint256){return 1;}}
"#
    .trim();
    let expected = r#"
contract Foo {
    function bar() public returns (uint256) {
        return 1;
    }
}
"#
    .trim_start();

    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let edit = analysis
        .format_document(file_id, &FormatterConfig::default())
        .expect("format edit");

    assert_eq!(edit.new_text, expected);
    assert_eq!(
        edit.range,
        TextRange::new(TextSize::from(0), TextSize::from(text.len() as u32))
    );
}

#[test]
fn format_document_handles_multiline_items() {
    let text = r#"
contract Foo{
function add(uint256 a,uint256 b)public returns(uint256){
return a+b;
}
}
"#
    .trim();
    let expected = r#"
contract Foo {
    function add(uint256 a, uint256 b) public returns (uint256) {
        return a + b;
    }
}
"#
    .trim_start();

    let path = NormalizedPath::new("/workspace/src/Main.sol");
    let (analysis, snapshot) = setup_analysis(vec![(path.clone(), text.to_string())], vec![]);
    let file_id = snapshot.file_id(&path).expect("file id");

    let edit = analysis
        .format_document(file_id, &FormatterConfig::default())
        .expect("format edit");

    assert_eq!(edit.new_text, expected);
}
