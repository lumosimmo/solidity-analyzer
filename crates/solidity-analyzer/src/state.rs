use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::config::LspConfig;
use crate::lsp_utils;
use futures::future::AbortHandle;
use sa_config::ResolvedFoundryConfig;
use sa_ide::AnalysisHost;
use sa_paths::NormalizedPath;
use sa_vfs::{Vfs, VfsSnapshot};

#[derive(Debug, Clone, Copy)]
pub struct OpenDocument {
    pub version: i32,
}

pub struct ServerState {
    pub(crate) analysis_host: AnalysisHost,
    pub(crate) vfs: Vfs,
    pub(crate) vfs_snapshot: Option<VfsSnapshot>,
    pub(crate) open_documents: HashMap<NormalizedPath, OpenDocument>,
    pub(crate) indexed_files: HashSet<NormalizedPath>,
    pub(crate) foundry_root_cache: HashMap<NormalizedPath, Option<NormalizedPath>>,
    pub(crate) config: Option<ResolvedFoundryConfig>,
    pub(crate) lsp_config: LspConfig,
    pub(crate) supports_server_status: bool,
    pub(crate) root_path: Option<NormalizedPath>,
    pub(crate) prompted_solc_install: bool,
    pub(crate) format_tasks: FormatTaskState,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            analysis_host: AnalysisHost::new(),
            vfs: Vfs::default(),
            vfs_snapshot: None,
            open_documents: HashMap::new(),
            indexed_files: HashSet::new(),
            foundry_root_cache: HashMap::new(),
            config: None,
            lsp_config: LspConfig::default(),
            supports_server_status: false,
            root_path: None,
            prompted_solc_install: false,
            format_tasks: FormatTaskState::default(),
        }
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerState {
    pub(crate) fn discover_foundry_root(
        &mut self,
        path: &NormalizedPath,
    ) -> Option<NormalizedPath> {
        let start = Path::new(path.as_str());
        let start_dir = if start.is_dir() {
            start
        } else {
            start.parent()?
        };
        let start_dir = lsp_utils::normalize_path(start_dir);
        if let Some(cached) = self.foundry_root_cache.get(&start_dir) {
            return cached.clone();
        }
        let found = lsp_utils::find_foundry_root(Path::new(start_dir.as_str()));
        self.foundry_root_cache.insert(start_dir, found.clone());
        found
    }
}

#[derive(Default)]
pub(crate) struct FormatTaskState {
    tasks: HashMap<NormalizedPath, FormatTask>,
    next_generation: u64,
}

impl FormatTaskState {
    pub(crate) fn register(&mut self, path: NormalizedPath, handle: AbortHandle) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        if let Some(existing) = self.tasks.insert(path, FormatTask { generation, handle }) {
            existing.handle.abort();
        }
        generation
    }

    pub(crate) fn is_current(&self, path: &NormalizedPath, generation: u64) -> bool {
        self.tasks
            .get(path)
            .is_some_and(|task| task.generation == generation)
    }

    pub(crate) fn finish(&mut self, path: &NormalizedPath, generation: u64) {
        if self.is_current(path, generation) {
            self.tasks.remove(path);
        }
    }
}

struct FormatTask {
    generation: u64,
    // Retained so register() can abort a previously tracked task.
    handle: AbortHandle,
}
