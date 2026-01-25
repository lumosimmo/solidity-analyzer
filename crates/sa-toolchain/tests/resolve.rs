use std::fs;
use std::path::{Path, PathBuf};

use foundry_compilers::solc::Solc;
use sa_test_support::write_stub_solc;
use sa_test_utils::{Fixture, FixtureBuilder};
use sa_toolchain::Toolchain;
use tempfile::tempdir;

mod common;
use common::{ToolchainTestEnv, toml_path};

struct SvmSolcGuard {
    solc_path: PathBuf,
    version_dir: PathBuf,
    created_dir: bool,
    created_file: bool,
}

impl SvmSolcGuard {
    fn path(&self) -> &Path {
        &self.solc_path
    }
}

impl Drop for SvmSolcGuard {
    fn drop(&mut self) {
        if self.created_file {
            let _ = fs::remove_file(&self.solc_path);
        }
        if self.created_dir {
            let _ = fs::remove_dir_all(&self.version_dir);
        }
    }
}

fn install_svm_stub(version: &str) -> SvmSolcGuard {
    let svm_home = Solc::svm_home().expect("svm home");
    let version_dir = svm_home.join(version);
    let solc_path = version_dir.join(format!("solc-{version}"));

    let mut created_dir = false;
    if !version_dir.is_dir() {
        fs::create_dir_all(&version_dir).expect("svm dir");
        created_dir = true;
    }

    let created_file = if solc_path.exists() {
        false
    } else {
        fs::write(&solc_path, "").expect("write svm solc");
        true
    };

    SvmSolcGuard {
        solc_path,
        version_dir,
        created_dir,
        created_file,
    }
}

fn fixture_with_toml(foundry_toml: &str) -> Fixture {
    FixtureBuilder::new()
        .expect("fixture builder")
        .foundry_toml(foundry_toml)
        .build()
        .expect("fixture")
}

fn toolchain_from_fixture(fixture: &Fixture) -> Toolchain {
    Toolchain::new(fixture.config().clone())
}

#[test]
fn resolves_explicit_solc_path() {
    let _env = ToolchainTestEnv::new();

    let solc_dir = tempdir().expect("solc tempdir");
    let solc_path = write_stub_solc(solc_dir.path(), "0.8.20", None);
    let foundry_toml = format!(
        r#"
[profile.default]
solc = "{}"
"#,
        toml_path(&solc_path)
    );

    let fixture = fixture_with_toml(&foundry_toml);
    let toolchain = toolchain_from_fixture(&fixture);
    let resolved = toolchain.resolve().expect("resolve solc");

    assert_eq!(resolved.path(), solc_path.as_path());
    assert_eq!(resolved.version().to_string(), "0.8.20");
}

#[test]
fn resolves_explicit_solc_version_from_svm() {
    let _env = ToolchainTestEnv::new();

    let version = "9.9.9";
    let solc_guard = install_svm_stub(version);

    let foundry_toml = format!(
        r#"
[profile.default]
solc_version = "{version}"
"#
    );

    let fixture = fixture_with_toml(&foundry_toml);
    let toolchain = toolchain_from_fixture(&fixture);
    let resolved = toolchain.resolve().expect("resolve solc");

    assert_eq!(resolved.path(), solc_guard.path());
    assert_eq!(resolved.version().to_string(), version);
}

#[test]
fn install_solc_respects_offline_mode() {
    let _env = ToolchainTestEnv::new();

    let foundry_toml = r#"
[profile.default]
solc_version = "0.8.20"
offline = true
"#;

    let fixture = fixture_with_toml(foundry_toml);
    let toolchain = toolchain_from_fixture(&fixture);
    let error = toolchain
        .install_solc()
        .expect_err("offline install should fail");

    assert!(error.to_string().contains("offline"));
}

#[test]
fn resolves_solc_from_path_when_no_spec() {
    let solc_dir = tempdir().expect("solc dir");
    let solc_path = write_stub_solc(solc_dir.path(), "0.8.21", None);
    let _env = ToolchainTestEnv::new().with_path_prepend(solc_dir.path());

    let fixture = FixtureBuilder::new()
        .expect("fixture builder")
        .build()
        .expect("fixture");

    let toolchain = toolchain_from_fixture(&fixture);
    let resolved = toolchain.resolve().expect("resolve solc");

    assert_eq!(resolved.path(), solc_path.as_path());
    assert_eq!(resolved.version().to_string(), "0.8.21");
}

#[test]
fn resolves_solc_version_requirement_to_latest_installed() {
    let _env = ToolchainTestEnv::new();

    let _guard_old = install_svm_stub("9.9.1");
    let guard_new = install_svm_stub("9.9.2");

    let foundry_toml = r#"
[profile.default]
solc_version = "^9.9.0"
"#;
    let fixture = fixture_with_toml(foundry_toml);
    let toolchain = toolchain_from_fixture(&fixture);
    let resolved = toolchain.resolve().expect("resolve solc");

    assert_eq!(resolved.path(), guard_new.path());
    assert_eq!(resolved.version().to_string(), "9.9.2");
}

#[test]
fn rejects_invalid_solc_spec() {
    let _env = ToolchainTestEnv::new();

    let foundry_toml = r#"
[profile.default]
solc = "not-a-version"
"#;
    let fixture = fixture_with_toml(foundry_toml);
    let toolchain = toolchain_from_fixture(&fixture);
    let error = toolchain.resolve().expect_err("invalid solc spec");

    assert!(error.to_string().contains("invalid solc specification"));
}

#[test]
fn install_solc_accepts_local_path_in_offline_mode() {
    let _env = ToolchainTestEnv::new();

    let solc_dir = tempdir().expect("solc dir");
    let solc_path = write_stub_solc(solc_dir.path(), "0.8.19", None);
    let foundry_toml = format!(
        r#"
[profile.default]
solc = "{}"
offline = true
"#,
        toml_path(&solc_path)
    );
    let fixture = fixture_with_toml(&foundry_toml);
    let toolchain = toolchain_from_fixture(&fixture);
    assert!(toolchain.solc_spec_is_path());

    let message = toolchain.install_solc().expect("install solc");
    assert!(message.contains("local path"));
    assert!(message.contains("version 0.8.19"));
}

#[test]
fn solc_spec_is_path_detects_path_like_specs() {
    let _env = ToolchainTestEnv::new();

    let fixture = fixture_with_toml(
        r#"
[profile.default]
solc = "bin/solc"
"#,
    );
    let toolchain = toolchain_from_fixture(&fixture);
    assert!(toolchain.solc_spec_is_path());

    let fixture = fixture_with_toml(
        r#"
[profile.default]
solc = "solc.exe"
"#,
    );
    let toolchain = toolchain_from_fixture(&fixture);
    assert!(toolchain.solc_spec_is_path());
}

#[test]
fn solc_spec_is_path_is_false_for_version_specs() {
    let _env = ToolchainTestEnv::new();

    let fixture = fixture_with_toml(
        r#"
[profile.default]
solc_version = "0.8.20"
"#,
    );
    let toolchain = toolchain_from_fixture(&fixture);
    assert!(!toolchain.solc_spec_is_path());
}

#[test]
fn install_solc_offline_without_spec_errors() {
    let _env = ToolchainTestEnv::new();

    let fixture = fixture_with_toml(
        r#"
[profile.default]
offline = true
"#,
    );
    let toolchain = toolchain_from_fixture(&fixture);
    let error = toolchain
        .install_solc()
        .expect_err("offline install should fail");

    assert!(error.to_string().contains("offline"));
}

#[test]
fn resolve_errors_when_version_missing() {
    let home = tempdir().expect("home dir");
    let _env = ToolchainTestEnv::new().with_home(home.path());

    let fixture = fixture_with_toml(
        r#"
[profile.default]
solc_version = "0.8.99"
"#,
    );
    let toolchain = toolchain_from_fixture(&fixture);
    let error = toolchain.resolve().expect_err("missing solc version");

    assert!(error.to_string().contains("not installed"));
}

#[test]
fn resolve_version_requirement_errors_without_installed_versions() {
    let _env = ToolchainTestEnv::new();

    let fixture = fixture_with_toml(
        r#"
[profile.default]
solc_version = "^9999.0.0"
"#,
    );
    let toolchain = toolchain_from_fixture(&fixture);
    let error = toolchain
        .resolve()
        .expect_err("missing matching solc versions");

    assert!(
        error
            .to_string()
            .contains("no installed solc version matches requirement")
    );
}

#[test]
fn install_solc_reports_already_installed_version() {
    let _env = ToolchainTestEnv::new();

    let version = "10.0.0";
    let _guard = install_svm_stub(version);

    let foundry_toml = format!(
        r#"
[profile.default]
solc_version = "{version}"
"#
    );
    let fixture = fixture_with_toml(&foundry_toml);
    let toolchain = toolchain_from_fixture(&fixture);
    let message = toolchain.install_solc().expect("install solc");

    assert!(message.contains("already installed"));
    assert!(message.contains(version));
}
