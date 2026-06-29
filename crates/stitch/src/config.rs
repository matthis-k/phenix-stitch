use std::path::Path;

use crate::model::WorkspaceConfig;

/// Find and load `.stitch.json` from current directory or parents.
pub fn find_and_load() -> Result<WorkspaceConfig, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {}", e))?;
    let mut current: Option<&Path> = Some(&cwd);

    while let Some(dir) = current {
        let candidate = dir.join(".stitch.json");
        if candidate.exists() {
            let mut cfg = load_from(&candidate)?;
            cfg.config_dir = Some(dir.to_path_buf());
            return Ok(cfg);
        }

        let topology = dir.join(".stitch").join("topology.json");
        if topology.exists() {
            let mut cfg = load_topology_as_config(&topology)?;
            cfg.config_dir = Some(dir.to_path_buf());
            return Ok(cfg);
        }
        current = dir.parent();
    }

    // Fallback: use cwd, discover child dirs with .git
    let repos = discover_git_children(&cwd)?;
    if repos.is_empty() {
        return Err(
            "No .stitch.json found and no Git repos discovered in current directory.".to_string(),
        );
    }

    let ws_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();

    Ok(WorkspaceConfig {
        version: 1,
        workspace: ws_name,
        repos,
        config_dir: None,
    })
}

pub fn load_from(path: &Path) -> Result<WorkspaceConfig, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let cfg: WorkspaceConfig = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    if cfg.version < 1 {
        return Err(format!("Unsupported config version {}", cfg.version));
    }
    Ok(cfg)
}

fn load_topology_as_config(path: &Path) -> Result<WorkspaceConfig, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if version < 1 {
        return Err(format!("Unsupported topology version {}", version));
    }
    let workspace = value
        .get("workspace")
        .and_then(|v| v.as_str())
        .unwrap_or("workspace")
        .to_string();
    let repo_values = value
        .get("repos")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            format!(
                "Topology file {} missing required non-empty 'repos' array",
                path.display()
            )
        })?;
    if repo_values.is_empty() {
        return Err(format!(
            "Topology file {} has empty 'repos' array",
            path.display()
        ));
    }
    let mut repos = Vec::new();
    for repo in repo_values {
        let name = repo
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("Topology entry in {} missing 'name'", path.display()))?;
        let repo_path = repo.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            format!(
                "Topology entry '{}' in {} missing 'path'",
                name,
                path.display()
            )
        })?;
        repos.push(crate::model::RepoConfig {
            name: name.to_string(),
            path: repo_path.to_string(),
        });
    }
    Ok(WorkspaceConfig {
        version,
        workspace,
        repos,
        config_dir: None,
    })
}

fn discover_git_children(dir: &Path) -> Result<Vec<crate::model::RepoConfig>, String> {
    let mut repos = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read dir {}: {}", dir.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        if path.is_dir() && path.join(".git").exists() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if !name.starts_with('.') {
                    repos.push(crate::model::RepoConfig {
                        name: name.to_string(),
                        path: path
                            .strip_prefix(dir)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .to_string(),
                    });
                }
            }
        }
    }

    repos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(repos)
}
