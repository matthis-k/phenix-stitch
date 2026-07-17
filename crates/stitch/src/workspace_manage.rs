use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::graph::parse_flake_lock;
use crate::model::RepoConfig;
use crate::workspace::{git_remote_url, parse_remote_identity, WorkspaceDiscoveryPolicy};

pub const WORKSPACE_POLICY_FILE: &str = ".stitch-workspace.json";

pub fn load_policy(root: &Path) -> Result<Option<WorkspaceDiscoveryPolicy>, String> {
    let path = root.join(WORKSPACE_POLICY_FILE);
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("Read workspace policy {}: {error}", path.display()))?;
    let policy = serde_json::from_str::<WorkspaceDiscoveryPolicy>(&content)
        .map_err(|error| format!("Parse workspace policy {}: {error}", path.display()))?;
    validate_policy(&policy)?;
    Ok(Some(policy))
}

pub fn resolve_policy(root: &Path) -> Result<WorkspaceDiscoveryPolicy, String> {
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

/// Return the desired workspace inventory from matching GitHub nodes in the
/// complete root lock graph. This is read-only: Stitch reports paths and remotes
/// but does not create, update, or delete repositories.
pub fn locked_workspace_inventory(root: &Path) -> Result<Vec<RepoConfig>, String> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let policy = resolve_policy(&root)?;
    validate_policy(&policy)?;

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
        if !policy_matches(&policy, owner, repository) {
            continue;
        }

        let path = existing_or_default_path(&root, &policy, repository);
        repositories.insert(
            repository.to_string(),
            RepoConfig {
                name: repository.to_string(),
                path: path_to_config(&root, &path),
                remote: Some(format!("github:{owner}/{repository}")),
            },
        );
    }

    Ok(repositories.into_values().collect())
}

fn validate_policy(policy: &WorkspaceDiscoveryPolicy) -> Result<(), String> {
    if policy.owner.as_deref().is_none_or(str::is_empty) {
        return Err("Workspace inventory requires a repository owner".to_string());
    }
    if policy.repository_pattern.is_empty() {
        return Err("Workspace repository pattern must not be empty".to_string());
    }
    if policy.search_roots.is_empty() {
        return Err("Workspace inventory requires at least one search root".to_string());
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
        let candidate = resolve_path(root, search_root).join(repository);
        if candidate.join(".git").is_dir() || candidate.join("flake.nix").is_file() {
            return candidate;
        }
    }
    resolve_path(root, &policy.search_roots[0]).join(repository)
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
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
    use std::collections::BTreeSet;

    use super::*;

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
    fn inventory_includes_transitive_matching_nodes() {
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

        let repositories = locked_workspace_inventory(directory.path()).unwrap();
        let names = repositories
            .into_iter()
            .map(|repository| repository.name)
            .collect::<BTreeSet<_>>();

        assert_eq!(
            names,
            BTreeSet::from(["phenix-opencode".to_string(), "phenix-tools".to_string()])
        );
    }

    #[test]
    fn glob_supports_workspace_patterns() {
        assert!(glob_matches("phenix-*", "phenix-stitch"));
        assert!(!glob_matches("phenix-*", "nixpkgs"));
    }
}
