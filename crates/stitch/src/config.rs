use std::path::{Path, PathBuf};

use crate::model::WorkspaceConfig;

const WORKSPACE_ROOT_ENV: &str = "STITCH_WORKSPACE_ROOT";
const WORKSPACE_MARKER: &str = ".stitch-workspace";

/// Resolve the workspace root and load the canonical discovered configuration.
///
/// Resolution order is explicit environment, an ancestor workspace marker or
/// `repos/` directory, then the current Git repository as a single-repository
/// workspace. Committed `.stitch.json` inventory is deliberately not an
/// authority.
pub fn find_and_load() -> Result<WorkspaceConfig, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;

    if let Some(root) = std::env::var_os(WORKSPACE_ROOT_ENV).map(PathBuf::from) {
        return load_workspace_root(&root).map_err(|error| {
            format!(
                "Cannot load workspace from {WORKSPACE_ROOT_ENV}={}: {error}",
                root.display()
            )
        });
    }

    if let Some(root) = find_workspace_root(&cwd) {
        return load_workspace_root(&root);
    }

    let git_root = git_repository_root(&cwd).unwrap_or(cwd);
    load_workspace_root(&git_root)
}

pub fn load_workspace_root(root: &Path) -> Result<WorkspaceConfig, String> {
    if !root.exists() {
        return Err(format!("Workspace root does not exist: {}", root.display()));
    }
    crate::workspace::load_workspace_config(root)
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|directory| {
            directory.join(WORKSPACE_MARKER).exists() || directory.join("repos").is_dir()
        })
        .map(Path::to_path_buf)
}

fn git_repository_root(start: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_workspace_container_wins() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let member = workspace.join("repos/member/src");
        std::fs::create_dir_all(&member).unwrap();

        assert_eq!(find_workspace_root(&member), Some(workspace));
    }

    #[test]
    fn explicit_marker_defines_workspace_without_repos_directory() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let nested = workspace.join("nested/member");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(workspace.join(WORKSPACE_MARKER), "").unwrap();

        assert_eq!(find_workspace_root(&nested), Some(workspace));
    }
}
