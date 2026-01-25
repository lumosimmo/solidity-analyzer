use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{XtaskError, workspace_root};

const SERVER_BINARY: &str = "solidity-analyzer";

struct TargetSpec {
    vscode: &'static str,
    rust: &'static str,
    is_windows: bool,
}

const TARGETS: &[TargetSpec] = &[
    TargetSpec {
        vscode: "linux-x64",
        rust: "x86_64-unknown-linux-gnu",
        is_windows: false,
    },
    TargetSpec {
        vscode: "linux-arm64",
        rust: "aarch64-unknown-linux-gnu",
        is_windows: false,
    },
    TargetSpec {
        vscode: "darwin-x64",
        rust: "x86_64-apple-darwin",
        is_windows: false,
    },
    TargetSpec {
        vscode: "darwin-arm64",
        rust: "aarch64-apple-darwin",
        is_windows: false,
    },
    TargetSpec {
        vscode: "win32-x64",
        rust: "x86_64-pc-windows-msvc",
        is_windows: true,
    },
    TargetSpec {
        vscode: "win32-arm64",
        rust: "aarch64-pc-windows-msvc",
        is_windows: true,
    },
];

pub fn run(target: Option<String>, out: Option<PathBuf>) -> Result<(), XtaskError> {
    let root = workspace_root()?;
    let vscode_target = match target.as_deref() {
        Some(value) => value,
        None => host_vscode_target()?,
    };
    let spec = lookup_target(vscode_target).ok_or_else(|| {
        XtaskError::new(format!(
            "unsupported VS Code target '{vscode_target}'. Expected one of: {}",
            TARGETS
                .iter()
                .map(|entry| entry.vscode)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;

    let rust_target = if target.is_some() {
        Some(spec.rust)
    } else {
        None
    };

    build_binary(&root, rust_target)?;
    let exe_name = binary_name(spec.is_windows);
    let source = built_binary_path(&root, rust_target, exe_name);
    if !source.exists() {
        return Err(XtaskError::new(format!(
            "missing build artifact at {}",
            source.display()
        )));
    }

    let server_dir = root.join("editors").join("code").join("server");
    fs::create_dir_all(&server_dir)
        .map_err(|err| XtaskError::new(format!("failed to create server dir: {err}")))?;
    let destination = server_dir.join(exe_name);
    fs::copy(&source, &destination)
        .map_err(|err| XtaskError::new(format!("failed to copy server binary: {err}")))?;

    install_bun_dependencies(&root)?;

    let mut cmd = Command::new("bun");
    cmd.current_dir(root.join("editors").join("code")).args([
        "run",
        "package",
        "--target",
        vscode_target,
    ]);
    if let Some(out_path) = out {
        let out_path = if out_path.is_absolute() {
            out_path
        } else {
            root.join(out_path)
        };
        cmd.args(["--out", &out_path.to_string_lossy()]);
    }

    let status = cmd
        .status()
        .map_err(|err| XtaskError::new(format!("failed to run bun package: {err}")))?;
    if !status.success() {
        return Err(XtaskError::new("extension packaging failed"));
    }

    Ok(())
}

fn install_bun_dependencies(root: &Path) -> Result<(), XtaskError> {
    let status = Command::new("bun")
        .current_dir(root.join("editors").join("code"))
        .args(["install", "--frozen-lockfile"])
        .status()
        .map_err(|err| XtaskError::new(format!("failed to run bun install: {err}")))?;
    if !status.success() {
        return Err(XtaskError::new("bun install failed"));
    }

    Ok(())
}

fn lookup_target(vscode_target: &str) -> Option<&'static TargetSpec> {
    TARGETS.iter().find(|entry| entry.vscode == vscode_target)
}

fn host_vscode_target() -> Result<&'static str, XtaskError> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "x86_64") => Ok("darwin-x64"),
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("windows", "x86_64") => Ok("win32-x64"),
        ("windows", "aarch64") => Ok("win32-arm64"),
        _ => Err(XtaskError::new(format!(
            "unsupported host platform {os}-{arch}; use --target to override"
        ))),
    }
}

fn build_binary(root: &Path, rust_target: Option<&str>) -> Result<(), XtaskError> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root)
        .args(["build", "--locked", "--release", "-p", "solidity-analyzer"]);
    if let Some(target) = rust_target {
        cmd.args(["--target", target]);
    }

    let status = cmd
        .status()
        .map_err(|err| XtaskError::new(format!("failed to run cargo build: {err}")))?;
    if !status.success() {
        return Err(XtaskError::new("ext build failed"));
    }

    Ok(())
}

fn built_binary_path(root: &Path, rust_target: Option<&str>, exe_name: &str) -> PathBuf {
    match rust_target {
        Some(target) => root
            .join("target")
            .join(target)
            .join("release")
            .join(exe_name),
        None => root.join("target").join("release").join(exe_name),
    }
}

fn binary_name(is_windows: bool) -> &'static str {
    if is_windows {
        "solidity-analyzer.exe"
    } else {
        SERVER_BINARY
    }
}
