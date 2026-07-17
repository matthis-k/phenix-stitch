use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::git::GitRepo;
use crate::graph::parse_flake_lock;
use crate::model::RepoConfig;
use crate::workspace::{
    git_remote_url, parse_remote_identity, state_file, WorkspaceDiscoveryPolicy,
};

pub const WORKSPACE_POLICY_FILE: &str = ".stitch-workspace.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceActionKind {
    Present,
    Clone,
    Remove,
    Forget,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAction {
    pub repository: String,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    pub action: WorkspaceActionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMutationReport {
    pub workspace: String,
    pub applied: bool,
    pub changed: usize,
    pub blocked: usize,
    pub actions: Vec<WorkspaceAction>,
}

impl WorkspaceMutationReport {
    fn new(workspace: String, applied: bool) -> Self {
        Self {
            workspace,
            applied,
            changed: 0,
            blocked: 0,
            actions: Vec::new(),
        }
    }

    fn push(&mut self, action: WorkspaceAction) {
        match action.action {
            WorkspaceActionKind::Clone
            | WorkspaceActionKind::Remove
            | WorkspaceActionKind::Forget => self.changed += 1,
            WorkspaceActionKind::Blocked => self.blocked += 1,
            WorkspaceActionKind::Present => {}
        }
        self.actions.push(action);
    }

    fn append(&mut self, mut other: Self) {
        self.changed += other.changed;
        self.blocked += other.blocked;
        self.actions.append(&mut other.actions);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedWorkspaceState {
    #[serde(default)]
    repos: BTreeMap<String, ManagedRepository>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedRepository {
    path: PathBuf,
    remote: String,
}

pub fn load_policy(root: &Path) -> Result<Option<WorkspaceDiscoveryPolicy>, String> {
    let path = root.join(WORKSPACE_POLICY_FILE);
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("Read workspace policy {}: {error}", path.display()))?;
    let policy = serde_json::from_str::<WorkspaceDiscoveryPolicy>(&content)
        .map_err(|error| format!("Parse workspace policy {}: {error}", path.display()))?;
    validate_management_policy(&policy)?;
    Ok(Some(policy))
}

pub fn resolve_management_policy(root: &Path) -> Result<WorkspaceDiscoveryPolicy, String> {
    if let Some(policy) = load_policy(root)? {
        return Ok(policy);
    }

    let owner = git_remote_url(root)
        .as_deref()
        .and_then(parse_remote_identity)
        .map(|identity| identity.owner)
        .ok_or_else(|| {
            format!(
                "Cannot infer workspace owner from {}; add {}",
                root.display(),
                WORKSPACE_POLICY_FILE
            )
        })?;

    Ok(WorkspaceDiscoveryPolicy {
        owner: Some(owner),
        repository_pattern: "*".to_string(),
        search_roots: vec![PathBuf::from("repos")],
    })
}

pub fn locked_workspace_repositories(
    root: &Path,
    policy: &WorkspaceDiscoveryPolicy,
) -> Result<Vec<RepoConfig>, String> {
    validate_management_policy(policy)?;
    let lock_path = root.join("flake.lock");
    let lock = parse_flake_lock(&lock_path)
        .map_err(|error| format!("Read workspace lock {}: {error}", lock_path.display()))?;

    let mut repositories = BTreeMap::<String, RepoConfig>::new();
    for node in lock.nodes.values() {
        let Some(locked) = node.locked.as_ref() else {
            continue;
        };
        if locked.kind.as_deref() != Some("github") {
            continue;
        }
        let (Some(owner), Some(repository)) = (locked.owner.as_deref(), locked.repo.as_deref())
        else {
            continue;
        };
        if !policy_matches(policy, owner, repository) {
            continue;
        }

        let path = existing_or_default_path(root, policy, repository);
        repositories.insert(
            repository.to_string(),
            RepoConfig {
                name: repository.to_string(),
                path: path_to_config(root, &path),
                remote: Some(format!("github:{owner}/{repository}")),
            },
        );
    }

    Ok(repositories.into_values().collect())
}

pub fn populate_workspace(root: &Path, apply: bool) -> Result<WorkspaceMutationReport, String> {
    let root = canonical_or_owned(root);
    let workspace = workspace_name(&root);
    let policy = resolve_management_policy(&root)?;
    let repositories = locked_workspace_repositories(&root, &policy)?;
    let mut state = load_managed_state(&workspace)?;
    let mut report = WorkspaceMutationReport::new(workspace.clone(), apply);

    for repository in repositories {
        let path = resolve_repo_path(&root, &repository.path);
        let remote = repository.remote.clone();

        if path.join(".git").is_dir() {
            report.push(WorkspaceAction {
                repository: repository.name,
                path,
                remote,
                action: WorkspaceActionKind::Present,
                reason: None,
            });
            continue;
        }

        if path.exists() {
            report.push(WorkspaceAction {
                repository: repository.name,
                path,
                remote,
                action: WorkspaceActionKind::Blocked,
                reason: Some("target exists but is not a Git repository".to_string()),
            });
            continue;
        }

        let Some(remote) = remote else {
            report.push(WorkspaceAction {
                repository: repository.name,
                path,
                remote: None,
                action: WorkspaceActionKind::Blocked,
                reason: Some("locked repository has no clone remote".to_string()),
            });
            continue;
        };

        report.push(WorkspaceAction {
            repository: repository.name.clone(),
            path: path.clone(),
            remote: Some(remote.clone()),
            action: WorkspaceActionKind::Clone,
            reason: (!apply).then(|| "dry run".to_string()),
        });

        if apply {
            clone_repository(&remote, &path)?;
            state.repos.insert(
                repository.name,
                ManagedRepository {
                    path: path_to_config(&root, &path).into(),
                    remote,
                },
            );
            save_managed_state(&workspace, &state)?;
        }
    }

    Ok(report)
}

pub fn clean_workspace(
    root: &Path,
    apply: bool,
    force: bool,
) -> Result<WorkspaceMutationReport, String> {
    let root = canonical_or_owned(root);
    let workspace = workspace_name(&root);
    let policy = resolve_management_policy(&root)?;
    let desired = locked_workspace_repositories(&root, &policy)?
        .into_iter()
        .map(|repository| repository.name)
        .collect::<BTreeSet<_>>();
    let mut state = load_managed_state(&workspace)?;
    let mut report = WorkspaceMutationReport::new(workspace.clone(), apply);

    let managed_names = state.repos.keys().cloned().collect::<Vec<_>>();
    for name in managed_names {
        if desired.contains(&name) {
            continue;
        }

        let Some(managed) = state.repos.get(&name).cloned() else {
            continue;
        };
        let path = resolve_repo_path(&root, &managed.path);

        if !path.exists() {
            report.push(WorkspaceAction {
                repository: name.clone(),
                path,
                remote: Some(managed.remote),
                action: WorkspaceActionKind::Forget,
                reason: (!apply).then(|| "managed path is already absent".to_string()),
            });
            if apply {
                state.repos.remove(&name);
                save_managed_state(&workspace, &state)?;
            }
            continue;
        }

        if let Some(reason) = removal_blocker(&root, &policy, &path, &managed.remote, force)? {
            report.push(WorkspaceAction {
                repository: name,
                path,
                remote: Some(managed.remote),
                action: WorkspaceActionKind::Blocked,
                reason: Some(reason),
            });
            continue;
        }

        report.push(WorkspaceAction {
            repository: name.clone(),
            path: path.clone(),
            remote: Some(managed.remote),
            action: WorkspaceActionKind::Remove,
            reason: (!apply).then(|| "dry run".to_string()),
        });
        if apply {
            std::fs::remove_dir_all(&path)
                .map_err(|error| format!("Remove managed repository {}: {error}", path.display()))?;
            state.repos.remove(&name);
            save_managed_state(&workspace, &state)?;
        }
    }

    Ok(report)
}

pub fn sync_workspace(
    root: &Path,
    apply: bool,
    prune: bool,
    force: bool,
) -> Result<WorkspaceMutationReport, String> {
    let mut report = populate_workspace(root, apply)?;
    if prune {
        report.append(clean_workspace(root, apply, force)?);
    }
    Ok(report)
}

fn validate_management_policy(policy: &WorkspaceDiscoveryPolicy) -> Result<(), String> {
    if policy.owner.as_deref().is_none_or(str::is_empty) {
        return Err("Workspace management requires a repository owner".to_string());
    }
    if policy.repository_pattern.is_empty() {
        return Err("Workspace repository pattern must not be empty".to_string());
    }
    if policy.search_roots.is_empty() {
        return Err("Workspace management requires at least one search root".to_string());
    }
    Ok(())
}

fn policy_matches(policy: &WorkspaceDiscoveryPolicy, owner: &str, repository: &str) -> bool {
    let owner_matches = policy
        .owner
        .as_deref()
        .is_none_or(|expected| expected.eq_ignore_ascii_case(owner));
    owner_matches && glob_matches(&policy.repository_pattern, repository)
}

fn existing_or_default_path(
    root: &Path,
    policy: &WorkspaceDiscoveryPolicy,
    repository: &str,
) -> PathBuf {
    for search_root in &policy.search_roots {
        let candidate = resolve_repo_path(root, search_root).join(repository);
        if candidate.join(".git").is_dir() || candidate.join("flake.nix").is_file() {
            return candidate;
        }
    }
    resolve_repo_path(root, &policy.search_roots[0]).join(repository)
}

fn clone_repository(remote: &str, path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Clone target has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Create repository directory {}: {error}", parent.display()))?;

    let url = clone_url(remote);
    let output = Command::new("git")
        .args(["clone", "--origin", "origin", "--"])
        .arg(&url)
        .arg(path)
        .output()
        .map_err(|error| format!("Run git clone for {remote}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Clone {remote} into {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn clone_url(remote: &str) -> String {
    remote
        .strip_prefix("github:")
        .map(|path| format!("https://github.com/{path}.git"))
        .unwrap_or_else(|| remote.to_string())
}

fn removal_blocker(
    root: &Path,
    policy: &WorkspaceDiscoveryPolicy,
    path: &Path,
    expected_remote: &str,
    force: bool,
) -> Result<Option<String>, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("Inspect managed repository {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(Some("managed path is a symbolic link".to_string()));
    }
    if !is_under_search_root(root, policy, path) {
        return Ok(Some("managed path is outside configured search roots".to_string()));
    }
    if !path.join(".git").is_dir() {
        return Ok(Some("managed path is no longer a Git repository".to_string()));
    }

    let repo = GitRepo::open(path)?;
    let actual_remote = repo.remote_url("origin")?;
    if !same_remote(&actual_remote, expected_remote) {
        return Ok(Some(format!(
            "origin remote changed from {expected_remote} to {actual_remote}"
        )));
    }
    if repo.status()?.is_dirty() && !force {
        return Ok(Some(
            "repository is dirty; pass --force with --apply to remove it".to_string(),
        ));
    }
    Ok(None)
}

fn same_remote(actual: &str, expected: &str) -> bool {
    match (
        parse_remote_identity(actual),
        parse_remote_identity(expected),
    ) {
        (Some(actual), Some(expected)) => {
            actual.owner.eq_ignore_ascii_case(&expected.owner)
                && actual.repository.eq_ignore_ascii_case(&expected.repository)
        }
        _ => actual == expected,
    }
}

fn is_under_search_root(root: &Path, policy: &WorkspaceDiscoveryPolicy, path: &Path) -> bool {
    let path = canonical_or_owned(path);
    policy.search_roots.iter().any(|search_root| {
        let search_root = canonical_or_owned(&resolve_repo_path(root, search_root));
        path.starts_with(search_root) && path != root
    })
}

fn managed_state_file(workspace: &str) -> PathBuf {
    state_file(workspace).with_extension("managed.json")
}

fn load_managed_state(workspace: &str) -> Result<ManagedWorkspaceState, String> {
    let path = managed_state_file(workspace);
    if !path.exists() {
        return Ok(ManagedWorkspaceState::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("Read managed workspace state {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("Parse managed workspace state {}: {error}", path.display()))
}

fn save_managed_state(workspace: &str, state: &ManagedWorkspaceState) -> Result<(), String> {
    let path = managed_state_file(workspace);
    let parent = path
        .parent()
        .ok_or_else(|| format!("Managed state path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Create managed state directory {}: {error}", parent.display()))?;
    let temporary = path.with_extension("managed.json.tmp");
    let payload = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Serialize managed workspace state: {error}"))?;
    std::fs::write(&temporary, payload)
        .map_err(|error| format!("Write managed workspace state {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, &path)
        .map_err(|error| format!("Replace managed workspace state {}: {error}", path.display()))
}

fn workspace_name(root: &Path) -> String {
    std::env::var("STITCH_WORKSPACE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            root.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workspace")
                .to_string()
        })
}

fn resolve_repo_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn path_to_config(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn canonical_or_owned(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;

    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        match token {
            b'*' => {
                current[0] = previous[0];
                for index in 1..=value.len() {
                    current[index] = previous[index] || current[index - 1];
                }
            }
            b'?' => current[1..].copy_from_slice(&previous[..value.len()]),
            literal => {
                for index in 1..=value.len() {
                    current[index] = previous[index - 1] && *literal == value[index - 1];
                }
            }
        }
        previous = current;
    }

    previous[value.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_remote_becomes_clone_url() {
        assert_eq!(
            clone_url("github:matthis-k/phenix-stitch"),
            "https://github.com/matthis-k/phenix-stitch.git"
        );
    }

    #[test]
    fn policy_file_is_loaded() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join(WORKSPACE_POLICY_FILE),
            r#"{
                "owner": "matthis-k",
                "repository_pattern": "phenix-*",
                "search_roots": ["repos"]
            }"#,
        )
        .unwrap();

        let policy = load_policy(directory.path()).unwrap().unwrap();
        assert_eq!(policy.owner.as_deref(), Some("matthis-k"));
        assert_eq!(policy.repository_pattern, "phenix-*");
        assert_eq!(policy.search_roots, vec![PathBuf::from("repos")]);
    }

    #[test]
    fn locked_repositories_include_transitive_matching_nodes() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("flake.lock"),
            r#"{
                "nodes": {
                    "root": {"inputs": {"tools": "tools"}},
                    "tools": {
                        "inputs": {"opencode": "opencode"},
                        "locked": {
                            "type": "github",
                            "owner": "matthis-k",
                            "repo": "phenix-tools"
                        }
                    },
                    "opencode": {
                        "locked": {
                            "type": "github",
                            "owner": "matthis-k",
                            "repo": "phenix-opencode"
                        }
                    },
                    "nixpkgs": {
                        "locked": {
                            "type": "github",
                            "owner": "NixOS",
                            "repo": "nixpkgs"
                        }
                    }
                },
                "root": "root",
                "version": 7
            }"#,
        )
        .unwrap();

        let policy = WorkspaceDiscoveryPolicy {
            owner: Some("matthis-k".to_string()),
            repository_pattern: "phenix-*".to_string(),
            search_roots: vec![PathBuf::from("repos")],
        };
        let repositories = locked_workspace_repositories(directory.path(), &policy).unwrap();
        let names = repositories
            .into_iter()
            .map(|repository| repository.name)
            .collect::<BTreeSet<_>>();

        assert_eq!(
            names,
            BTreeSet::from([
                "phenix-opencode".to_string(),
                "phenix-tools".to_string()
            ])
        );
    }

    #[test]
    fn glob_supports_workspace_patterns() {
        assert!(glob_matches("phenix-*", "phenix-stitch"));
        assert!(!glob_matches("phenix-*", "nixpkgs"));
    }
}
