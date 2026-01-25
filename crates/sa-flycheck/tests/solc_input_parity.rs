use std::fs;

use foundry_compilers::artifacts::{
    BytecodeHash, EvmVersion, ModelCheckerEngine, ModelCheckerTarget, RevertStrings, SolcInput,
};
use sa_flycheck::{FlycheckConfig, FlycheckHandle, FlycheckRequest};
use sa_test_support::setup_foundry_root;
use sa_test_utils::toolchain::{
    SolcTestMode, StubSolcOptions, solc_path_for_tests_with_mode, stub_solc_input_path,
    stub_solc_output_empty,
};
use sa_test_utils::{EnvGuard, env_lock, load_foundry_config};
use tempfile::tempdir;
use tokio::time::{Duration, timeout};

const FLYCHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::await_holding_lock)]
async fn flycheck_solc_input_matches_foundry_settings() {
    let _lock = env_lock();
    let _profile_guard = EnvGuard::unset("FOUNDRY_PROFILE");
    let _solc_guard = EnvGuard::unset("FOUNDRY_SOLC_VERSION");
    let _dapp_guard = EnvGuard::unset("DAPP_SOLC_VERSION");

    let root_dir = tempdir().expect("tempdir");
    let root = root_dir.path();
    setup_foundry_root(root);
    let source = r#"
pragma solidity ^0.8.20;

contract Main {
    function run() external pure returns (uint256) {
        return 1;
    }
}
"#;
    fs::write(root.join("src/Main.sol"), source).expect("write source");

    let solc_dir = tempdir().expect("solc dir");
    let solc_json = stub_solc_output_empty();
    let solc_path = solc_path_for_tests_with_mode(
        solc_dir.path(),
        "0.8.20",
        StubSolcOptions {
            json: Some(solc_json),
            sleep_seconds: None,
            capture_stdin: true,
        },
        SolcTestMode::Stub,
    );

    let solc_toml = solc_path
        .to_str()
        .expect("valid UTF-8 path")
        .replace('\\', "/");
    let foundry_toml = format!(
        r#"
[profile.default]
solc = "{solc}"
optimizer = true
optimizer_runs = 999
via_ir = true
evm_version = "paris"
revert_strings = "strip"
bytecode_hash = "ipfs"
cbor_metadata = false
extra_output = ["metadata", "ir-optimized"]
ast = true
extra_args = ["--foo", "--bar=1"]

[profile.default.model_checker]
engine = "chc"
targets = ["assert"]
"#,
        solc = solc_toml
    );
    fs::write(root.join("foundry.toml"), foundry_toml).expect("write foundry.toml");

    let config = load_foundry_config(root, None).expect("load config");
    let (handle, mut results) = FlycheckHandle::spawn(FlycheckConfig::default());
    handle
        .check(FlycheckRequest::new(config))
        .await
        .expect("send flycheck request");

    let _ = timeout(FLYCHECK_TIMEOUT, results.recv())
        .await
        .expect("flycheck timeout")
        .expect("flycheck result");

    let input_path = stub_solc_input_path(&solc_path);
    let input_json = fs::read_to_string(&input_path).expect("read solc input");
    let input: SolcInput = serde_json::from_str(&input_json).expect("parse solc input");
    let settings = input.settings;

    assert_eq!(settings.optimizer.enabled, Some(true));
    assert_eq!(settings.optimizer.runs, Some(999));
    assert_eq!(settings.via_ir, Some(true));
    assert_eq!(settings.evm_version, Some(EvmVersion::Paris));

    let metadata = settings.metadata.expect("metadata");
    assert_eq!(metadata.bytecode_hash, Some(BytecodeHash::Ipfs));
    assert_eq!(metadata.cbor_metadata, Some(false));

    let debug = settings.debug.expect("debug settings");
    assert_eq!(debug.revert_strings, Some(RevertStrings::Strip));

    let model_checker = settings.model_checker.expect("model checker");
    assert_eq!(model_checker.engine, Some(ModelCheckerEngine::CHC));
    assert_eq!(
        model_checker.targets,
        Some(vec![ModelCheckerTarget::Assert])
    );

    let (_, file_outputs) = settings
        .output_selection
        .0
        .iter()
        .next()
        .expect("output selection");
    let contract_outputs = file_outputs.get("*").expect("contract outputs");
    assert!(contract_outputs.contains(&"metadata".to_string()));
    assert!(contract_outputs.contains(&"irOptimized".to_string()));
    let file_level_outputs = file_outputs.get("").expect("file outputs");
    assert!(file_level_outputs.contains(&"ast".to_string()));
}
