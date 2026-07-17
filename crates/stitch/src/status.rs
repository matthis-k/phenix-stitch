use crate::git;
use crate::model::{RepoAvailability, RepoStatus, WorkspaceConfig};

pub fn collect_all(config: &WorkspaceConfig) -> Result<Vec<RepoStatus>, String> {
    let mut statuses = Vec::new();
    for repository in &config.repos {
        let path = repository.resolved_path(config);
        if !path.exists() {
            statuses.push(RepoStatus {
                name: repository.name.clone(),
                path: repository.path.clone(),
                branch: String::new(),
                is_dirty: false,
                status: RepoAvailability::Missing,
                staged_count: 0,
                unstaged_count: 0,
                untracked_count: 0,
                ahead: None,
                behind: None,
            });
            continue;
        }
        if !path.join(".git").exists() {
            statuses.push(RepoStatus {
                name: repository.name.clone(),
                path: repository.path.clone(),
                branch: String::new(),
                is_dirty: false,
                status: RepoAvailability::NotGitRepo,
                staged_count: 0,
                unstaged_count: 0,
                untracked_count: 0,
                ahead: None,
                behind: None,
            });
            continue;
        }
        statuses.push(git::get_status(&repository.name, &path)?);
    }
    Ok(statuses)
}
