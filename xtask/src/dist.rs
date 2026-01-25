use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::{XtaskError, workspace_root};

pub fn run(out_dir: Option<PathBuf>) -> Result<(), XtaskError> {
    let root = workspace_root()?;
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["build", "--locked", "--release", "-p", "solidity-analyzer"])
        .status()
        .map_err(|err| XtaskError::new(format!("failed to run cargo build: {err}")))?;

    if !status.success() {
        return Err(XtaskError::new("dist build failed"));
    }

    let exe_name = if cfg!(windows) {
        "solidity-analyzer.exe"
    } else {
        "solidity-analyzer"
    };
    let source = root.join("target").join("release").join(exe_name);
    if !source.exists() {
        return Err(XtaskError::new(format!(
            "missing build artifact at {}",
            source.display()
        )));
    }

    let out_dir = out_dir.unwrap_or_else(|| root.join("dist"));
    fs::create_dir_all(&out_dir)
        .map_err(|err| XtaskError::new(format!("failed to create dist dir: {err}")))?;

    let artifact_name = format!(
        "solidity-analyzer-{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let artifact_name = if cfg!(windows) {
        format!("{artifact_name}.exe")
    } else {
        artifact_name
    };
    let destination = out_dir.join(artifact_name);

    fs::copy(&source, &destination)
        .map_err(|err| XtaskError::new(format!("failed to copy artifact: {err}")))?;

    println!("dist artifact: {}", destination.display());
    Ok(())
}
