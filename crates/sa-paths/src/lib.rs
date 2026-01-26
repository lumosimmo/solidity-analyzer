use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NormalizedPath {
    inner: String,
}

impl NormalizedPath {
    pub fn new(path: impl AsRef<str>) -> Self {
        let mut path = path.as_ref().replace('\\', "/");
        let mut extended_prefix = "";

        if let Some(rest) = path.strip_prefix("//?/") {
            extended_prefix = "//?/";
            path = rest.to_string();
        } else if let Some(rest) = path.strip_prefix("//./") {
            extended_prefix = "//./";
            path = rest.to_string();
        }

        let mut prefix = String::new();
        let mut absolute = false;
        let mut add_separator_after_prefix = false;

        if path.starts_with("//") {
            prefix.push_str("//");
            path = path.trim_start_matches('/').to_string();
            absolute = true;
        } else if path.len() >= 2 && path.as_bytes()[1] == b':' {
            prefix.push_str(&path[..2]);
            path = path[2..].to_string();
            if path.starts_with('/') {
                absolute = true;
                path = path.trim_start_matches('/').to_string();
                add_separator_after_prefix = true;
            }
        } else if path.starts_with('/') {
            absolute = true;
            path = path.trim_start_matches('/').to_string();
        }

        if !extended_prefix.is_empty() {
            absolute = true;
            prefix = format!("{extended_prefix}{prefix}");
            if prefix == extended_prefix {
                add_separator_after_prefix = false;
            }
        }

        let mut components: Vec<&str> = Vec::new();
        for part in path.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                if let Some(last) = components.pop() {
                    if last == ".." {
                        components.push(last);
                        components.push("..");
                    }
                } else if !absolute {
                    components.push("..");
                }
                continue;
            }
            components.push(part);
        }

        let mut normalized = String::new();
        if !prefix.is_empty() {
            normalized.push_str(&prefix);
            if absolute && add_separator_after_prefix {
                normalized.push('/');
            }
        } else if absolute {
            normalized.push('/');
        }

        normalized.push_str(&components.join("/"));

        let is_root_with_prefix = !prefix.is_empty() && absolute && components.is_empty();
        if normalized.len() > 1 && normalized.ends_with('/') && !is_root_with_prefix {
            normalized.pop();
        }

        if normalized.is_empty() {
            normalized.push('.');
        }

        if cfg!(windows) {
            normalized.make_ascii_lowercase();
        }

        Self { inner: normalized }
    }

    pub fn as_str(&self) -> &str {
        &self.inner
    }

    fn starts_with(&self, root: &NormalizedPath) -> bool {
        if root.inner == "/" {
            return self.inner.starts_with('/');
        }
        if self.inner == root.inner {
            return true;
        }
        self.inner
            .strip_prefix(&root.inner)
            .is_some_and(|rest| rest.starts_with('/'))
    }

    fn strip_prefix<'a>(&'a self, root: &NormalizedPath) -> Option<&'a str> {
        if root.inner == "/" {
            return self.inner.strip_prefix('/');
        }
        if self.inner == root.inner {
            return Some("");
        }
        self.inner
            .strip_prefix(&root.inner)
            .and_then(|rest| rest.strip_prefix('/'))
    }
}

impl fmt::Display for NormalizedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkspacePath {
    inner: String,
}

impl WorkspacePath {
    pub fn new(root: &NormalizedPath, path: &NormalizedPath) -> Option<Self> {
        if !path.starts_with(root) {
            return None;
        }
        let relative = path.strip_prefix(root).unwrap_or_default();
        let inner = if relative.is_empty() { "." } else { relative };
        Some(Self {
            inner: inner.to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.inner
    }
}

impl fmt::Display for WorkspacePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{NormalizedPath, WorkspacePath};

    #[test]
    fn normalizes_separators_and_components() {
        let path = NormalizedPath::new("/workspace//src/./contracts/../lib.sol");
        assert_eq!(path.as_str(), "/workspace/src/lib.sol");

        let mixed = NormalizedPath::new("/workspace\\src/./contracts\\../lib.sol");
        assert_eq!(mixed.as_str(), "/workspace/src/lib.sol");

        let windows_path = NormalizedPath::new("C:\\workspace\\src\\lib.sol");
        let expected = if cfg!(windows) {
            "c:/workspace/src/lib.sol"
        } else {
            "C:/workspace/src/lib.sol"
        };
        assert_eq!(windows_path.as_str(), expected);
    }

    #[test]
    fn preserves_windows_roots_and_unc_paths() {
        let root = NormalizedPath::new("C:\\");
        let expected_root = if cfg!(windows) { "c:/" } else { "C:/" };
        assert_eq!(root.as_str(), expected_root);

        let unc = NormalizedPath::new(r"\\server\share\dir");
        assert_eq!(unc.as_str(), "//server/share/dir");

        let extended = NormalizedPath::new(r"\\?\C:\Repo\src");
        let expected_extended = if cfg!(windows) {
            "//?/c:/repo/src"
        } else {
            "//?/C:/Repo/src"
        };
        assert_eq!(extended.as_str(), expected_extended);

        let extended_unc = NormalizedPath::new(r"\\?\UNC\server\share\dir");
        assert_eq!(extended_unc.as_str(), "//?/UNC/server/share/dir");
    }

    #[test]
    fn normalization_is_stable_for_hashing() {
        let a = NormalizedPath::new("/workspace/src/lib.sol");
        let b = NormalizedPath::new("/workspace/src/./lib.sol");
        let c = NormalizedPath::new("/workspace/src\\lib.sol");

        let mut set = HashSet::new();
        set.insert(a);
        set.insert(b);
        set.insert(c);

        assert_eq!(set.len(), 1);
    }

    #[test]
    fn workspace_relative_paths_are_derived_from_roots() {
        let root = NormalizedPath::new("/workspace");
        let file = NormalizedPath::new("/workspace/src/lib.sol");

        let relative = WorkspacePath::new(&root, &file).expect("relative path");
        assert_eq!(relative.as_str(), "src/lib.sol");

        let outside = NormalizedPath::new("/other/src/lib.sol");
        assert!(WorkspacePath::new(&root, &outside).is_none());
    }
}
