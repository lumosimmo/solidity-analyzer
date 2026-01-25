use sa_test_utils::FixtureBuilder;
use sa_toolchain::{Toolchain, select_version_for_requirement};
use semver::VersionReq;

mod common;
use common::ToolchainTestEnv;

#[test]
fn auto_detects_solc_version_from_sources() {
    let _env = ToolchainTestEnv::new();

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/Main.sol",
            r#"
pragma solidity ^0.8.0;

contract Main {}
"#,
        )
        .build()
        .expect("fixture");

    let toolchain = Toolchain::new(fixture.config().clone());
    let versions = toolchain
        .auto_detect_versions()
        .expect("auto detect versions");

    assert_eq!(versions.len(), 1);
    let expected =
        select_version_for_requirement(&VersionReq::parse("^0.8.0").expect("version req"))
            .expect("version requirement should resolve");
    assert_eq!(versions[0], expected);
}

#[test]
fn auto_detect_sorts_and_deduplicates_versions() {
    let _env = ToolchainTestEnv::new();

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .file(
            "src/A.sol",
            r#"
pragma solidity ^0.8.0;

contract A {}
"#,
        )
        .file(
            "src/B.sol",
            r#"
pragma solidity ^0.7.0;

contract B {}
"#,
        )
        .file(
            "src/C.sol",
            r#"
pragma solidity ^0.8.0;

contract C {}
"#,
        )
        .build()
        .expect("fixture");

    let toolchain = Toolchain::new(fixture.config().clone());
    let versions = toolchain
        .auto_detect_versions()
        .expect("auto detect versions");

    let mut expected = vec![
        select_version_for_requirement(&VersionReq::parse("^0.7.0").expect("version req"))
            .expect("version requirement should resolve"),
        select_version_for_requirement(&VersionReq::parse("^0.8.0").expect("version req"))
            .expect("version requirement should resolve"),
    ];
    expected.sort();
    expected.dedup();

    assert_eq!(versions, expected);
}
