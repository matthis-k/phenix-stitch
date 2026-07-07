use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::graph::parse_flake_lock;
use crate::model::{RepoConfig, WorkspaceConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceMode {
    Locked,
    Workspace,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub version: u32,
    #[serde(default)]
    pub repos: BTreeMap<String, WorkspaceRepoState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRepoState {
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

pub fn load_workspace_config(root: &Path) -> Result<WorkspaceConfig, String> {
    let lock_path = root.join("flake.lock");
    let lock = parse_flake_lock(&lock_path).map_err(|e| format!("Read locked workspace: {e}"))?;
    let workspace = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();
    let state = load_state(&workspace).ok();

    let root_lock_node = lock.root.as_deref().unwrap_or("root");
    let root_inputs = lock
        .nodes
        .get(root_lock_node)
        .and_then(|n| n.inputs.as_ref())
        .ok_or_else(|| format!("{} has no root inputs", lock_path.display()))?;

    let mut repos = vec![RepoConfig {
        name: workspace.clone(),
        path: ".".to_string(),
        remote: git_remote_url(root),
    }];

    for (input_name, input_value) in root_inputs {
        let Some(target) = crate::graph::lock::input_target_name(input_value) else {
            continue;
        };
        let Some(node) = lock.nodes.get(&target) else {
            continue;
        };
        let Some(locked) = &node.locked else {
            continue;
        };
        if locked.kind.as_deref() != Some("github") {
            continue;
        }
        let Some(owner) = locked.owner.as_deref() else {
            continue;
        };
        let Some(repo) = locked.repo.as_deref() else {
            continue;
        };
        if owner != "matthis-k" || !repo.starts_with("phenix-") {
            continue;
        }
        let path = state
            .as_ref()
            .and_then(|s| s.repos.get(repo))
            .map(|r| r.path.clone())
            .unwrap_or_else(|| discover_local_repo_path(root, repo));
        repos.push(RepoConfig {
            name: input_name.clone(),
            path: path_to_config(root, &path),
            remote: Some(format!("github:{owner}/{repo}")),
        });
    }

    repos.sort_by(|a, b| a.name.cmp(&b.name));
    repos.dedup_by(|a, b| a.name == b.name);
    Ok(WorkspaceConfig {
        version: 1,
        workspace,
        repos,
        config_dir: Some(root.to_path_buf()),
    })
}

pub fn state_file(workspace: &str) -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from(".stitch-state"))
        .join("stitch")
        .join(format!("{workspace}.json"))
}

pub fn load_state(workspace: &str) -> Result<WorkspaceState, String> {
    let path = state_file(workspace);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Read workspace state {}: {e}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Parse workspace state {}: {e}", path.display()))
}

pub fn git_remote_url(repo: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!url.is_empty()).then_some(url)
}

fn discover_local_repo_path(root: &Path, repo: &str) -> PathBuf {
    for candidate in [
        root.join("repos").join(repo),
        root.join("flakes/00-pins").join(repo),
        root.join("flakes/02-producers").join(repo),
        root.join("flakes/03-integrations").join(repo),
        root.join("flakes/04-pkgs").join(repo),
        root.join("flakes/05-consumers").join(repo),
    ] {
        if candidate.join(".git").exists() || candidate.join("flake.nix").exists() {
            return candidate;
        }
    }
    root.join("repos").join(repo)
}

fn path_to_config(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_file_uses_workspace_name() {
        let path = state_file("demo");
        assert!(path.ends_with("stitch/demo.json"));
    }
}
