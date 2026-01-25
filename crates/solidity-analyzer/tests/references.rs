use std::fs;

use sa_span::{
    TextRange, TextSize,
    lsp::{to_lsp_position, to_lsp_range},
};
use sa_test_support::{
    extract_offset, extract_offsets,
    lsp::{
        create_foundry_workspace, response_result, send_notification, send_request,
        setup_lsp_service,
    },
};
use tempfile::tempdir;
use tower_lsp::lsp_types::{
    Location, ReferenceContext, ReferenceParams, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, Url,
};

async fn references_for_single_file(
    text: &str,
    offset: TextSize,
) -> (tempfile::TempDir, Url, Vec<Location>) {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let main_path = root.join("src/Main.sol");
    fs::write(&main_path, text).expect("write main");

    let (mut service, _root_uri) = setup_lsp_service(&root, || {
        tower_lsp::LspService::new(solidity_analyzer::Server::new)
    })
    .await;

    let main_uri = Url::from_file_path(&main_path).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: text.to_string(),
            },
        },
    )
    .await;

    let position = to_lsp_position(offset, text);
    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: ReferenceContext {
            include_declaration: true,
        },
    };
    let response = send_request(&mut service, 2, "textDocument/references", params).await;
    let result = response_result::<Option<Vec<Location>>>(response).expect("references response");

    (temp, main_uri, result)
}

#[tokio::test]
async fn references_return_locations_across_files() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (main_text, offset) =
        extract_offset("import \"./Thing.sol\"; contract Main { /*caret*/Lib lib; }");
    let thing_text = "contract Lib {}".to_string();
    fs::write(root.join("src/Main.sol"), &main_text).expect("write main");
    fs::write(root.join("src/Thing.sol"), &thing_text).expect("write thing");

    let (mut service, _root_uri) = setup_lsp_service(&root, || {
        tower_lsp::LspService::new(solidity_analyzer::Server::new)
    })
    .await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let thing_uri = Url::from_file_path(root.join("src/Thing.sol")).expect("thing uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: main_text.clone(),
            },
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: thing_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: thing_text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(offset, &main_text);
    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: ReferenceContext {
            include_declaration: true,
        },
    };
    let response = send_request(&mut service, 2, "textDocument/references", params).await;
    let result = response_result::<Option<Vec<tower_lsp::lsp_types::Location>>>(response)
        .expect("references response");

    assert!(result.iter().any(|location| location.uri == main_uri));
    assert!(result.iter().any(|location| location.uri == thing_uri));
}

#[tokio::test]
async fn references_include_definition_site_and_modifier_usage() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    modifier guard(uint256 amount) {
        _;
    }

    function foo(uint256 val/*caret*/ue) public guard(value) {
        value;
    }
}
"#,
    );
    let (_temp, main_uri, result) = references_for_single_file(&text, offset).await;

    let positions = text
        .match_indices("value")
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    let expected_ranges = positions
        .iter()
        .map(|start| {
            let range = TextRange::new(
                TextSize::from(*start as u32),
                TextSize::from((*start + "value".len()) as u32),
            );
            to_lsp_range(range, &text)
        })
        .collect::<Vec<_>>();

    for expected in expected_ranges {
        assert!(
            result
                .iter()
                .any(|location| location.uri == main_uri && location.range == expected)
        );
    }
}

#[tokio::test]
async fn references_handle_struct_constructor_member_access() {
    let (text, offset) = extract_offset(
        r#"
contract Main {
    struct Data {
        uint256 value;
    }

    uint256 count;

    function foo() public {
        Data(1).value;
        count = 1;
        co/*caret*/unt;
    }
}
"#,
    );
    let (_temp, main_uri, result) = references_for_single_file(&text, offset).await;

    let positions = text
        .match_indices("count")
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    let expected_ranges = positions
        .iter()
        .map(|start| {
            let range = TextRange::new(
                TextSize::from(*start as u32),
                TextSize::from((*start + "count".len()) as u32),
            );
            to_lsp_range(range, &text)
        })
        .collect::<Vec<_>>();

    for expected in expected_ranges {
        assert!(
            result
                .iter()
                .any(|location| location.uri == main_uri && location.range == expected)
        );
    }
}

#[tokio::test]
async fn references_resolve_inherited_state_variable() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

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
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) = setup_lsp_service(&root, || {
        tower_lsp::LspService::new(solidity_analyzer::Server::new)
    })
    .await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(caret_offset, &text);
    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: ReferenceContext {
            include_declaration: true,
        },
    };
    let response = send_request(&mut service, 2, "textDocument/references", params).await;
    let result = response_result::<Option<Vec<tower_lsp::lsp_types::Location>>>(response)
        .expect("references response");

    let len = TextSize::from("value".len() as u32);
    let expected = vec![
        to_lsp_range(TextRange::at(def_offset, len), &text),
        to_lsp_range(TextRange::at(caret_offset, len), &text),
    ];

    let mut ranges = result
        .iter()
        .filter(|location| location.uri == main_uri)
        .map(|location| location.range)
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| {
        (
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character,
        )
    });

    let mut expected_ranges = expected;
    expected_ranges.sort_by_key(|range| {
        (
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character,
        )
    });

    assert_eq!(ranges, expected_ranges);
}

#[tokio::test]
async fn references_resolve_overridden_function_in_multiple_inheritance() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offsets) = extract_offsets(
        r#"
contract A {
    function foo() public virtual {}
}

contract B is A {
    function /*def*/foo() public virtual override {}
}

contract C is A {
    function foo() public virtual override {}
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
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) = setup_lsp_service(&root, || {
        tower_lsp::LspService::new(solidity_analyzer::Server::new)
    })
    .await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(caret_offset, &text);
    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: ReferenceContext {
            include_declaration: true,
        },
    };
    let response = send_request(&mut service, 2, "textDocument/references", params).await;
    let result = response_result::<Option<Vec<tower_lsp::lsp_types::Location>>>(response)
        .expect("references response");

    let len = TextSize::from("foo".len() as u32);
    let expected = vec![
        to_lsp_range(TextRange::at(def_offset, len), &text),
        to_lsp_range(TextRange::at(caret_offset, len), &text),
    ];

    let mut ranges = result
        .iter()
        .filter(|location| location.uri == main_uri)
        .map(|location| location.range)
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| {
        (
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character,
        )
    });

    let mut expected_ranges = expected;
    expected_ranges.sort_by_key(|range| {
        (
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character,
        )
    });

    assert_eq!(ranges, expected_ranges);
}

#[tokio::test]
async fn references_resolve_overloaded_function_calls() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

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
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) = setup_lsp_service(&root, || {
        tower_lsp::LspService::new(solidity_analyzer::Server::new)
    })
    .await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: main_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(caret_offset, &text);
    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: ReferenceContext {
            include_declaration: true,
        },
    };
    let response = send_request(&mut service, 2, "textDocument/references", params).await;
    let result = response_result::<Option<Vec<tower_lsp::lsp_types::Location>>>(response)
        .expect("references response");

    let len = TextSize::from("foo".len() as u32);
    let expected = vec![
        to_lsp_range(TextRange::at(def_offset, len), &text),
        to_lsp_range(TextRange::at(caret_offset, len), &text),
    ];

    let mut ranges = result
        .iter()
        .filter(|location| location.uri == main_uri)
        .map(|location| location.range)
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| {
        (
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character,
        )
    });

    let mut expected_ranges = expected;
    expected_ranges.sort_by_key(|range| {
        (
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character,
        )
    });

    assert_eq!(ranges, expected_ranges);
}
