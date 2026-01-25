use std::path::PathBuf;

use xtask::{Command, help_text, parse_args};

fn assert_parse_error(args: &[&str], expected: &str) {
    let err = parse_args(args.iter().copied()).expect_err("expected parse error");
    assert!(
        err.to_string().contains(expected),
        "expected error containing '{expected}', got '{err}'"
    );
}

#[test]
fn parse_help_command() {
    let cmd = parse_args(["help"]).expect("parse help");
    assert_eq!(cmd, Command::Help);
}

#[test]
fn parse_default_to_help_when_empty() {
    let cmd = parse_args(std::iter::empty::<&str>()).expect("parse empty");
    assert_eq!(cmd, Command::Help);
}

#[test]
fn parse_codegen_check() {
    let cmd = parse_args(["codegen", "--check"]).expect("parse codegen");
    assert_eq!(cmd, Command::Codegen { check: true });
}

#[test]
fn parse_profile_out() {
    let cmd = parse_args(["profile", "--out", "target/profile.jsonl"]).expect("parse profile");
    assert_eq!(
        cmd,
        Command::Profile {
            out: Some(PathBuf::from("target/profile.jsonl"))
        }
    );
}

#[test]
fn parse_dist_out_dir() {
    let cmd = parse_args(["dist", "--out-dir", "dist"]).expect("parse dist");
    assert_eq!(
        cmd,
        Command::Dist {
            out_dir: Some(PathBuf::from("dist")),
            skip_build: false
        }
    );
}

#[test]
fn parse_dist_skip_build() {
    let cmd = parse_args(["dist", "--skip-build"]).expect("parse dist");
    assert_eq!(
        cmd,
        Command::Dist {
            out_dir: None,
            skip_build: true
        }
    );
}

#[test]
fn parse_dist_out_dir_and_skip_build() {
    let cmd = parse_args(["dist", "--out-dir", "dist", "--skip-build"]).expect("parse dist");
    assert_eq!(
        cmd,
        Command::Dist {
            out_dir: Some(PathBuf::from("dist")),
            skip_build: true
        }
    );
}

#[test]
fn parse_ext_target_and_out() {
    let cmd =
        parse_args(["ext", "--target", "linux-x64", "--out", "dist/sa.vsix"]).expect("parse ext");
    assert_eq!(
        cmd,
        Command::Ext {
            target: Some("linux-x64".to_string()),
            out: Some(PathBuf::from("dist/sa.vsix"))
        }
    );
}

#[test]
fn help_includes_ext_flags() {
    let text = help_text();
    assert!(text.contains("ext"));
    assert!(text.contains("--target"));
    assert!(text.contains("--out"));
}

#[test]
fn help_includes_dist_flags() {
    let text = help_text();
    assert!(text.contains("dist"));
    assert!(text.contains("--out-dir"));
    assert!(text.contains("--skip-build"));
}

#[test]
fn parse_unknown_command_errors() {
    assert_parse_error(&["unknown"], "unknown command");
}

#[test]
fn parse_codegen_rejects_extra_args() {
    assert_parse_error(&["codegen", "--nope"], "unexpected argument for codegen");
}

#[test]
fn parse_profile_requires_out_value() {
    assert_parse_error(&["profile", "--out"], "missing value for --out");
    assert_parse_error(&["profile", "--out", ""], "missing value for --out");
    assert_parse_error(&["profile", "--out", "a", "--out", "b"], "duplicate --out");
}

#[test]
fn parse_dist_requires_out_dir_value() {
    assert_parse_error(&["dist", "--out-dir"], "missing value for --out-dir");
    assert_parse_error(&["dist", "--out-dir", ""], "missing value for --out-dir");
    assert_parse_error(
        &["dist", "--out-dir", "a", "--out-dir", "b"],
        "duplicate --out-dir",
    );
}

#[test]
fn parse_dist_rejects_invalid_flags() {
    assert_parse_error(
        &["dist", "--skip-build", "--skip-build"],
        "duplicate --skip-build",
    );
    assert_parse_error(&["dist", "--nope"], "unexpected argument for dist");
}

#[test]
fn parse_ext_rejects_invalid_flags_and_values() {
    assert_parse_error(&["ext", "--target"], "missing value for --target");
    assert_parse_error(&["ext", "--out"], "missing value for --out");
    assert_parse_error(
        &["ext", "--target", "-invalid"],
        "invalid value for --target",
    );
    assert_parse_error(&["ext", "--out", "-invalid"], "invalid value for --out");
    assert_parse_error(
        &["ext", "--target", "a", "--target", "b"],
        "duplicate --target",
    );
    assert_parse_error(&["ext", "--out", "a", "--out", "b"], "duplicate --out");
    assert_parse_error(&["ext", "--nope"], "unexpected argument for ext");
}
