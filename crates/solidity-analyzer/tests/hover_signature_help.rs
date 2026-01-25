use std::fs;

use sa_span::lsp::{to_lsp_position, to_lsp_range};
use sa_test_support::{
    extract_offset, extract_offsets, find_range,
    lsp::{
        create_foundry_workspace, response_result, send_notification, send_request,
        setup_lsp_service,
    },
};
use tempfile::tempdir;
use tower_lsp::lsp_types::{
    Documentation, Hover, HoverContents, HoverParams, MarkupContent, MarkupKind, SignatureHelp,
    SignatureHelpParams, TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Url,
};

#[tokio::test]
async fn hover_returns_markup_content() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offset) = extract_offset(
        "/// Greeter docs\ncontract Greeter {\n    function greet() public {}\n}\ncontract Main { /*caret*/Greeter g; }",
    );
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

    let position = to_lsp_position(offset, &text);
    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/hover", params).await;
    let result = response_result::<Option<Hover>>(response).expect("hover result");
    let MarkupContent { kind, value } = match result.contents {
        tower_lsp::lsp_types::HoverContents::Markup(markup) => markup,
        other => panic!("unexpected hover contents: {other:?}"),
    };
    assert_eq!(kind, MarkupKind::Markdown);
    assert!(value.contains("```solidity\ncontract Greeter\n```"));
    assert!(value.contains("Greeter docs"));
}

#[tokio::test]
async fn hover_range_matches_reference_token() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let other_text = r#"
// padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding
// padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding padding
contract Foo {}
"#;
    fs::write(root.join("src/Other.sol"), other_text).expect("write other");

    let (text, offset) = extract_offset(
        r#"
import "./Other.sol";

contract Main {
    function run() public {
        Fo/*caret*/o value;
    }
}
"#,
    );
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

    let position = to_lsp_position(offset, &text);
    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
    };
    let response = send_request(&mut service, 6, "textDocument/hover", params).await;
    let hover = response_result::<Option<Hover>>(response).expect("hover result");
    let expected_range = to_lsp_range(find_range(&text, "Foo"), &text);
    assert_eq!(hover.range, Some(expected_range));
}

#[tokio::test]
async fn signature_help_returns_active_parameter() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offset) = extract_offset(
        "contract Math {\n    function add(uint256 left, uint256 right) public {}\n    function test() public { add(1, /*caret*/2); }\n}",
    );
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

    let position = to_lsp_position(offset, &text);
    let params = SignatureHelpParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        context: None,
    };
    let response = send_request(&mut service, 3, "textDocument/signatureHelp", params).await;
    let result = response_result::<Option<SignatureHelp>>(response).expect("signature help result");

    assert_eq!(result.active_parameter, Some(1));
    let signature = result.signatures.first().expect("signature");
    assert!(
        signature
            .label
            .contains("function add(uint256 left, uint256 right)")
    );
}

#[tokio::test]
async fn hover_and_signature_help_render_natspec_sections() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offsets) = extract_offsets(
        r#"contract Math {
    /// @notice Adds two values.
    /// @param left The left value.
    /// @param right The right value.
    /// @return sum The sum.
    function add(uint256 left, uint256 right) public returns (uint256 sum) { return left + right; }
}
contract Main { function run() public { Math math = new Math(); math./*hover*/add(1, /*caret*/2); } }"#,
        &["/*hover*/", "/*caret*/"],
    );
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

    let hover_offset = offsets[0];
    let signature_offset = offsets[1];
    let hover_position = to_lsp_position(hover_offset, &text);
    let signature_position = to_lsp_position(signature_offset, &text);
    let hover_params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position: hover_position,
        },
        work_done_progress_params: Default::default(),
    };
    let hover_response = send_request(&mut service, 4, "textDocument/hover", hover_params).await;
    let hover_result = response_result::<Option<Hover>>(hover_response).expect("hover value");
    let HoverContents::Markup(MarkupContent { kind, value }) = hover_result.contents else {
        panic!("unexpected hover contents");
    };
    assert_eq!(kind, MarkupKind::Markdown);
    assert!(value.contains("```solidity\nfunction add("));
    assert!(value.contains("**Parameters**"));
    assert!(value.contains("- `left`: The left value."));
    assert!(value.contains("- `right`: The right value."));
    assert!(value.contains("**Returns**"));
    assert!(value.contains("- `sum`: The sum."));

    let signature_params = SignatureHelpParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position: signature_position,
        },
        work_done_progress_params: Default::default(),
        context: None,
    };
    let signature_response = send_request(
        &mut service,
        5,
        "textDocument/signatureHelp",
        signature_params,
    )
    .await;
    let signature_result =
        response_result::<Option<SignatureHelp>>(signature_response).expect("signature value");
    let signature = signature_result.signatures.first().expect("signature");
    let Documentation::MarkupContent(MarkupContent { kind, value }) =
        signature.documentation.as_ref().expect("signature docs")
    else {
        panic!("expected markup docs");
    };
    assert_eq!(*kind, MarkupKind::Markdown);
    assert!(value.contains("**Parameters**"));
    assert!(value.contains("- `left`: The left value."));
    assert!(value.contains("- `right`: The right value."));
    assert!(value.contains("**Returns**"));
    assert!(value.contains("- `sum`: The sum."));
}

#[tokio::test]
async fn hover_and_signature_help_render_inheritdoc_for_dependency_layouts() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let foundry_toml = r#"
[profile.default]
remappings = ["@openzeppelin/=lib/openzeppelin-contracts/"]
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let contracts_dir = root.join("lib/openzeppelin-contracts/contracts/token/ERC20/extensions");
    fs::create_dir_all(&contracts_dir).expect("create contracts dir");

    let interface_text = r#"interface IERC20Permit {
    /// @notice Approve spending by signature.
    /// @param owner The owner.
    /// @param spender The spender.
    function permit(address owner, address spender) external;
}"#;
    fs::write(contracts_dir.join("IERC20Permit.sol"), interface_text).expect("write IERC20Permit");

    let contract_text = r#"import "@openzeppelin/contracts/token/ERC20/extensions/IERC20Permit.sol";

contract ERC20Permit is IERC20Permit {
    /// @inheritdoc IERC20Permit
    function permit(address owner, address spender) external override {}
}"#;
    fs::write(contracts_dir.join("ERC20Permit.sol"), contract_text).expect("write ERC20Permit");

    let (text, offsets) = extract_offsets(
        r#"import "@openzeppelin/contracts/token/ERC20/extensions/ERC20Permit.sol";

contract Main {
    function run(ERC20Permit token) public {
        token./*hover*/permit(address(0), /*caret*/address(0));
    }
}"#,
        &["/*hover*/", "/*caret*/"],
    );
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

    let hover_position = to_lsp_position(offsets[0], &text);
    let hover_params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position: hover_position,
        },
        work_done_progress_params: Default::default(),
    };
    let hover_response = send_request(&mut service, 6, "textDocument/hover", hover_params).await;
    let hover_result = response_result::<Option<Hover>>(hover_response).expect("hover value");
    let HoverContents::Markup(MarkupContent { kind, value }) = hover_result.contents else {
        panic!("unexpected hover contents");
    };
    assert_eq!(kind, MarkupKind::Markdown);
    assert!(value.contains("Approve spending by signature."));
    assert!(value.contains("**Parameters**"));
    assert!(value.contains("- `owner`: The owner."));
    assert!(value.contains("- `spender`: The spender."));

    let signature_position = to_lsp_position(offsets[1], &text);
    let signature_params = SignatureHelpParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position: signature_position,
        },
        work_done_progress_params: Default::default(),
        context: None,
    };
    let signature_response = send_request(
        &mut service,
        7,
        "textDocument/signatureHelp",
        signature_params,
    )
    .await;
    let signature_result =
        response_result::<Option<SignatureHelp>>(signature_response).expect("signature value");
    let signature = signature_result.signatures.first().expect("signature");
    let Documentation::MarkupContent(MarkupContent { kind, value }) =
        signature.documentation.as_ref().expect("signature docs")
    else {
        panic!("expected markup docs");
    };
    assert_eq!(*kind, MarkupKind::Markdown);
    assert!(value.contains("Approve spending by signature."));
    assert!(value.contains("**Parameters**"));
    assert!(value.contains("- `owner`: The owner."));
    assert!(value.contains("- `spender`: The spender."));
}

#[tokio::test]
async fn hover_renders_natspec_links() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offsets) = extract_offsets(
        r#"contract Governor {
    /// @notice Uses {quorum}, {Governor.quorum}, and {Governor-quorum}.
    function quorum(uint256 timepoint) public view returns (uint256) { return timepoint; }
}
contract Main { function run() public { Governor gov = new Governor(); gov./*hover*/quorum(/*caret*/1); } }"#,
        &["/*hover*/", "/*caret*/"],
    );
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

    let hover_offset = offsets[0];
    let hover_position = to_lsp_position(hover_offset, &text);
    let hover_params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position: hover_position,
        },
        work_done_progress_params: Default::default(),
    };
    let hover_response = send_request(&mut service, 7, "textDocument/hover", hover_params).await;
    let hover_result = response_result::<Option<Hover>>(hover_response).expect("hover value");
    let HoverContents::Markup(MarkupContent { kind, value }) = hover_result.contents else {
        panic!("unexpected hover contents");
    };
    assert_eq!(kind, MarkupKind::Markdown);
    assert!(value.contains("[`{quorum}`](file://"));
    assert!(value.contains("[`{Governor.quorum}`](file://"));
    assert!(value.contains("[`{Governor-quorum}`](file://"));
}

#[tokio::test]
async fn hover_and_signature_help_preserve_natspec_markdown() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offsets) = extract_offsets(
        r#"contract Math {
    /**
     * @dev Example usage:
     *
     * ```solidity
     *     function demo() public {}
     * ```
     *
     * _Available since v5.1._
     */
    function add(uint256 left) public {}
}
contract Main { function run() public { Math math = new Math(); math./*hover*/add(/*caret*/1); } }"#,
        &["/*hover*/", "/*caret*/"],
    );
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

    let hover_offset = offsets[0];
    let signature_offset = offsets[1];
    let hover_position = to_lsp_position(hover_offset, &text);
    let signature_position = to_lsp_position(signature_offset, &text);
    let hover_params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position: hover_position,
        },
        work_done_progress_params: Default::default(),
    };
    let hover_response = send_request(&mut service, 6, "textDocument/hover", hover_params).await;
    let hover_result = response_result::<Option<Hover>>(hover_response).expect("hover value");
    let HoverContents::Markup(MarkupContent { kind, value }) = hover_result.contents else {
        panic!("unexpected hover contents");
    };
    assert_eq!(kind, MarkupKind::Markdown);
    assert!(value.contains("```solidity"));
    assert!(value.contains("    function demo() public {}"));
    assert!(value.contains("_Available since v5.1._"));

    let signature_params = SignatureHelpParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position: signature_position,
        },
        work_done_progress_params: Default::default(),
        context: None,
    };
    let signature_response = send_request(
        &mut service,
        7,
        "textDocument/signatureHelp",
        signature_params,
    )
    .await;
    let signature_result =
        response_result::<Option<SignatureHelp>>(signature_response).expect("signature value");
    let signature = signature_result.signatures.first().expect("signature");
    let Documentation::MarkupContent(MarkupContent { kind, value }) =
        signature.documentation.as_ref().expect("signature docs")
    else {
        panic!("expected markup docs");
    };
    assert_eq!(*kind, MarkupKind::Markdown);
    assert!(value.contains("```solidity"));
    assert!(value.contains("    function demo() public {}"));
    assert!(value.contains("_Available since v5.1._"));
}
