use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::{XtaskError, workspace_root};

pub fn run(out: Option<PathBuf>) -> Result<(), XtaskError> {
    let root = workspace_root()?;
    let out_path = out.unwrap_or_else(|| {
        root.join("target")
            .join("profile")
            .join("lsp-requests.jsonl")
    });

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| XtaskError::new(format!("failed to create profile directory: {err}")))?;
    }
    if out_path.exists() {
        fs::remove_file(&out_path)
            .map_err(|err| XtaskError::new(format!("failed to clear profile output: {err}")))?;
    }

    let status = Command::new("cargo")
        .current_dir(&root)
        .env("SA_PROFILE_PATH", &out_path)
        .args(["test", "-p", "solidity-analyzer", "--test", "lsp_harness"])
        .status()
        .map_err(|err| XtaskError::new(format!("failed to run cargo test: {err}")))?;

    if !status.success() {
        return Err(XtaskError::new("profile run failed"));
    }

    println!("profile output: {}", out_path.display());
    Ok(())
}
