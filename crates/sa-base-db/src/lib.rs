use std::collections::HashMap;
use std::sync::Arc;

use salsa::Setter;

use sa_config::ResolvedFoundryConfig;
use sa_paths::NormalizedPath;
pub use sa_vfs::FileId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectId(u32);

impl ProjectId {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LanguageKind {
    Solidity,
    Toml,
    Json,
    Unknown,
}

#[salsa::input(debug)]
pub struct FileInput {
    #[returns(ref)]
    pub text: Arc<str>,
    pub version: u32,
    pub kind: LanguageKind,
}

#[salsa::input(debug)]
pub struct FilePathInput {
    #[returns(ref)]
    pub path: Arc<NormalizedPath>,
}

#[salsa::input(debug)]
pub struct ProjectInput {
    #[returns(ref)]
    pub workspace: Arc<sa_project_model::FoundryWorkspace>,
    #[returns(ref)]
    pub config: Arc<ResolvedFoundryConfig>,
}

#[derive(Default, Debug, Clone)]
struct InputStorage {
    files: HashMap<FileId, FileInput>,
    paths: HashMap<FileId, FilePathInput>,
    path_to_file_id: HashMap<NormalizedPath, FileId>,
    projects: HashMap<ProjectId, ProjectInput>,
}

impl InputStorage {
    fn file_input(&self, file_id: FileId) -> FileInput {
        self.files
            .get(&file_id)
            .copied()
            .unwrap_or_else(|| panic!("missing FileInput for {file_id:?}"))
    }

    fn file_path(&self, file_id: FileId) -> FilePathInput {
        self.paths
            .get(&file_id)
            .copied()
            .unwrap_or_else(|| panic!("missing FilePathInput for {file_id:?}"))
    }

    fn project_input(&self, project_id: ProjectId) -> ProjectInput {
        self.projects
            .get(&project_id)
            .copied()
            .unwrap_or_else(|| panic!("missing ProjectInput for {project_id:?}"))
    }

    fn project_input_opt(&self, project_id: ProjectId) -> Option<ProjectInput> {
        self.projects.get(&project_id).copied()
    }

    fn file_id_for_path(&self, path: &NormalizedPath) -> Option<FileId> {
        self.path_to_file_id.get(path).copied()
    }
}

#[salsa::db]
pub trait SaDatabase: salsa::Database {}

#[salsa::db]
impl<Db: salsa::Database> SaDatabase for Db {}

#[salsa::db]
#[derive(Default, Clone)]
pub struct Database {
    storage: salsa::Storage<Self>,
    inputs: InputStorage,
}

#[salsa::db]
impl salsa::Database for Database {}

impl Database {
    pub fn file_input(&self, file_id: FileId) -> FileInput {
        self.inputs.file_input(file_id)
    }

    pub fn set_file_input(
        &mut self,
        file_id: FileId,
        text: Arc<str>,
        version: u32,
        kind: LanguageKind,
    ) {
        let input = self.inputs.files.get(&file_id).copied();
        match input {
            Some(input) => {
                input.set_text(self).to(text);
                input.set_version(self).to(version);
                input.set_kind(self).to(kind);
            }
            None => {
                let input = FileInput::new(self, text, version, kind);
                self.inputs.files.insert(file_id, input);
            }
        }
    }

    pub fn set_file(
        &mut self,
        file_id: FileId,
        text: Arc<str>,
        version: u32,
        kind: LanguageKind,
        path: Arc<NormalizedPath>,
    ) {
        self.set_file_input(file_id, text, version, kind);
        self.set_file_path(file_id, path);
    }

    pub fn file_path(&self, file_id: FileId) -> Arc<NormalizedPath> {
        self.inputs.file_path(file_id).path(self).clone()
    }

    pub fn set_file_path(&mut self, file_id: FileId, path: Arc<NormalizedPath>) {
        let input = self.inputs.paths.get(&file_id).copied();
        match input {
            Some(input) => {
                let previous = input.path(self).clone();
                if previous.as_ref() != path.as_ref() {
                    self.inputs.path_to_file_id.remove(previous.as_ref());
                }
                input.set_path(self).to(path);
            }
            None => {
                let input = FilePathInput::new(self, path);
                self.inputs.paths.insert(file_id, input);
            }
        }
        let stored_path = self.inputs.paths.get(&file_id).expect("file path input");
        self.inputs
            .path_to_file_id
            .insert(stored_path.path(self).as_ref().clone(), file_id);
    }

    pub fn file_ids(&self) -> impl Iterator<Item = FileId> + '_ {
        self.inputs.files.keys().copied()
    }

    pub fn file_id_for_path(&self, path: &NormalizedPath) -> Option<FileId> {
        self.inputs.file_id_for_path(path)
    }

    pub fn project_input(&self, project_id: ProjectId) -> ProjectInput {
        self.inputs.project_input(project_id)
    }

    pub fn project_input_opt(&self, project_id: ProjectId) -> Option<ProjectInput> {
        self.inputs.project_input_opt(project_id)
    }

    pub fn set_project_input(&mut self, project_id: ProjectId, config: Arc<ResolvedFoundryConfig>) {
        let workspace = Arc::new(config.workspace().clone());
        let input = self.inputs.projects.get(&project_id).copied();
        match input {
            Some(input) => {
                input.set_workspace(self).to(Arc::clone(&workspace));
                input.set_config(self).to(config);
            }
            None => {
                let input = ProjectInput::new(self, workspace, config);
                self.inputs.projects.insert(project_id, input);
            }
        }
    }
}

pub trait SaDatabaseExt {
    fn file_input(&self, file_id: FileId) -> FileInput;
    fn file_path(&self, file_id: FileId) -> Arc<NormalizedPath>;
    fn file_ids(&self) -> Vec<FileId>;
    fn project_input(&self, project_id: ProjectId) -> ProjectInput;
}

impl SaDatabaseExt for Database {
    fn file_input(&self, file_id: FileId) -> FileInput {
        self.file_input(file_id)
    }

    fn file_path(&self, file_id: FileId) -> Arc<NormalizedPath> {
        self.file_path(file_id)
    }

    fn file_ids(&self) -> Vec<FileId> {
        self.file_ids().collect()
    }

    fn project_input(&self, project_id: ProjectId) -> ProjectInput {
        self.project_input(project_id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Database, FileId, LanguageKind, ProjectId};
    use sa_config::ResolvedFoundryConfig;
    use sa_paths::NormalizedPath;
    use sa_project_model::{FoundryProfile, FoundryWorkspace};

    fn path(value: &str) -> Arc<NormalizedPath> {
        Arc::new(NormalizedPath::new(value))
    }

    #[test]
    fn file_id_is_opaque_and_comparable() {
        let file_a = FileId::from_raw(0);
        let file_b = FileId::from_raw(1);

        assert!(file_a != file_b);
        assert!(file_a == file_a);
    }

    #[test]
    fn inputs_round_trip() {
        let mut db = Database::default();
        let file_id = FileId::from_raw(0);

        db.set_file_input(file_id, Arc::from("pragma"), 0, LanguageKind::Solidity);

        let input = db.file_input(file_id);
        assert_eq!(input.text(&db).as_ref(), "pragma");
        assert_eq!(input.version(&db), 0);
        assert_eq!(input.kind(&db), LanguageKind::Solidity);

        db.set_file_input(file_id, Arc::from("contract"), 1, LanguageKind::Unknown);

        let input = db.file_input(file_id);
        assert_eq!(input.text(&db).as_ref(), "contract");
        assert_eq!(input.version(&db), 1);
        assert_eq!(input.kind(&db), LanguageKind::Unknown);
    }

    #[test]
    fn project_input_round_trip() {
        let mut db = Database::default();
        let root = NormalizedPath::new("/workspace");
        let default_profile = FoundryProfile::new("default");
        let workspace = FoundryWorkspace::new(root);
        let config = Arc::new(ResolvedFoundryConfig::new(
            workspace.clone(),
            default_profile,
        ));
        let project_id = ProjectId::from_raw(0);

        db.set_project_input(project_id, Arc::clone(&config));

        let input = db.project_input(project_id);
        assert_eq!(input.workspace(&db).as_ref(), config.workspace());
        assert_eq!(input.config(&db), &config);
    }

    #[test]
    fn set_file_registers_path_mapping() {
        let mut db = Database::default();
        let file_id = FileId::from_raw(42);
        let file_path = path("/workspace/src/Main.sol");

        db.set_file(
            file_id,
            Arc::from(r#"contract Main {}"#),
            0,
            LanguageKind::Solidity,
            Arc::clone(&file_path),
        );

        assert_eq!(db.file_id_for_path(file_path.as_ref()), Some(file_id));
        assert_eq!(db.file_path(file_id).as_ref(), file_path.as_ref());
    }

    #[test]
    fn file_id_for_path_updates_when_path_changes() {
        let mut db = Database::default();
        let file_id = FileId::from_raw(7);
        let old_path = path("/workspace/src/Old.sol");
        let new_path = path("/workspace/src/New.sol");

        db.set_file_path(file_id, Arc::clone(&old_path));
        assert_eq!(db.file_id_for_path(old_path.as_ref()), Some(file_id));

        db.set_file_path(file_id, Arc::clone(&new_path));
        assert_eq!(db.file_id_for_path(old_path.as_ref()), None);
        assert_eq!(db.file_id_for_path(new_path.as_ref()), Some(file_id));
        assert_eq!(db.file_path(file_id).as_ref(), new_path.as_ref());
    }
}
