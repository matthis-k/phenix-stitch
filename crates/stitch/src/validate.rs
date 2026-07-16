use crate::config;
use crate::git;
use crate::model::{Changeset, WorkspaceConfig};
use std::path::Path;

#[allow(dead_code)]
pub fn execute(json: bool) -> Result<(), String> {
    let cfg = config::find_and_load()?;
    let cs = crate::changeset::load_current()?;

    let cs = match cs {
        Some(cs) => cs,
        None => {
            return Err(
                "No active changeset. Run `stitch changeset new \"<title>\"` first.".to_string(),
            )
        }
    };

    let errors = validate_changeset(&cfg, &cs)?;

    if json {
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "valid": errors.is_empty(),
            "errors": errors
        }))
        .map_err(|e| format!("JSON: {}", e))?;
        println!("{}", output);
    } else {
        if errors.is_empty() {
            println!("Changeset '{}' is valid.", cs.id);
        } else {
            println!(
                "Changeset '{}' has {} validation error(s):",
                cs.id,
                errors.len()
            );
            for e in &errors {
                println!("  - {}", e);
            }
        }
    }

    if !errors.is_empty() {
        return Err("Validation failed".to_string());
    }

    Ok(())
}

pub fn validate_changeset(cfg: &WorkspaceConfig, cs: &Changeset) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();

    // Check each repo plan
    for rp in &cs.repos {
        let repo_cfg = cfg.repos.iter().find(|r| r.name == rp.name);
        let repo_cfg = match repo_cfg {
            Some(r) => r,
            None => {
                errors.push(format!(
                    "Repo '{}' in changeset is not in workspace config",
                    rp.name
                ));
                continue;
            }
        };

        let repo_path = repo_cfg.resolved_path(cfg);

        // Check repo exists
        if !repo_path.exists() {
            errors.push(format!(
                "Repo path '{}' does not exist",
                repo_path.display()
            ));
            continue;
        }

        // Check it's a Git repo
        if !repo_path.join(".git").exists() {
            errors.push(format!("'{}' is not a Git repo", repo_path.display()));
            continue;
        }

        // If committing, must have a message
        if rp.action.as_deref() == Some("commit") {
            if rp.message.as_ref().is_none_or(|m| m.trim().is_empty()) {
                errors.push(format!(
                    "Repo '{}' has action 'commit' but no message",
                    rp.name
                ));
            }

            // Check selected files exist
            for f in &rp.files {
                let full_path = repo_path.join(f);
                let exists = full_path.exists();
                if !exists {
                    // Could be a deleted file; check if it was tracked
                    let tracked = git_tracked_file(&repo_path, f)?;
                    if !tracked {
                        errors.push(format!(
                            "File '{}' in repo '{}' does not exist and is not tracked by Git",
                            f, rp.name
                        ));
                    }
                }
            }
        }

        // Check for merge conflicts
        if git::has_merge_conflict_markers(&repo_path)? {
            errors.push(format!("Repo '{}' has merge conflict markers", rp.name));
        }

        // Check for mid-merge
        if git::is_mid_merge(&repo_path)? {
            errors.push(format!("Repo '{}' is in the middle of a merge", rp.name));
        }
    }

    Ok(errors)
}

fn git_tracked_file(repo: &Path, file: &str) -> Result<bool, String> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "--error-unmatch", file])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git ls-files: {}", e))?;
    Ok(output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn test_validate_no_message_for_commit_action() {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let path_str = repo_path.to_string_lossy().to_string();
        let cfg = WorkspaceConfig {
            workspace: "test".to_string(),
            repos: vec![RepoConfig {
                name: "test-repo".to_string(),
                path: path_str.clone(),
                remote: None,
            }],
            config_dir: None,
        };

        let cs = Changeset {
            id: "test-cs".to_string(),
            title: "Test".to_string(),
            workspace: "test".to_string(),
            state: ChangesetState::Planned,
            repos: vec![RepoPlan {
                name: "test-repo".to_string(),
                path: path_str,
                action: Some("commit".to_string()),
                message: None,
                message_source: "human".to_string(),
                files: vec![],
                push: false,
                state: RepoState::Planned,
                commit_hash: None,
            }],
        };

        let errors = validate_changeset(&cfg, &cs).unwrap();
        assert!(errors.iter().any(|e| e.contains("no message")));
    }

    #[test]
    fn test_validate_non_existent_repo() {
        let dir = tempfile::tempdir().unwrap();
        let missing_path = dir
            .path()
            .join("missing-repo")
            .to_string_lossy()
            .to_string();
        let cfg = WorkspaceConfig {
            workspace: "test".to_string(),
            repos: vec![RepoConfig {
                name: "ghost-repo".to_string(),
                path: missing_path.clone(),
                remote: None,
            }],
            config_dir: None,
        };

        let cs = Changeset {
            id: "test-cs".to_string(),
            title: "Test".to_string(),
            workspace: "test".to_string(),
            state: ChangesetState::Planned,
            repos: vec![RepoPlan {
                name: "ghost-repo".to_string(),
                path: missing_path,
                action: Some("commit".to_string()),
                message: Some("test".to_string()),
                message_source: "human".to_string(),
                files: vec![],
                push: false,
                state: RepoState::Planned,
                commit_hash: None,
            }],
        };

        let errors = validate_changeset(&cfg, &cs).unwrap();
        assert!(errors.iter().any(|e| e.contains("does not exist")));
    }
}
