use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub workspace: String,
    pub repos: Vec<RepoConfig>,
    #[serde(skip)]
    pub config_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

impl RepoConfig {
    pub fn resolved_path(&self, workspace: &WorkspaceConfig) -> PathBuf {
        let path = Path::new(&self.path);
        if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(config_dir) = &workspace.config_dir {
            config_dir.join(path)
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RepoAvailability {
    #[serde(rename = "present")]
    Present,
    #[serde(rename = "missing")]
    Missing,
    #[serde(rename = "not_git_repo")]
    NotGitRepo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStatus {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_dirty: bool,
    pub status: RepoAvailability,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<usize>,
}
