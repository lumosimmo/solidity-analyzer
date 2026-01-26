use std::fs;
use std::path::Path;

use sa_span::{TextRange, TextSize, lsp::to_lsp_position, lsp::to_lsp_range};
use sa_test_support::{
    extract_offset, extract_offsets, find_range,
    lsp::{
        create_foundry_workspace, make_server, response_result, send_notification, send_request,
        setup_lsp_service,
    },
};
use tempfile::tempdir;
use tower_lsp::lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, Url,
};

#[tokio::test]
async fn goto_definition_resolves_across_files() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (dep_text, dep_offset) = extract_offset(
        "contract /*caret*/Dep { function value() public pure returns (uint) { return 1; } }",
    );
    let (main_text, main_offset) = extract_offset(
        "import \"./Dep.sol\";\ncontract Main { function foo() public { /*caret*/Dep dep = new Dep(); } }",
    );
    fs::write(root.join("src/Dep.sol"), &dep_text).expect("write dep");
    fs::write(root.join("src/Main.sol"), &main_text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let dep_uri = Url::from_file_path(root.join("src/Dep.sol")).expect("dep uri");
    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let open_dep = TextDocumentItem {
        uri: dep_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: dep_text.clone(),
    };
    let open_main = TextDocumentItem {
        uri: main_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: main_text.clone(),
    };
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_dep,
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_main,
        },
    )
    .await;

    let position = to_lsp_position(main_offset, &main_text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let expected_range = TextRange::at(dep_offset, TextSize::from(3));
    let expected_range = to_lsp_range(expected_range, &dep_text);
    assert_eq!(location.uri, dep_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_reexported_import() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let base_text = r#"
contract Base {}
"#;
    let intermediate_text = r#"
import {Base} from "./Base.sol";

contract Intermediate is Base {}
"#;
    let (main_text, main_offset) = extract_offset(
        r#"
import {Intermediate, Base} from "./Intermediate.sol";

contract Main is Intermediate {
    Ba/*caret*/se value;
}
"#,
    );

    let base_uri = write_source(&root, "src/Base.sol", base_text);
    write_source(&root, "src/Intermediate.sol", intermediate_text);
    let main_uri = write_source(&root, "src/Main.sol", &main_text);

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;
    open_document(&mut service, main_uri.clone(), main_text.clone()).await;

    let location =
        request_goto_definition(&mut service, main_uri, &main_text, main_offset, 40).await;
    let expected_range = to_lsp_range(find_range(base_text, "Base"), base_text);
    assert_eq!(location.uri, base_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_import_path() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let dep_text = "contract Dep {}";
    let (main_text, main_offset) =
        extract_offset("import \"./De/*caret*/p.sol\";\ncontract Main {}");
    fs::write(root.join("src/Dep.sol"), dep_text).expect("write dep");
    fs::write(root.join("src/Main.sol"), &main_text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let open_main = TextDocumentItem {
        uri: main_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: main_text.clone(),
    };
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_main,
        },
    )
    .await;

    let position = to_lsp_position(main_offset, &main_text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let dep_uri = Url::from_file_path(root.join("src/Dep.sol")).expect("dep uri");
    let expected_range = to_lsp_range(
        TextRange::new(TextSize::from(0), TextSize::from(0)),
        dep_text,
    );
    assert_eq!(location.uri, dep_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_without_opening_dependency() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (dep_text, dep_offset) = extract_offset(
        r#"
contract /*caret*/Dep {
    function value() public pure returns (uint) {
        return 1;
    }
}
"#,
    );
    let (main_text, main_offset) = extract_offset(
        r#"
import "./Dep.sol";

contract Main {
    function foo() public {
        /*caret*/Dep dep = new Dep();
    }
}
"#,
    );
    fs::write(root.join("src/Dep.sol"), &dep_text).expect("write dep");
    fs::write(root.join("src/Main.sol"), &main_text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let open_main = TextDocumentItem {
        uri: main_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: main_text.clone(),
    };
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_main,
        },
    )
    .await;

    let position = to_lsp_position(main_offset, &main_text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let dep_uri = Url::from_file_path(root.join("src/Dep.sol")).expect("dep uri");
    let expected_range = TextRange::at(dep_offset, TextSize::from(3));
    let expected_range = to_lsp_range(expected_range, &dep_text);
    assert_eq!(location.uri, dep_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_after_closing_dependency() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (dep_text, dep_offset) = extract_offset(
        r#"
contract /*caret*/Dep {
    function value() public pure returns (uint) {
        return 1;
    }
}
        "#,
    );
    let (main_text, main_offset) = extract_offset(
        r#"
import "./Dep.sol";

contract Main {
    function foo() public {
        /*caret*/Dep dep = new Dep();
    }
}
"#,
    );
    fs::write(root.join("src/Dep.sol"), &dep_text).expect("write dep");
    fs::write(root.join("src/Main.sol"), &main_text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let dep_uri = Url::from_file_path(root.join("src/Dep.sol")).expect("dep uri");
    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let open_dep = TextDocumentItem {
        uri: dep_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: dep_text.clone(),
    };
    let open_main = TextDocumentItem {
        uri: main_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: main_text.clone(),
    };
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_dep,
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_main,
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didClose",
        tower_lsp::lsp_types::DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: dep_uri.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(main_offset, &main_text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let expected_range = TextRange::at(dep_offset, TextSize::from(3));
    let expected_range = to_lsp_range(expected_range, &dep_text);
    assert_eq!(location.uri, dep_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_inherited_function() {
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

contract C is A {}

contract D is B, C {
    function bar() public {
        fo/*caret*/o();
    }
}
"#,
        &["/*def*/", "/*caret*/"],
    );
    let def_offset = offsets[0];
    let call_offset = offsets[1];
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let open_main = TextDocumentItem {
        uri: main_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: text.clone(),
    };
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_main,
        },
    )
    .await;

    let position = to_lsp_position(call_offset, &text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 6, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let expected_range = TextRange::at(def_offset, TextSize::from(3));
    let expected_range = to_lsp_range(expected_range, &text);
    assert_eq!(location.uri, main_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_super_call() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offsets) = extract_offsets(
        r#"
contract A {
    function foo() public virtual {}
}

contract B is A {
    function foo() public virtual override {}
}

contract C is A {
    function /*def*/foo() public virtual override {}
}

contract D is B, C {
    function foo() public override(B, C) {
        super.fo/*caret*/o();
    }
}
"#,
        &["/*def*/", "/*caret*/"],
    );
    let def_offset = offsets[0];
    let call_offset = offsets[1];
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let open_main = TextDocumentItem {
        uri: main_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: text.clone(),
    };
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_main,
        },
    )
    .await;

    let position = to_lsp_position(call_offset, &text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 7, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let expected_range = TextRange::at(def_offset, TextSize::from(3));
    let expected_range = to_lsp_range(expected_range, &text);
    assert_eq!(location.uri, main_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_super_receiver_and_member() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offsets) = extract_offsets(
        r#"
contract Base {
    function /*def*/supportsInterface(bytes4 interfaceId) public view virtual returns (bool) {
        return true;
    }
}

contract Derived is Base {
    function supportsInterface(bytes4 interfaceId) public view override returns (bool) {
        return /*super*/super./*method*/supportsInterface(interfaceId);
    }
}
"#,
        &["/*def*/", "/*super*/", "/*method*/"],
    );
    let def_offset = offsets[0];
    let super_offset = offsets[1];
    let method_offset = offsets[2];
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    open_document(&mut service, main_uri.clone(), text.clone()).await;

    let expected_range = TextRange::at(def_offset, TextSize::from(17));
    let expected_range = to_lsp_range(expected_range, &text);

    let super_location =
        request_goto_definition(&mut service, main_uri.clone(), &text, super_offset, 8).await;
    assert_eq!(super_location.uri, main_uri);
    assert_eq!(super_location.range, expected_range);

    let method_location =
        request_goto_definition(&mut service, main_uri.clone(), &text, method_offset, 9).await;
    assert_eq!(method_location.uri, main_uri);
    assert_eq!(method_location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_for_opened_lib_files() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (helper_text, helper_offset) = extract_offset(
        r#"
contract /*caret*/Helper {}
"#,
    );
    let (uses_text, uses_offset) = extract_offset(
        r#"
import "./Helper.sol";

contract UsesHelper {
    /*caret*/Helper helper;
}
"#,
    );
    fs::write(root.join("lib/Helper.sol"), &helper_text).expect("write helper");
    fs::write(root.join("lib/UsesHelper.sol"), &uses_text).expect("write uses helper");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let helper_uri = Url::from_file_path(root.join("lib/Helper.sol")).expect("helper uri");
    let uses_uri = Url::from_file_path(root.join("lib/UsesHelper.sol")).expect("uses uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: helper_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: helper_text.clone(),
            },
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uses_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: uses_text.clone(),
            },
        },
    )
    .await;

    let position = to_lsp_position(uses_offset, &uses_text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: uses_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let expected_range = TextRange::at(helper_offset, TextSize::from(6));
    let expected_range = to_lsp_range(expected_range, &helper_text);
    assert_eq!(location.uri, helper_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_returns_none_for_unresolved_identifiers() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (main_text, main_offset) =
        extract_offset("contract Main { function foo() public { /*caret*/Unknown dep; } }");
    fs::write(root.join("src/Main.sol"), &main_text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
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

    let position = to_lsp_position(main_offset, &main_text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    assert!(result.is_none());
}

#[tokio::test]
async fn goto_definition_resolves_local_binding() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo(uint256 value) public {
        val/*caret*/ue;
    }
}
"#,
    );
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let open_main = TextDocumentItem {
        uri: main_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: text.clone(),
    };
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_main,
        },
    )
    .await;

    let position = to_lsp_position(offset, &text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let expected = find_range(&text, "value");
    let expected = to_lsp_range(expected, &text);
    assert_eq!(location.uri, main_uri);
    assert_eq!(location.range, expected);
}

#[tokio::test]
async fn goto_definition_resolves_local_definition_site() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offset) = extract_offset(
        r#"
contract Main {
    function foo(uint256 val/*caret*/ue) public {
        value;
    }
}
"#,
    );
    fs::write(root.join("src/Main.sol"), &text).expect("write main");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let main_uri = Url::from_file_path(root.join("src/Main.sol")).expect("main uri");
    let open_main = TextDocumentItem {
        uri: main_uri.clone(),
        language_id: "solidity".to_string(),
        version: 1,
        text: text.clone(),
    };
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_main,
        },
    )
    .await;

    let position = to_lsp_position(offset, &text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: main_uri.clone(),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(&mut service, 2, "textDocument/definition", params).await;
    let result = response_result::<Option<GotoDefinitionResponse>>(response);
    let location = match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    };

    let expected = find_range(&text, "value");
    let expected = to_lsp_range(expected, &text);
    assert_eq!(location.uri, main_uri);
    assert_eq!(location.range, expected);
}

#[tokio::test]
async fn goto_definition_resolves_inherited_member_without_opening_base() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (parent_text, parent_offset) = extract_offset(
        r#"
contract Parent {
    function /*caret*/value() public pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    let (child_text, child_offset) = extract_offset(
        r#"
import "./Parent.sol";

contract Child is Parent {
    function foo() public pure returns (uint256) {
        return val/*caret*/ue();
    }
}
"#,
    );

    fs::write(root.join("src/Parent.sol"), &parent_text).expect("write parent");
    fs::write(root.join("src/Child.sol"), &child_text).expect("write child");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let child_uri = Url::from_file_path(root.join("src/Child.sol")).expect("child uri");
    let parent_uri = Url::from_file_path(root.join("src/Parent.sol")).expect("parent uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: child_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: child_text.clone(),
            },
        },
    )
    .await;

    let location =
        request_goto_definition(&mut service, child_uri, &child_text, child_offset, 30).await;
    let expected_range = TextRange::at(parent_offset, TextSize::from(5));
    let expected_range = to_lsp_range(expected_range, &parent_text);
    assert_eq!(location.uri, parent_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_inherited_member_after_closing_base() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (parent_text, parent_offset) = extract_offset(
        r#"
contract Parent {
    function /*caret*/value() public pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    let (child_text, child_offset) = extract_offset(
        r#"
import "./Parent.sol";

contract Child is Parent {
    function foo() public pure returns (uint256) {
        return val/*caret*/ue();
    }
}
"#,
    );

    fs::write(root.join("src/Parent.sol"), &parent_text).expect("write parent");
    fs::write(root.join("src/Child.sol"), &child_text).expect("write child");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let child_uri = Url::from_file_path(root.join("src/Child.sol")).expect("child uri");
    let parent_uri = Url::from_file_path(root.join("src/Parent.sol")).expect("parent uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: parent_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: parent_text.clone(),
            },
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: child_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: child_text.clone(),
            },
        },
    )
    .await;
    send_notification(
        &mut service,
        "textDocument/didClose",
        tower_lsp::lsp_types::DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: parent_uri.clone(),
            },
        },
    )
    .await;

    let location =
        request_goto_definition(&mut service, child_uri, &child_text, child_offset, 31).await;
    let expected_range = TextRange::at(parent_offset, TextSize::from(5));
    let expected_range = to_lsp_range(expected_range, &parent_text);
    assert_eq!(location.uri, parent_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_interface_member_calls() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (interface_text, interface_offset) = extract_offset(
        r#"
interface IStuff {
    function /*caret*/doStuff() external view returns (uint256);
}
"#,
    );
    let (child_text, offsets) = extract_offsets(
        r#"
import "./IStuff.sol";

contract Child {
    function useIStuff(IStuff stuff) external view returns (uint256) {
        return stuff.doS/*call1*/tuff();
    }

    function useIStuff2(address stuffAddress) external view returns (uint256) {
        IStuff stuff = IStuff(stuffAddress);
        return stuff.doS/*call2*/tuff();
    }
}
"#,
        &["/*call1*/", "/*call2*/"],
    );

    fs::write(root.join("src/IStuff.sol"), &interface_text).expect("write interface");
    fs::write(root.join("src/Child.sol"), &child_text).expect("write child");

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let child_uri = Url::from_file_path(root.join("src/Child.sol")).expect("child uri");
    let interface_uri = Url::from_file_path(root.join("src/IStuff.sol")).expect("interface uri");
    open_document(&mut service, child_uri.clone(), child_text.clone()).await;

    let expected_range = TextRange::at(interface_offset, TextSize::from(7));
    let expected_range = to_lsp_range(expected_range, &interface_text);

    for (idx, offset) in offsets.into_iter().enumerate() {
        let location = request_goto_definition(
            &mut service,
            child_uri.clone(),
            &child_text,
            offset,
            33 + idx as i64,
        )
        .await;
        assert_eq!(location.uri, interface_uri);
        assert_eq!(location.range, expected_range);
    }
}

#[tokio::test]
async fn goto_definition_resolves_cast_receiver_member_call() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (interface_text, interface_offset) = extract_offset(
        r#"
interface IStuff {
    function /*caret*/doStuff() external view returns (uint256);
}
"#,
    );
    let (child_text, child_offset) = extract_offset(
        r#"
import "./IStuff.sol";

contract Child {
    function use(address stuffAddress) external view returns (uint256) {
        return IStuff(stuffAddress).doS/*caret*/tuff();
    }
}
"#,
    );

    let interface_uri = write_source(&root, "src/IStuff.sol", &interface_text);
    let child_uri = write_source(&root, "src/Child.sol", &child_text);

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;
    open_document(&mut service, child_uri.clone(), child_text.clone()).await;

    let location =
        request_goto_definition(&mut service, child_uri, &child_text, child_offset, 35).await;
    let expected_range = TextRange::at(interface_offset, TextSize::from(7));
    let expected_range = to_lsp_range(expected_range, &interface_text);
    assert_eq!(location.uri, interface_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_call_receiver_member_call() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (interface_text, interface_offset) = extract_offset(
        r#"
interface IStuff {
    function /*caret*/doStuff() external view returns (uint256);
}
"#,
    );
    let (child_text, child_offset) = extract_offset(
        r#"
import "./IStuff.sol";

contract Child {
    function getStuff(address stuffAddress) internal pure returns (IStuff) {
        return IStuff(stuffAddress);
    }

    function use(address stuffAddress) external view returns (uint256) {
        return getStuff(stuffAddress).doS/*caret*/tuff();
    }
}
"#,
    );

    let interface_uri = write_source(&root, "src/IStuff.sol", &interface_text);
    let child_uri = write_source(&root, "src/Child.sol", &child_text);

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;
    open_document(&mut service, child_uri.clone(), child_text.clone()).await;

    let location =
        request_goto_definition(&mut service, child_uri, &child_text, child_offset, 36).await;
    let expected_range = TextRange::at(interface_offset, TextSize::from(7));
    let expected_range = to_lsp_range(expected_range, &interface_text);
    assert_eq!(location.uri, interface_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_index_receiver_member_call() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (interface_text, interface_offset) = extract_offset(
        r#"
interface IStuff {
    function /*caret*/doStuff() external view returns (uint256);
}
"#,
    );
    let (child_text, child_offset) = extract_offset(
        r#"
import "./IStuff.sol";

contract Child {
    IStuff[] private stuffs;

    function use(uint256 index) external view returns (uint256) {
        return stuffs[index].doS/*caret*/tuff();
    }
}
"#,
    );

    let interface_uri = write_source(&root, "src/IStuff.sol", &interface_text);
    let child_uri = write_source(&root, "src/Child.sol", &child_text);

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;
    open_document(&mut service, child_uri.clone(), child_text.clone()).await;

    let location =
        request_goto_definition(&mut service, child_uri, &child_text, child_offset, 37).await;
    let expected_range = TextRange::at(interface_offset, TextSize::from(7));
    let expected_range = to_lsp_range(expected_range, &interface_text);
    assert_eq!(location.uri, interface_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_library_member_call() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (lib_text, lib_offset) = extract_offset(
        r#"
library Math {
    function /*caret*/add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
"#,
    );
    let (main_text, main_offset) = extract_offset(
        r#"
import "./Math.sol";

contract Calculator {
    function sum(uint256 a, uint256 b) external pure returns (uint256) {
        return Math.ad/*caret*/d(a, b);
    }
}
"#,
    );

    let lib_uri = write_source(&root, "src/Math.sol", &lib_text);
    let main_uri = write_source(&root, "src/Main.sol", &main_text);

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;
    open_document(&mut service, main_uri.clone(), main_text.clone()).await;

    let location =
        request_goto_definition(&mut service, main_uri, &main_text, main_offset, 38).await;
    let expected_range = TextRange::at(lib_offset, TextSize::from(3));
    let expected_range = to_lsp_range(expected_range, &lib_text);
    assert_eq!(location.uri, lib_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_contract_type_member_selector() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (foo_text, foo_offset) = extract_offset(
        r#"
contract Foo {
    function /*caret*/bar() public pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    let (main_text, main_offset) = extract_offset(
        r#"
import "./Foo.sol";

contract Main {
    function selector() external pure returns (bytes4) {
        return Foo.ba/*caret*/r.selector;
    }
}
"#,
    );

    let foo_uri = write_source(&root, "src/Foo.sol", &foo_text);
    let main_uri = write_source(&root, "src/Main.sol", &main_text);

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;
    open_document(&mut service, main_uri.clone(), main_text.clone()).await;

    let location =
        request_goto_definition(&mut service, main_uri, &main_text, main_offset, 39).await;
    let expected_range = TextRange::at(foo_offset, TextSize::from(3));
    let expected_range = to_lsp_range(expected_range, &foo_text);
    assert_eq!(location.uri, foo_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_with_nested_foundry_root() {
    let temp = tempdir().expect("tempdir");
    let parent_root = temp.path().canonicalize().expect("canonicalize root");
    let nested_root = parent_root.join("projects/nested");
    fs::create_dir_all(&nested_root).expect("nested root");
    create_foundry_workspace(&nested_root);

    let (parent_text, parent_offset) = extract_offset(
        r#"
contract Parent {
    function /*caret*/value() public pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    let (child_text, child_offset) = extract_offset(
        r#"
import "./Parent.sol";

contract Child is Parent {
    function foo() public pure returns (uint256) {
        return val/*caret*/ue();
    }
}
"#,
    );

    fs::write(nested_root.join("src/Parent.sol"), &parent_text).expect("write parent");
    fs::write(nested_root.join("src/Child.sol"), &child_text).expect("write child");

    let (mut service, _root_uri) =
        setup_lsp_service(&parent_root, make_server(solidity_analyzer::Server::new)).await;

    let child_uri = Url::from_file_path(nested_root.join("src/Child.sol")).expect("child uri");
    let parent_uri = Url::from_file_path(nested_root.join("src/Parent.sol")).expect("parent uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: child_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: child_text.clone(),
            },
        },
    )
    .await;

    let location =
        request_goto_definition(&mut service, child_uri, &child_text, child_offset, 32).await;
    let expected_range = TextRange::at(parent_offset, TextSize::from(5));
    let expected_range = to_lsp_range(expected_range, &parent_text);
    assert_eq!(location.uri, parent_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_resolves_for_files_created_after_init() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;

    let (parent_text, parent_offset) = extract_offset(
        r#"
contract Parent {
    function /*caret*/value() public pure returns (uint256) {
        return 1;
    }
}
"#,
    );
    let (child_text, child_offset) = extract_offset(
        r#"
import "./Parent.sol";

contract Child is Parent {
    function foo() public pure returns (uint256) {
        return val/*caret*/ue();
    }
}
"#,
    );

    fs::write(root.join("src/Parent.sol"), &parent_text).expect("write parent");
    fs::write(root.join("src/Child.sol"), &child_text).expect("write child");

    let child_uri = Url::from_file_path(root.join("src/Child.sol")).expect("child uri");
    let parent_uri = Url::from_file_path(root.join("src/Parent.sol")).expect("parent uri");
    send_notification(
        &mut service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: child_uri.clone(),
                language_id: "solidity".to_string(),
                version: 1,
                text: child_text.clone(),
            },
        },
    )
    .await;

    let location =
        request_goto_definition(&mut service, child_uri, &child_text, child_offset, 33).await;
    let expected_range = TextRange::at(parent_offset, TextSize::from(5));
    let expected_range = to_lsp_range(expected_range, &parent_text);
    assert_eq!(location.uri, parent_uri);
    assert_eq!(location.range, expected_range);
}

#[tokio::test]
async fn goto_definition_returns_none_for_overloaded_member_reference_without_args() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    create_foundry_workspace(&root);

    let (text, offset) = extract_offset(
        r#"
contract Foo {
    function bar(uint256) public {}
    function bar(string memory) public {}
}

contract Main {
    function test(Foo foo) public {
        function(uint256) external f = foo.ba/*caret*/r;
        f(1);
    }
}
"#,
    );

    let main_uri = write_source(&root, "src/Main.sol", &text);
    let (mut service, _root_uri) =
        setup_lsp_service(&root, make_server(solidity_analyzer::Server::new)).await;
    open_document(&mut service, main_uri.clone(), text.clone()).await;

    let result = request_goto_definition_result(&mut service, main_uri, &text, offset, 34).await;
    assert!(result.is_none());
}

fn first_location(result: Option<GotoDefinitionResponse>) -> Location {
    match result {
        Some(GotoDefinitionResponse::Scalar(location)) => location,
        Some(GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().next().expect("location")
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let link = links.into_iter().next().expect("location link");
            Location::new(link.target_uri, link.target_range)
        }
        None => panic!("expected definition location"),
    }
}

async fn request_goto_definition<S>(
    service: &mut tower_lsp::LspService<S>,
    uri: Url,
    text: &str,
    offset: TextSize,
    id: i64,
) -> Location
where
    S: tower_lsp::LanguageServer,
{
    let result = request_goto_definition_result(service, uri, text, offset, id).await;
    first_location(result)
}

async fn request_goto_definition_result<S>(
    service: &mut tower_lsp::LspService<S>,
    uri: Url,
    text: &str,
    offset: TextSize,
    id: i64,
) -> Option<GotoDefinitionResponse>
where
    S: tower_lsp::LanguageServer,
{
    let position = to_lsp_position(offset, text);
    let params = GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = send_request(service, id, "textDocument/definition", params).await;
    response_result::<Option<GotoDefinitionResponse>>(response)
}

async fn open_document<S>(service: &mut tower_lsp::LspService<S>, uri: Url, text: String)
where
    S: tower_lsp::LanguageServer,
{
    let open_doc = TextDocumentItem {
        uri,
        language_id: "solidity".to_string(),
        version: 1,
        text,
    };
    send_notification(
        service,
        "textDocument/didOpen",
        tower_lsp::lsp_types::DidOpenTextDocumentParams {
            text_document: open_doc,
        },
    )
    .await;
}

fn write_source(root: &Path, relative: &str, text: &str) -> Url {
    let path = root.join(relative);
    fs::write(&path, text).expect("write source");
    Url::from_file_path(path).expect("source uri")
}
