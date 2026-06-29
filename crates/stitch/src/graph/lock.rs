use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct FlakeLock {
    pub nodes: BTreeMap<String, LockNode>,
    pub root: Option<String>,
    pub version: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct LockNode {
    pub inputs: Option<BTreeMap<String, serde_json::Value>>,
    pub locked: Option<Locked>,
    pub original: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct Locked {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub rev: Option<String>,
    pub url: Option<String>,
    pub path: Option<String>,
}

pub fn parse_flake_lock(path: &Path) -> Result<FlakeLock, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("Read {path:?}: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Parse {path:?}: {e}"))
}

pub fn input_target_name(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(parts) => parts
            .iter()
            .rev()
            .find_map(|v| v.as_str().map(str::to_owned)),
        _ => None,
    }
}

pub fn normalize_repo_id_from_locked(locked: &Locked) -> Option<String> {
    match locked.kind.as_deref() {
        Some("github") => {
            let owner = locked.owner.as_deref()?;
            let repo = locked.repo.as_deref()?;
            Some(format!("github:{owner}/{repo}"))
        }
        Some("git") => locked.url.as_deref().and_then(normalize_git_url),
        Some("path") => locked.path.as_deref().and_then(path_basename),
        _ => None,
    }
}

fn normalize_git_url(url: &str) -> Option<String> {
    let url = url.strip_suffix(".git").unwrap_or(url);
    if let Some(rest) = url.strip_prefix("git@") {
        let rest = rest.replace(':', "/");
        Some(format!("git:{rest}"))
    } else if let Some(rest) = url.strip_prefix("https://") {
        Some(format!("git:{rest}"))
    } else if let Some(rest) = url.strip_prefix("git+file://") {
        path_basename(rest)
    } else {
        // Try to extract a basename as fallback
        url.rsplit_once('/')
            .or_else(|| url.rsplit_once(':'))
            .map(|(_, name)| name.to_string())
    }
}

fn path_basename(path: &str) -> Option<String> {
    let path = path.strip_prefix("file://").unwrap_or(path);
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

/// Build a set of aliases for a workspace node id to help with lock matching.
pub fn build_workspace_aliases(
    nodes: &BTreeMap<String, super::WorkspaceNode>,
) -> BTreeMap<String, String> {
    let mut aliases = BTreeMap::new();

    for (id, node) in nodes {
        // id itself
        aliases.insert(id.clone(), id.clone());

        // repo URL basename
        if let Some(url) = &node.repo_url {
            let base = normalize_git_url(url).unwrap_or_default();
            if !base.is_empty() {
                aliases.insert(base, id.clone());
            }

            // SSH-style: git@github.com:owner/repo.git -> "owner/repo" -> "repo"
            if let Some(rest) = url.strip_prefix("git@") {
                if let Some(path_part) = rest.split(':').nth(1) {
                    let path_part = path_part.strip_suffix(".git").unwrap_or(path_part);
                    aliases.insert(format!("github:{path_part}"), id.clone());
                    if let Some(repo_name) = path_part.split('/').nth(1) {
                        aliases.insert(repo_name.to_string(), id.clone());
                    }
                }
            }

            // HTTPS-style: https://github.com/owner/repo.git -> "owner/repo" -> "repo"
            if let Some(rest) = url.strip_prefix("https://github.com/") {
                let rest = rest.strip_suffix(".git").unwrap_or(rest);
                aliases.insert(format!("github:{rest}"), id.clone());
                if let Some(repo_name) = rest.split('/').nth(1) {
                    aliases.insert(repo_name.to_string(), id.clone());
                }
            }
        }
    }

    aliases
}

/// Map a lock target name to a workspace node id using aliases.
pub fn map_lock_target_to_workspace(
    lock_target_name: &str,
    lock_node: &LockNode,
    aliases: &BTreeMap<String, String>,
) -> Option<String> {
    // Direct alias lookup by name
    if let Some(id) = aliases.get(lock_target_name) {
        return Some(id.clone());
    }

    // Try via locked info normalization
    if let Some(locked) = &lock_node.locked {
        if let Some(normalized) = normalize_repo_id_from_locked(locked) {
            if let Some(id) = aliases.get(&normalized) {
                return Some(id.clone());
            }
            // Also try basename of normalized form
            if let Some(repo_name) = normalized.rsplit('/').next() {
                if let Some(id) = aliases.get(repo_name) {
                    return Some(id.clone());
                }
            }
        }
    }

    None
}

/// Build external input from a lock node that is not a workspace node.
pub fn external_from_lock(
    owner_node: String,
    input_name: String,
    lock_node: &LockNode,
) -> super::ExternalInput {
    let (locked_type, url_or_repo, rev) = match &lock_node.locked {
        Some(locked) => {
            let url_or_repo = match locked.kind.as_deref() {
                Some("github") => {
                    let owner = locked.owner.as_deref().unwrap_or("?");
                    let repo = locked.repo.as_deref().unwrap_or("?");
                    Some(format!("github:{owner}/{repo}"))
                }
                _ => locked.url.clone().or_else(|| locked.path.clone()),
            };
            (locked.kind.clone(), url_or_repo, locked.rev.clone())
        }
        None => (None, None, None),
    };

    super::ExternalInput {
        owner_node,
        input_name,
        locked_type,
        url_or_repo,
        rev,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_synthetic_lock() {
        let json = r#"{
            "nodes": {
                "root": {
                    "inputs": {
                        "phenix-pins": "phenix-pins",
                        "nixpkgs": "nixpkgs"
                    }
                },
                "phenix-pins": {
                    "locked": {
                        "type": "github",
                        "owner": "matthis-k",
                        "repo": "phenix-pins",
                        "rev": "abc"
                    }
                },
                "nixpkgs": {
                    "locked": {
                        "type": "github",
                        "owner": "NixOS",
                        "repo": "nixpkgs",
                        "rev": "def"
                    }
                }
            },
            "root": "root",
            "version": 7
        }"#;

        let lock: FlakeLock = serde_json::from_str(json).unwrap();
        assert_eq!(lock.version, Some(7));
        assert_eq!(lock.root.as_deref(), Some("root"));

        let root_node = lock.nodes.get("root").unwrap();
        let inputs = root_node.inputs.as_ref().unwrap();
        assert_eq!(
            inputs.get("phenix-pins").and_then(|v| v.as_str()),
            Some("phenix-pins")
        );
        assert_eq!(
            inputs.get("nixpkgs").and_then(|v| v.as_str()),
            Some("nixpkgs")
        );

        let pins = lock.nodes.get("phenix-pins").unwrap();
        let locked = pins.locked.as_ref().unwrap();
        assert_eq!(locked.kind.as_deref(), Some("github"));
        assert_eq!(locked.repo.as_deref(), Some("phenix-pins"));
    }

    #[test]
    fn test_input_target_name_string() {
        let val = serde_json::Value::String("phenix-pins".to_string());
        assert_eq!(input_target_name(&val), Some("phenix-pins".to_string()));
    }

    #[test]
    fn test_input_target_name_array() {
        let val = serde_json::json!(["nixpkgs", "follows"]);
        assert_eq!(input_target_name(&val), Some("follows".to_string()));
    }

    #[test]
    fn test_normalize_github_id() {
        let locked = Locked {
            kind: Some("github".to_string()),
            owner: Some("matthis-k".to_string()),
            repo: Some("phenix-tools".to_string()),
            rev: Some("abc".to_string()),
            url: None,
            path: None,
        };
        assert_eq!(
            normalize_repo_id_from_locked(&locked),
            Some("github:matthis-k/phenix-tools".to_string())
        );
    }

    #[test]
    fn test_normalize_git_url_ssh() {
        assert_eq!(
            normalize_git_url("git@github.com:matthis-k/phenix-tools.git"),
            Some("git:github.com/matthis-k/phenix-tools".to_string())
        );
    }

    #[test]
    fn test_build_workspace_aliases() {
        use crate::graph::{NodeKind, RepoRole, WorkspaceNode};
        use std::collections::BTreeMap;
        use std::path::PathBuf;

        let mut nodes = BTreeMap::new();
        nodes.insert(
            "phenix-tools".to_string(),
            WorkspaceNode {
                id: "phenix-tools".to_string(),
                path: PathBuf::from("."),
                repo_url: Some("git@github.com:matthis-k/phenix-tools.git".to_string()),
                kind: NodeKind::Unknown,
                role: RepoRole::Unknown,
                layer: None,
                is_root: false,
            },
        );

        let aliases = build_workspace_aliases(&nodes);
        assert_eq!(aliases.get("phenix-tools").unwrap(), "phenix-tools");
        assert!(aliases.contains_key("github:matthis-k/phenix-tools"));
    }
}
