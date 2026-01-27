use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sa_base_db::{Database, LanguageKind, ProjectId};
use sa_config::ResolvedFoundryConfig;
use sa_ide::{Analysis, AnalysisChange, AnalysisHost};
use sa_paths::NormalizedPath;
use sa_project_model::{FoundryProfile, FoundryWorkspace, Remapping};
use sa_span::{TextRange, TextSize};
use sa_vfs::{Vfs, VfsChange, VfsSnapshot};

pub mod lsp;

/// Extracts the cursor position from text marked with `/*caret*/`.
///
/// Returns the text with the marker removed and the offset position.
pub fn extract_offset(text: &str) -> (String, TextSize) {
    let marker = "/*caret*/";
    let idx = text.find(marker).expect("marker not found");
    let cleaned = text.replacen(marker, "", 1);
    (cleaned, TextSize::from(idx as u32))
}

/// Extracts multiple cursor positions from text marked with custom markers.
///
/// Returns the text with all markers removed and offsets in the same order as `markers`.
pub fn extract_offsets(text: &str, markers: &[&str]) -> (String, Vec<TextSize>) {
    let mut positions = Vec::with_capacity(markers.len());
    for marker in markers {
        let idx = text
            .find(marker)
            .unwrap_or_else(|| panic!("marker not found: {marker}"));
        positions.push((idx, marker.len(), *marker));
    }
    positions.sort_by_key(|(idx, _, _)| *idx);

    let mut cleaned = String::with_capacity(text.len());
    let mut last = 0;
    for (idx, len, _) in &positions {
        cleaned.push_str(&text[last..*idx]);
        last = idx + len;
    }
    cleaned.push_str(&text[last..]);

    let mut offsets = Vec::with_capacity(markers.len());
    for marker in markers {
        let (idx, _, _) = positions
            .iter()
            .find(|(_, _, candidate)| candidate == marker)
            .unwrap_or_else(|| panic!("marker not found: {marker}"));
        let removed_before = positions
            .iter()
            .filter(|(other_idx, _, _)| other_idx < idx)
            .map(|(_, marker_len, _)| *marker_len)
            .sum::<usize>();
        offsets.push(TextSize::from((idx - removed_before) as u32));
    }

    (cleaned, offsets)
}

/// Finds the TextRange for the first occurrence of `needle` in `text`.
pub fn find_range(text: &str, needle: &str) -> TextRange {
    let start = text.find(needle).expect("needle");
    let end = start + needle.len();
    TextRange::new(TextSize::from(start as u32), TextSize::from(end as u32))
}

/// Extracts a substring from text using a TextRange.
pub fn slice_range(text: &str, range: TextRange) -> &str {
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    &text[start..end]
}

/// Creates the standard Foundry workspace directories for tests.
pub fn setup_foundry_root(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::create_dir_all(root.join("lib")).expect("lib dir");
    fs::create_dir_all(root.join("test")).expect("test dir");
    fs::create_dir_all(root.join("script")).expect("script dir");
}

/// Creates the standard Foundry workspace directories plus optional extras used in tests.
pub fn setup_foundry_root_with_extras(root: &Path) {
    setup_foundry_root(root);
    fs::create_dir_all(root.join("lib2")).expect("lib2 dir");
    fs::create_dir_all(root.join("allow")).expect("allow dir");
    fs::create_dir_all(root.join("include")).expect("include dir");
    fs::create_dir_all(root.join("cache")).expect("cache dir");
    fs::create_dir_all(root.join("out/build-info")).expect("build-info dir");
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StubSolcOptions<'a> {
    pub json: Option<&'a str>,
    pub sleep_seconds: Option<u64>,
    pub capture_stdin: bool,
}

/// Writes a stub `solc` binary for tests and returns the binary path.
pub fn write_stub_solc_with_options(
    dir: &Path,
    version: &str,
    options: StubSolcOptions<'_>,
) -> PathBuf {
    let path = if cfg!(windows) {
        dir.join("solc.bat")
    } else {
        dir.join("solc")
    };

    let json = options
        .json
        .unwrap_or(r#"{"errors":[],"sources":{},"contracts":{}}"#);
    let sleep = options.sleep_seconds.unwrap_or(0);
    let capture = options.capture_stdin;
    let output_path = path.with_file_name("solc-output.json");
    let version_path = path.with_file_name("solc-version.txt");
    let version_output = if cfg!(windows) {
        format!("solc, the solidity compiler commandline interface\r\nVersion: {version}\r\n")
    } else {
        format!("solc, the solidity compiler commandline interface\nVersion: {version}\n")
    };
    fs::write(&output_path, json).expect("write stub solc output");
    fs::write(&version_path, version_output).expect("write stub solc version");
    let script = if cfg!(windows) {
        let sleep_block = if sleep > 0 {
            format!(
                "if {sleep} NEQ 0 (\r\n\
timeout /t {sleep} /nobreak >nul\r\n\
)\r\n"
            )
        } else {
            String::new()
        };
        let stdin_block = if capture {
            "more > \"%script_dir%solc-input.json\"\r\n".to_string()
        } else {
            "more > nul\r\n".to_string()
        };
        format!(
            "@echo off\r\n\
setlocal\r\n\
set \"script_dir=%~dp0\"\r\n\
if \"%1\"==\"--version\" (\r\n\
 type \"%script_dir%solc-version.txt\"\r\n\
 exit /b 0\r\n\
)\r\n\
{sleep_block}\
{stdin_block}\
type \"%script_dir%solc-output.json\"\r\n"
        )
    } else {
        let sleep_line = if sleep > 0 {
            format!("sleep {sleep}\n")
        } else {
            String::new()
        };
        let stdin_line = if capture {
            "cat > \"$script_dir/solc-input.json\"\n".to_string()
        } else {
            "cat >/dev/null\n".to_string()
        };
        format!(
            "#!/bin/sh\n\
script_dir=\"$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\"\n\
if [ \"$1\" = \"--version\" ]; then\n\
  cat \"$script_dir/solc-version.txt\"\n\
  exit 0\n\
fi\n\
{sleep_line}\
{stdin_line}\
cat \"$script_dir/solc-output.json\"\n"
        )
    };

    fs::write(&path, script).expect("write stub solc");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("set permissions");
    }

    path
}

/// Writes a stub `solc` binary for tests and returns the binary path.
pub fn write_stub_solc(dir: &Path, version: &str, json: Option<&str>) -> PathBuf {
    write_stub_solc_with_options(
        dir,
        version,
        StubSolcOptions {
            json,
            sleep_seconds: None,
            capture_stdin: false,
        },
    )
}

/// Sets up an Analysis instance with the given files and remappings.
///
/// This is the standard test setup helper for sa-ide tests. It creates a VFS
/// with the provided files, sets up a workspace, and returns an Analysis
/// snapshot along with the VFS snapshot for file lookups.
pub fn setup_analysis(
    files: Vec<(NormalizedPath, String)>,
    remappings: Vec<Remapping>,
) -> (Analysis, VfsSnapshot) {
    let mut vfs = Vfs::default();
    for (path, text) in &files {
        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from(text.as_str()),
        });
    }
    let snapshot = vfs.snapshot();

    let root = NormalizedPath::new("/workspace");
    let default_profile = FoundryProfile::new("default").with_remappings(remappings);
    let workspace = FoundryWorkspace::new(root);
    let config = ResolvedFoundryConfig::new(workspace.clone(), default_profile);

    let mut host = AnalysisHost::new();
    let mut change = AnalysisChange::new();
    change.set_vfs(snapshot.clone());
    change.set_config(config);
    host.apply_change(change);

    (host.snapshot(), snapshot)
}

/// Sets up a Database instance with the given files and remappings.
///
/// This is used for lower-level tests that need direct database access.
pub fn setup_db<S: AsRef<str>>(
    files: Vec<(NormalizedPath, S)>,
    remappings: Vec<Remapping>,
) -> (Database, ProjectId, VfsSnapshot) {
    let mut vfs = Vfs::default();
    for (path, text) in &files {
        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from(text.as_ref()),
        });
    }
    let snapshot = vfs.snapshot();

    let mut db = Database::default();
    for (file_id, path) in snapshot.iter() {
        let text = snapshot.file_text(file_id).expect("file text should exist");
        let version = snapshot.file_version(file_id).unwrap_or(0);
        db.set_file(
            file_id,
            Arc::from(text),
            version,
            LanguageKind::Solidity,
            Arc::new(path.clone()),
        );
    }

    let root = NormalizedPath::new("/workspace");
    let default_profile = FoundryProfile::new("default").with_remappings(remappings);
    let workspace = FoundryWorkspace::new(root);
    let config = ResolvedFoundryConfig::new(workspace.clone(), default_profile);
    let project_id = ProjectId::from_raw(0);
    db.set_project_input(project_id, Arc::new(config));

    (db, project_id, snapshot)
}
