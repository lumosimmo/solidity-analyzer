use std::collections::HashMap;
use std::sync::Arc;

use sa_paths::NormalizedPath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(u32);

impl FileId {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Debug)]
struct FileEntry {
    text: Arc<str>,
    version: u32,
}

#[derive(Debug)]
pub enum VfsChange {
    Set {
        path: NormalizedPath,
        text: Arc<str>,
    },
    Remove {
        path: NormalizedPath,
    },
}

#[derive(Default, Debug)]
pub struct Vfs {
    next_file_id: u32,
    path_to_id: HashMap<NormalizedPath, FileId>,
    id_to_path: HashMap<FileId, NormalizedPath>,
    files: HashMap<FileId, FileEntry>,
}

impl Vfs {
    pub fn apply_change(&mut self, change: VfsChange) {
        match change {
            VfsChange::Set { path, text } => {
                let file_id = self
                    .path_to_id
                    .get(&path)
                    .copied()
                    .unwrap_or_else(|| self.alloc_file_id(path.clone()));
                self.upsert_file(file_id, path, text);
            }
            VfsChange::Remove { path } => {
                if let Some(file_id) = self.path_to_id.remove(&path) {
                    self.id_to_path.remove(&file_id);
                    self.files.remove(&file_id);
                }
            }
        }
    }

    pub fn apply_changes(&mut self, changes: Vec<VfsChange>) {
        for change in changes {
            self.apply_change(change);
        }
    }

    pub fn snapshot(&self) -> VfsSnapshot {
        VfsSnapshot {
            path_to_id: self.path_to_id.clone(),
            id_to_path: self.id_to_path.clone(),
            files: self.files.clone(),
        }
    }

    fn alloc_file_id(&mut self, path: NormalizedPath) -> FileId {
        let file_id = FileId::from_raw(self.next_file_id);
        self.next_file_id = self.next_file_id.saturating_add(1);
        self.path_to_id.insert(path.clone(), file_id);
        self.id_to_path.insert(file_id, path);
        file_id
    }

    fn upsert_file(&mut self, file_id: FileId, path: NormalizedPath, text: Arc<str>) {
        let version = self
            .files
            .get(&file_id)
            .map(|entry| entry.version.saturating_add(1))
            .unwrap_or(0);

        self.path_to_id.insert(path.clone(), file_id);
        self.id_to_path.insert(file_id, path);
        self.files.insert(file_id, FileEntry { text, version });
    }
}

#[derive(Clone, Debug)]
pub struct VfsSnapshot {
    path_to_id: HashMap<NormalizedPath, FileId>,
    id_to_path: HashMap<FileId, NormalizedPath>,
    files: HashMap<FileId, FileEntry>,
}

impl VfsSnapshot {
    pub fn file_id(&self, path: &NormalizedPath) -> Option<FileId> {
        self.path_to_id.get(path).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (FileId, &NormalizedPath)> {
        self.id_to_path
            .iter()
            .map(|(file_id, path)| (*file_id, path))
    }

    pub fn path(&self, file_id: FileId) -> Option<&NormalizedPath> {
        self.id_to_path.get(&file_id)
    }

    pub fn file_text(&self, file_id: FileId) -> Option<&str> {
        self.files.get(&file_id).map(|entry| entry.text.as_ref())
    }

    pub fn file_version(&self, file_id: FileId) -> Option<u32> {
        self.files.get(&file_id).map(|entry| entry.version)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Vfs, VfsChange};
    use sa_paths::NormalizedPath;

    fn path(value: &str) -> NormalizedPath {
        NormalizedPath::new(value)
    }

    #[test]
    fn snapshot_returns_contents_by_file_id() {
        let mut vfs = Vfs::default();
        vfs.apply_changes(vec![
            VfsChange::Set {
                path: path("/workspace/src/A.sol"),
                text: Arc::from("contract A {}"),
            },
            VfsChange::Set {
                path: path("/workspace/src/B.sol"),
                text: Arc::from("contract B {}"),
            },
        ]);

        let snapshot = vfs.snapshot();
        let a_id = snapshot
            .file_id(&path("/workspace/src/A.sol"))
            .expect("file id");
        let b_id = snapshot
            .file_id(&path("/workspace/src/B.sol"))
            .expect("file id");

        assert_ne!(a_id, b_id);
        assert_eq!(snapshot.file_text(a_id), Some("contract A {}"));
        assert_eq!(snapshot.file_text(b_id), Some("contract B {}"));
    }

    #[test]
    fn versions_increment_on_change() {
        let mut vfs = Vfs::default();
        let path = path("/workspace/src/C.sol");

        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from("contract C {}"),
        });
        let snapshot = vfs.snapshot();
        let file_id = snapshot.file_id(&path).expect("file id");
        assert_eq!(snapshot.file_version(file_id), Some(0));

        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from("contract C { uint x; }"),
        });
        let snapshot = vfs.snapshot();
        let file_id = snapshot.file_id(&path).expect("file id");
        assert_eq!(snapshot.file_version(file_id), Some(1));
    }

    #[test]
    fn snapshots_are_immutable() {
        let mut vfs = Vfs::default();
        let path = path("/workspace/src/D.sol");

        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from("contract D {}"),
        });
        let snapshot_before = vfs.snapshot();
        let file_id = snapshot_before.file_id(&path).expect("file id");
        assert_eq!(snapshot_before.file_text(file_id), Some("contract D {}"));
        assert_eq!(snapshot_before.file_version(file_id), Some(0));

        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from("contract D { uint y; }"),
        });
        let snapshot_after = vfs.snapshot();

        assert_eq!(snapshot_before.file_text(file_id), Some("contract D {}"));
        assert_eq!(snapshot_before.file_version(file_id), Some(0));
        assert_eq!(
            snapshot_after.file_text(file_id),
            Some("contract D { uint y; }")
        );
        assert_eq!(snapshot_after.file_version(file_id), Some(1));
    }

    #[test]
    fn remove_clears_mappings_and_files() {
        let mut vfs = Vfs::default();
        let path = path("/workspace/src/E.sol");

        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from("contract E {}"),
        });
        let snapshot = vfs.snapshot();
        let file_id = snapshot.file_id(&path).expect("file id");

        vfs.apply_change(VfsChange::Remove { path: path.clone() });
        let snapshot = vfs.snapshot();

        assert_eq!(snapshot.file_id(&path), None);
        assert_eq!(snapshot.path(file_id), None);
        assert_eq!(snapshot.file_text(file_id), None);
    }

    #[test]
    fn readd_after_remove_allocates_new_id_and_resets_version() {
        let mut vfs = Vfs::default();
        let path = path("/workspace/src/F.sol");

        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from("contract F {}"),
        });
        let snapshot = vfs.snapshot();
        let old_id = snapshot.file_id(&path).expect("file id");
        assert_eq!(snapshot.file_version(old_id), Some(0));

        vfs.apply_change(VfsChange::Remove { path: path.clone() });
        vfs.apply_change(VfsChange::Set {
            path: path.clone(),
            text: Arc::from("contract F { uint x; }"),
        });
        let snapshot = vfs.snapshot();
        let new_id = snapshot.file_id(&path).expect("file id");

        assert_ne!(old_id, new_id);
        assert_eq!(snapshot.file_text(new_id), Some("contract F { uint x; }"));
        assert_eq!(snapshot.file_version(new_id), Some(0));
    }
}
