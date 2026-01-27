use std::borrow::Cow;
use std::path::{Path, PathBuf};

use foundry_compilers::utils::canonicalize;
use foundry_config::Config;
use foundry_config::utils::find_project_root;
use sa_paths::NormalizedPath;
use tower_lsp::lsp_types::Url;

pub fn url_to_path(uri: &Url) -> Option<NormalizedPath> {
    let path = uri.to_file_path().ok()?;
    Some(normalize_path(&path))
}

pub fn path_to_url(path: &NormalizedPath) -> Option<Url> {
    let cleaned = strip_verbatim_prefix(path.as_str());
    Url::from_file_path(PathBuf::from(cleaned.as_ref())).ok()
}

pub fn is_foundry_config_path(path: &NormalizedPath) -> bool {
    let file_name = Path::new(path.as_str())
        .file_name()
        .and_then(|name| name.to_str());
    matches!(file_name, Some("foundry.toml" | "remappings.txt"))
}

pub fn normalize_path(path: &Path) -> NormalizedPath {
    let canonical = canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    NormalizedPath::new(canonical.to_string_lossy())
}

fn strip_verbatim_prefix(path: &str) -> Cow<'_, str> {
    let rest = match path.strip_prefix("//?/") {
        Some(rest) => rest,
        None => match path.strip_prefix("//./") {
            Some(rest) => rest,
            None => return Cow::Borrowed(path),
        },
    };
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    if rest
        .get(0..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("unc/"))
    {
        return Cow::Owned(format!("//{}", &rest[4..]));
    }
    Cow::Borrowed(rest)
}

pub fn contains_foundry_config(path: &Path) -> bool {
    path.join(Config::FILE_NAME).is_file()
}

pub fn find_foundry_root(path: &Path) -> Option<NormalizedPath> {
    let start = if path.is_dir() { path } else { path.parent()? };
    let root = find_project_root(Some(start)).ok()?;
    contains_foundry_config(&root).then(|| normalize_path(&root))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        contains_foundry_config, find_foundry_root, is_foundry_config_path, normalize_path,
        path_to_url, url_to_path,
    };
    use sa_paths::NormalizedPath;
    use tower_lsp::lsp_types::Url;

    #[test]
    fn find_foundry_root_finds_nearest_config() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");
        let nested = root.join("project");
        let deeper = nested.join("src/contracts");
        fs::create_dir_all(&deeper).expect("create dirs");

        fs::write(root.join("foundry.toml"), "[profile.default]").expect("write foundry.toml");
        fs::write(nested.join("foundry.toml"), "[profile.default]").expect("write foundry.toml");

        let target_file = deeper.join("Main.sol");
        fs::write(&target_file, "contract Main {}").expect("write main");

        let found = find_foundry_root(&target_file).expect("found root");
        assert_eq!(found.as_str(), nested.to_string_lossy());
    }

    #[test]
    fn find_foundry_root_requires_foundry_toml() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");
        let nested = root.join("project/src");
        fs::create_dir_all(&nested).expect("create dirs");

        fs::write(root.join("remappings.txt"), "lib/=lib/").expect("write remappings");

        assert!(find_foundry_root(&nested).is_none());
    }

    #[test]
    fn find_foundry_root_returns_none_without_config() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");
        let nested = root.join("project/src");
        fs::create_dir_all(&nested).expect("create dirs");

        assert!(find_foundry_root(&nested).is_none());
    }

    #[test]
    fn is_foundry_config_path_matches_expected_files() {
        assert!(is_foundry_config_path(&NormalizedPath::new(
            "/workspace/foundry.toml"
        )));
        assert!(is_foundry_config_path(&NormalizedPath::new(
            "/workspace/remappings.txt"
        )));
        assert!(!is_foundry_config_path(&NormalizedPath::new(
            "/workspace/foundry.toml.bak"
        )));
        assert!(!is_foundry_config_path(&NormalizedPath::new(
            "/workspace/foundry.yaml"
        )));
    }

    #[test]
    fn contains_foundry_config_requires_foundry_toml() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");
        fs::write(root.join("foundry.toml"), "[profile.default]").expect("write foundry.toml");

        assert!(contains_foundry_config(&root));
    }

    #[test]
    fn contains_foundry_config_ignores_remappings_only() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");
        fs::write(root.join("remappings.txt"), "lib/=lib/").expect("write remappings");

        assert!(!contains_foundry_config(&root));
    }

    #[test]
    fn url_to_path_normalizes_dot_segments() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");
        let input = root.join("a/b/../c/./Main.sol");
        let url = Url::from_file_path(&input).expect("file url");

        let normalized = url_to_path(&url).expect("normalized path");
        let expected = NormalizedPath::new(root.join("a/c/Main.sol").to_string_lossy());

        assert_eq!(normalized.as_str(), expected.as_str());
    }

    #[test]
    fn normalize_path_falls_back_without_fs() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");
        let missing = root.join("missing/../virtual.sol");

        let normalized = normalize_path(&missing);
        let expected = NormalizedPath::new(root.join("virtual.sol").to_string_lossy());

        assert_eq!(normalized.as_str(), expected.as_str());
    }

    #[test]
    fn path_to_url_round_trips_existing_path() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().canonicalize().expect("canonicalize root");
        let file_path = root.join("src/Main.sol");
        fs::create_dir_all(file_path.parent().expect("parent")).expect("create dirs");
        fs::write(&file_path, "contract Main {}").expect("write file");

        let normalized = normalize_path(&file_path);
        let url = path_to_url(&normalized).expect("file url");
        let round_trip = url_to_path(&url).expect("round trip path");

        assert_eq!(round_trip.as_str(), normalized.as_str());
    }
}
