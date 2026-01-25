use std::fs;

use sa_paths::NormalizedPath;
use sa_project_model::Remapping;
use sa_test_support::lsp::{response_result, send_notification, send_request};
use sa_test_support::setup_foundry_root;
use tempfile::tempdir;
use tower_lsp::lsp_types::{
    ClientCapabilities, DidChangeWatchedFilesParams, FileChangeType, FileEvent, InitializeParams,
    InitializedParams, Url, WorkspaceFolder,
};

async fn initialize_server(
    root_uri: Option<Url>,
    workspace_folders: Option<Vec<WorkspaceFolder>>,
) -> tower_lsp::LspService<solidity_analyzer::Server> {
    let (mut service, _socket) = tower_lsp::LspService::new(solidity_analyzer::Server::new);
    let initialize = InitializeParams {
        root_uri,
        workspace_folders,
        capabilities: ClientCapabilities::default(),
        ..InitializeParams::default()
    };
    let response = send_request(&mut service, 1, "initialize", initialize).await;
    let _ = response_result::<tower_lsp::lsp_types::InitializeResult>(response);
    send_notification(&mut service, "initialized", InitializedParams {}).await;
    service
}

fn remapping_target<'a>(remappings: &'a [Remapping], from: &str) -> Option<&'a str> {
    remappings
        .iter()
        .find(|remapping| remapping.from() == from)
        .map(Remapping::to)
}

#[tokio::test]
async fn initialize_loads_foundry_workspace_and_reloads_on_change() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    fs::create_dir_all(root.join("src")).expect("src dir");

    let foundry_toml = r#"
[profile.default]
remappings = ["lib/=lib/forge-std/src/"]
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let root_uri = Url::from_file_path(&root).expect("root uri");
    let mut service = initialize_server(Some(root_uri.clone()), None).await;

    let (analysis, _) = service.inner().snapshot().await;
    let workspace = analysis.workspace();
    assert_eq!(
        workspace.root(),
        &NormalizedPath::new(root.to_string_lossy())
    );
    let profile = workspace.profile(Some("default"));
    let remappings = profile.remappings();
    let target = remapping_target(remappings, "lib/").expect("remapping");
    assert!(target.ends_with("lib/forge-std/src/"));
    drop(analysis);

    let updated_toml = r#"
[profile.default]
remappings = ["lib/=lib/updated/src/"]
"#;
    fs::write(root.join("foundry.toml"), updated_toml).expect("rewrite foundry.toml");
    let config_uri = Url::from_file_path(root.join("foundry.toml")).expect("config uri");
    let watched = DidChangeWatchedFilesParams {
        changes: vec![FileEvent {
            uri: config_uri,
            typ: FileChangeType::CHANGED,
        }],
    };
    send_notification(&mut service, "workspace/didChangeWatchedFiles", watched).await;

    let (analysis, _) = service.inner().snapshot().await;
    let profile = analysis.workspace().profile(Some("default"));
    let remappings = profile.remappings();
    let target = remapping_target(remappings, "lib/").expect("remapping after reload");
    assert!(target.ends_with("lib/updated/src/"));
}

#[tokio::test]
async fn initialize_prefers_workspace_folders_over_root_uri() {
    let temp_a = tempdir().expect("tempdir a");
    let root_a = temp_a.path().canonicalize().expect("canonicalize root a");
    setup_foundry_root(&root_a);
    fs::write(root_a.join("foundry.toml"), "[profile.default]").expect("write foundry.toml a");

    let temp_b = tempdir().expect("tempdir b");
    let root_b = temp_b.path().canonicalize().expect("canonicalize root b");
    setup_foundry_root(&root_b);
    fs::write(root_b.join("foundry.toml"), "[profile.default]").expect("write foundry.toml b");

    let root_a_uri = Url::from_file_path(&root_a).expect("root a uri");
    let root_b_uri = Url::from_file_path(&root_b).expect("root b uri");
    let folders = vec![WorkspaceFolder {
        uri: root_a_uri.clone(),
        name: "alpha".to_string(),
    }];

    let service = initialize_server(Some(root_b_uri), Some(folders)).await;

    let (analysis, _) = service.inner().snapshot().await;
    assert_eq!(
        analysis.workspace().root(),
        &NormalizedPath::new(root_a.to_string_lossy())
    );
}

#[tokio::test]
async fn initialize_discovers_root_from_nested_workspace_folder() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    setup_foundry_root(&root);
    fs::write(root.join("foundry.toml"), "[profile.default]").expect("write foundry.toml");

    let nested = root.join("nested/project/src");
    fs::create_dir_all(&nested).expect("create nested dirs");
    let nested_uri = Url::from_file_path(&nested).expect("nested uri");

    let folders = vec![WorkspaceFolder {
        uri: nested_uri,
        name: "nested".to_string(),
    }];

    let service = initialize_server(None, Some(folders)).await;

    let (analysis, _) = service.inner().snapshot().await;
    assert_eq!(
        analysis.workspace().root(),
        &NormalizedPath::new(root.to_string_lossy())
    );
}

#[tokio::test]
async fn reloads_when_remappings_txt_changes() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    setup_foundry_root(&root);

    let foundry_toml = r#"
[profile.default]
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");
    fs::write(root.join("remappings.txt"), "lib/=lib/old/src/").expect("write remappings.txt");

    let root_uri = Url::from_file_path(&root).expect("root uri");
    let mut service = initialize_server(Some(root_uri.clone()), None).await;

    let (analysis, _) = service.inner().snapshot().await;
    let profile = analysis.workspace().profile(Some("default"));
    let remappings = profile.remappings();
    let target = remapping_target(remappings, "lib/").expect("remapping");
    assert!(target.ends_with("lib/old/src/"));
    drop(analysis);

    fs::write(root.join("remappings.txt"), "lib/=lib/new/src/").expect("rewrite remappings.txt");
    let config_uri = Url::from_file_path(root.join("remappings.txt")).expect("config uri");
    let watched = DidChangeWatchedFilesParams {
        changes: vec![FileEvent {
            uri: config_uri,
            typ: FileChangeType::CHANGED,
        }],
    };
    send_notification(&mut service, "workspace/didChangeWatchedFiles", watched).await;

    let (analysis, _) = service.inner().snapshot().await;
    let profile = analysis.workspace().profile(Some("default"));
    let remappings = profile.remappings();
    let target = remapping_target(remappings, "lib/").expect("remapping after reload");
    assert!(target.ends_with("lib/new/src/"));
}

#[tokio::test]
async fn ignores_non_config_watched_files() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    setup_foundry_root(&root);

    let foundry_toml = r#"
[profile.default]
remappings = ["lib/=lib/old/src/"]
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let root_uri = Url::from_file_path(&root).expect("root uri");
    let mut service = initialize_server(Some(root_uri.clone()), None).await;

    let (analysis, _) = service.inner().snapshot().await;
    let profile = analysis.workspace().profile(Some("default"));
    let remappings = profile.remappings();
    let target = remapping_target(remappings, "lib/").expect("remapping");
    assert!(target.ends_with("lib/old/src/"));
    drop(analysis);

    let updated_toml = r#"
[profile.default]
remappings = ["lib/=lib/new/src/"]
"#;
    fs::write(root.join("foundry.toml"), updated_toml).expect("rewrite foundry.toml");

    let other_path = root.join("src/Other.sol");
    fs::write(&other_path, r#"contract Other {}"#).expect("write other source");
    let other_uri = Url::from_file_path(&other_path).expect("other uri");
    let watched = DidChangeWatchedFilesParams {
        changes: vec![FileEvent {
            uri: other_uri,
            typ: FileChangeType::CHANGED,
        }],
    };
    send_notification(&mut service, "workspace/didChangeWatchedFiles", watched).await;

    let (analysis, _) = service.inner().snapshot().await;
    let profile = analysis.workspace().profile(Some("default"));
    let remappings = profile.remappings();
    let target = remapping_target(remappings, "lib/").expect("remapping after change");
    assert!(target.ends_with("lib/old/src/"));
}

#[tokio::test]
async fn reloads_when_foundry_config_is_created() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonicalize root");
    setup_foundry_root(&root);
    assert!(
        !root.join("foundry.toml").exists(),
        "unexpected foundry.toml"
    );

    let root_uri = Url::from_file_path(&root).expect("root uri");
    let mut service = initialize_server(Some(root_uri.clone()), None).await;

    let foundry_toml = r#"
[profile.default]
remappings = ["lib/=lib/new/src/"]
"#;
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let config_uri = Url::from_file_path(root.join("foundry.toml")).expect("config uri");
    let watched = DidChangeWatchedFilesParams {
        changes: vec![FileEvent {
            uri: config_uri,
            typ: FileChangeType::CREATED,
        }],
    };
    send_notification(&mut service, "workspace/didChangeWatchedFiles", watched).await;

    let (analysis, _) = service.inner().snapshot().await;
    let profile = analysis.workspace().profile(Some("default"));
    let remappings = profile.remappings();
    let target = remapping_target(remappings, "lib/").expect("remapping after create");
    assert!(target.ends_with("lib/new/src/"));
}
