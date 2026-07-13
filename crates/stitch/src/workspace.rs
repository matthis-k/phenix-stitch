use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::graph::parse_flake_lock;
use crate::model::{RepoConfig, WorkspaceConfig};

const DISCOVERY_OWNER_ENV: &str = "STITCH_DISCOVERY_OWNER";
const DISCOVERY_REPOSITORY_PATTERN_ENV: &str = "STITCH_DISCOVERY_REPOSITORY_PATTERN";
const DISCOVERY_ROOTS_ENV: &str = "STITCH_DISCOVERY_ROOTS";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceMode {
    Locked,
    Workspace,
    Mixed,
}

/// Declarative workspace-membership policy.
///
/// Membership and dependency discovery are deliberately separate concerns:
/// this policy selects local repositories, while each selected repository's
/// `flake.lock` determines graph edges. `repository_pattern` is a shell-style
/// glob over repository names (`*` and `?` are supported).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDiscoveryPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default = "default_repository_pattern")]
    pub repository_pattern: String,
    #[serde(default = "default_search_roots")]
    pub search_roots: Vec<PathBuf>,
}

impl Default for WorkspaceDiscoveryPolicy {
    fn default() -> Self {
        Self {
            owner: None,
            repository_pattern: default_repository_pattern(),
            search_roots: default_search_roots(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub version: u32,
    #[serde(default)]
    pub discovery: WorkspaceDiscoveryPolicy,
    #[serde(default)]
    pub repos: BTreeMap<String, WorkspaceRepoState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRepoState {
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteIdentity {
    pub owner: String,
    pub repository: String,
}

pub fn load_workspace_config(root: &Path) -> Result<WorkspaceConfig, String> {
    load_workspace_config_with_policy(root, None)
}

pub fn load_workspace_config_with_policy(
    root: &Path,
    policy_override: Option<WorkspaceDiscoveryPolicy>,
) -> Result<WorkspaceConfig, String> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let workspace = workspace_name(&root);
    let state = load_state(&workspace).ok();
    let policy = resolve_discovery_policy(state.as_ref(), policy_override)?;
    let matcher = RepositoryMatcher::new(&policy)?;

    let mut discovered = discover_local_repositories(&root, &policy, &matcher)?;
    apply_locked_inputs(&root, &policy, &matcher, &mut discovered)?;
    apply_state_repositories(&root, state.as_ref(), &matcher, &mut discovered);

    let mut repos = vec![RepoConfig {
        name: workspace.clone(),
        path: ".".to_string(),
        remote: git_remote_url(&root),
    }];
    repos.extend(discovered.into_values());

    Ok(WorkspaceConfig {
        version: 2,
        workspace,
        repos,
        config_dir: Some(root),
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

pub fn save_state(workspace: &str, state: &WorkspaceState) -> Result<(), String> {
    let path = state_file(workspace);
    let parent = path
        .parent()
        .ok_or_else(|| format!("Workspace state path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Create workspace state directory {}: {e}", parent.display()))?;

    let temporary = path.with_extension("json.tmp");
    let payload =
        serde_json::to_vec_pretty(state).map_err(|e| format!("Serialize workspace state: {e}"))?;
    std::fs::write(&temporary, payload)
        .map_err(|e| format!("Write workspace state {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, &path)
        .map_err(|e| format!("Replace workspace state {}: {e}", path.display()))
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

pub fn parse_remote_identity(remote: &str) -> Option<RemoteIdentity> {
    let path = if let Some(path) = remote.strip_prefix("github:") {
        path
    } else if let Some((_, path)) = remote.split_once(':').filter(|(prefix, _)| {
        prefix.contains('@') && !prefix.contains('/') && !prefix.contains('\\')
    }) {
        path
    } else if let Some((_, authority_and_path)) = remote.split_once("://") {
        authority_and_path
            .split_once('/')
            .map(|(_, remainder)| remainder)?
    } else {
        remote
    };

    let mut components = path.trim_matches('/').split('/');
    let owner = components.next()?.trim();
    let repository = components.next()?.trim().trim_end_matches(".git");
    if owner.is_empty() || repository.is_empty() || components.next().is_some() {
        return None;
    }

    Some(RemoteIdentity {
        owner: owner.to_string(),
        repository: repository.to_string(),
    })
}

fn resolve_discovery_policy(
    state: Option<&WorkspaceState>,
    policy_override: Option<WorkspaceDiscoveryPolicy>,
) -> Result<WorkspaceDiscoveryPolicy, String> {
    let mut policy = policy_override
        .or_else(|| state.map(|state| state.discovery.clone()))
        .unwrap_or_default();

    if let Ok(owner) = std::env::var(DISCOVERY_OWNER_ENV) {
        let owner = owner.trim();
        policy.owner = (!owner.is_empty()).then(|| owner.to_string());
    }
    if let Ok(pattern) = std::env::var(DISCOVERY_REPOSITORY_PATTERN_ENV) {
        let pattern = pattern.trim();
        if !pattern.is_empty() {
            policy.repository_pattern = pattern.to_string();
        }
    }
    if let Some(raw_roots) = std::env::var_os(DISCOVERY_ROOTS_ENV) {
        let roots = std::env::split_paths(&raw_roots).collect::<Vec<_>>();
        if !roots.is_empty() {
            policy.search_roots = roots;
        }
    }

    RepositoryMatcher::new(&policy)?;
    Ok(policy)
}

fn discover_local_repositories(
    root: &Path,
    policy: &WorkspaceDiscoveryPolicy,
    matcher: &RepositoryMatcher,
) -> Result<BTreeMap<String, RepoConfig>, String> {
    let mut repos = BTreeMap::new();

    for configured_root in &policy.search_roots {
        let search_root = resolve_search_root(root, configured_root);
        if !search_root.is_dir() {
            continue;
        }

        let entries = std::fs::read_dir(&search_root)
            .map_err(|e| format!("Read discovery root {}: {e}", search_root.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| {
                format!("Read repository entry under {}: {e}", search_root.display())
            })?;
            let path = entry.path();
            if !path.is_dir() || !path.join(".git").exists() {
                continue;
            }
            let Some(directory_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if directory_name.starts_with('.') {
                continue;
            }

            let remote = git_remote_url(&path);
            let identity = remote.as_deref().and_then(parse_remote_identity);
            let repository_name = identity
                .as_ref()
                .map(|identity| identity.repository.as_str())
                .unwrap_or(directory_name);
            if !matcher.matches(repository_name, identity.as_ref()) {
                continue;
            }

            repos.insert(
                repository_name.to_string(),
                RepoConfig {
                    name: repository_name.to_string(),
                    path: path_to_config(root, &path),
                    remote,
                },
            );
        }
    }

    Ok(repos)
}

fn apply_locked_inputs(
    root: &Path,
    policy: &WorkspaceDiscoveryPolicy,
    matcher: &RepositoryMatcher,
    repos: &mut BTreeMap<String, RepoConfig>,
) -> Result<(), String> {
    // Locked inputs are dependency evidence, not implicit workspace membership.
    // They may contribute missing members only when the caller supplied an
    // actual owner or name constraint that distinguishes workspace repos from
    // external dependencies such as nixpkgs.
    if policy.owner.is_none() && policy.repository_pattern == "*" {
        return Ok(());
    }

    let lock_path = root.join("flake.lock");
    if !lock_path.exists() {
        return Ok(());
    }

    let lock = parse_flake_lock(&lock_path).map_err(|e| format!("Read locked workspace: {e}"))?;
    let root_lock_node = lock.root.as_deref().unwrap_or("root");
    let Some(root_inputs) = lock
        .nodes
        .get(root_lock_node)
        .and_then(|node| node.inputs.as_ref())
    else {
        return Ok(());
    };

    for input_value in root_inputs.values() {
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
        let (Some(owner), Some(repository)) = (locked.owner.as_deref(), locked.repo.as_deref())
        else {
            continue;
        };
        let identity = RemoteIdentity {
            owner: owner.to_string(),
            repository: repository.to_string(),
        };
        if !matcher.matches(repository, Some(&identity)) {
            continue;
        }

        repos.entry(repository.to_string()).or_insert_with(|| {
            let path = discover_local_repo_path(root, policy, repository);
            RepoConfig {
                name: repository.to_string(),
                path: path_to_config(root, &path),
                remote: Some(format!("github:{owner}/{repository}")),
            }
        });
    }

    Ok(())
}

fn apply_state_repositories(
    root: &Path,
    state: Option<&WorkspaceState>,
    matcher: &RepositoryMatcher,
    repos: &mut BTreeMap<String, RepoConfig>,
) {
    let Some(state) = state else {
        return;
    };

    for (name, configured) in &state.repos {
        let identity = configured.remote.as_deref().and_then(parse_remote_identity);
        let repository_name = identity
            .as_ref()
            .map(|identity| identity.repository.as_str())
            .unwrap_or(name);
        if !matcher.matches(repository_name, identity.as_ref()) {
            continue;
        }

        let path = if configured.path.is_absolute() {
            configured.path.clone()
        } else {
            root.join(&configured.path)
        };
        repos.insert(
            repository_name.to_string(),
            RepoConfig {
                name: repository_name.to_string(),
                path: path_to_config(root, &path),
                remote: configured.remote.clone(),
            },
        );
    }
}

fn discover_local_repo_path(
    root: &Path,
    policy: &WorkspaceDiscoveryPolicy,
    repository: &str,
) -> PathBuf {
    for configured_root in &policy.search_roots {
        let candidate = resolve_search_root(root, configured_root).join(repository);
        if candidate.join(".git").exists() || candidate.join("flake.nix").exists() {
            return candidate;
        }
    }

    let fallback_root = policy
        .search_roots
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("repos"));
    resolve_search_root(root, &fallback_root).join(repository)
}

fn resolve_search_root(root: &Path, configured: &Path) -> PathBuf {
    if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        root.join(configured)
    }
}

fn path_to_config(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn workspace_name(root: &Path) -> String {
    std::env::var("STITCH_WORKSPACE")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| {
            root.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workspace")
                .to_string()
        })
}

fn default_repository_pattern() -> String {
    "*".to_string()
}

fn default_search_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("."), PathBuf::from("repos")]
}

struct RepositoryMatcher {
    owner: Option<String>,
    repository_pattern: String,
}

impl RepositoryMatcher {
    fn new(policy: &WorkspaceDiscoveryPolicy) -> Result<Self, String> {
        if policy.repository_pattern.is_empty() {
            return Err("Workspace repository pattern must not be empty".to_string());
        }
        Ok(Self {
            owner: policy.owner.clone(),
            repository_pattern: policy.repository_pattern.clone(),
        })
    }

    fn matches(&self, repository: &str, identity: Option<&RemoteIdentity>) -> bool {
        if !glob_matches(&self.repository_pattern, repository) {
            return false;
        }
        match (&self.owner, identity) {
            (None, _) => true,
            (Some(expected), Some(actual)) => expected.eq_ignore_ascii_case(&actual.owner),
            (Some(_), None) => false,
        }
    }
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
    fn state_file_uses_workspace_name() {
        let path = state_file("demo");
        assert!(path.ends_with("stitch/demo.json"));
    }

    #[test]
    fn parses_supported_remote_forms() {
        for remote in [
            "github:matthis-k/phenix-stitch",
            "git@github.com:matthis-k/phenix-stitch.git",
            "https://github.com/matthis-k/phenix-stitch.git",
            "ssh://git@github.com/matthis-k/phenix-stitch.git",
        ] {
            assert_eq!(
                parse_remote_identity(remote),
                Some(RemoteIdentity {
                    owner: "matthis-k".to_string(),
                    repository: "phenix-stitch".to_string(),
                })
            );
        }
    }

    #[test]
    fn repository_matcher_combines_owner_and_pattern() {
        let matcher = RepositoryMatcher::new(&WorkspaceDiscoveryPolicy {
            owner: Some("matthis-k".to_string()),
            repository_pattern: "phenix-*".to_string(),
            search_roots: default_search_roots(),
        })
        .unwrap();

        assert!(matcher.matches(
            "phenix-stitch",
            Some(&RemoteIdentity {
                owner: "MATTHIS-K".to_string(),
                repository: "phenix-stitch".to_string(),
            })
        ));
        assert!(!matcher.matches(
            "other-stitch",
            Some(&RemoteIdentity {
                owner: "matthis-k".to_string(),
                repository: "other-stitch".to_string(),
            })
        ));
        assert!(!matcher.matches("phenix-stitch", None));
    }

    #[test]
    fn glob_pattern_supports_star_and_question_mark() {
        assert!(glob_matches("phenix-*", "phenix-stitch"));
        assert!(glob_matches("phenix-????", "phenix-tend"));
        assert!(!glob_matches("phenix-????", "phenix-stitch"));
        assert!(!glob_matches("phenix-*", "newxos"));
    }

    #[test]
    fn unconstrained_policy_does_not_promote_external_locked_inputs() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("flake.lock"),
            r#"{
                "nodes": {
                    "root": {"inputs": {"nixpkgs": "nixpkgs"}},
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

        let config = load_workspace_config_with_policy(
            directory.path(),
            Some(WorkspaceDiscoveryPolicy::default()),
        )
        .unwrap();

        assert_eq!(config.repos.len(), 1);
        assert!(!config.repos.iter().any(|repo| repo.name == "nixpkgs"));
    }

    #[test]
    fn empty_repository_pattern_is_rejected() {
        let error = RepositoryMatcher::new(&WorkspaceDiscoveryPolicy {
            owner: None,
            repository_pattern: String::new(),
            search_roots: default_search_roots(),
        })
        .err()
        .unwrap();
        assert!(error.contains("must not be empty"));
    }
}
