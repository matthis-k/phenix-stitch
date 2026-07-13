use crate::git;
use crate::graph;
use crate::model::{RepoAvailability, RepoStatus, WorkspaceConfig};

pub fn collect_all(cfg: &WorkspaceConfig) -> Result<Vec<RepoStatus>, String> {
    let mut statuses = Vec::new();
    for repo in &cfg.repos {
        let path = repo.resolved_path(cfg);
        if !path.exists() {
            statuses.push(RepoStatus {
                name: repo.name.clone(),
                path: repo.path.clone(),
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
                name: repo.name.clone(),
                path: repo.path.clone(),
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
        statuses.push(git::get_status(&repo.name, &path)?);
    }
    Ok(statuses)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IntegrationCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IntegrationReport {
    pub all_passed: bool,
    pub checks: Vec<IntegrationCheck>,
    pub repo_statuses: Option<Vec<RepoStatus>>,
}

pub fn check_integration(cfg: &WorkspaceConfig) -> Result<IntegrationReport, String> {
    use std::path::Path;

    let root = cfg.config_dir.as_deref().unwrap_or(Path::new("."));
    let mut checks = Vec::new();

    checks.push(IntegrationCheck {
        name: "workspace.inventory".to_string(),
        passed: !cfg.repos.is_empty(),
        detail: format!(
            "Discovered {} repository member(s) for workspace '{}'",
            cfg.repos.len(),
            cfg.workspace
        ),
    });

    let metadata = root.join(".stitch").join("topology.json");
    let metadata = metadata.exists().then_some(metadata);
    let graph_report =
        graph::derive::derive_workspace_graph(root, metadata.as_deref()).map(|dag| {
            graph::validate::validate_graph(
                &dag,
                &graph::validate::ValidateOptions { strict: true },
            )
        });
    checks.push(IntegrationCheck {
        name: "workspace.graph".to_string(),
        passed: graph_report.as_ref().is_ok_and(|report| report.valid),
        detail: match graph_report {
            Ok(report) => format!(
                "Discovered graph valid={}, nodes={}, edges={}, diagnostics={}",
                report.valid,
                report.node_count,
                report.edge_count,
                report.diagnostics.len()
            ),
            Err(error) => format!("Failed to derive discovered workspace graph: {error}"),
        },
    });

    let tend_json = root.join(".tend.json");
    checks.push(IntegrationCheck {
        name: "tend.config".to_string(),
        passed: tend_json.exists(),
        detail: if tend_json.exists() {
            format!("Found: {}", tend_json.display())
        } else {
            format!("Missing: {}", tend_json.display())
        },
    });

    let tend_available = std::process::Command::new("tend")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    checks.push(IntegrationCheck {
        name: "tend.binary".to_string(),
        passed: tend_available,
        detail: if tend_available {
            "tend found on PATH".to_string()
        } else {
            "tend not found on PATH".to_string()
        },
    });

    let mut repos_ok = 0u32;
    let mut missing = Vec::new();
    for repo in &cfg.repos {
        let path = repo.resolved_path(cfg);
        if path.exists() {
            repos_ok += 1;
        } else {
            missing.push(format!("{} ({})", repo.name, path.display()));
        }
    }
    checks.push(IntegrationCheck {
        name: "repos.present".to_string(),
        passed: missing.is_empty(),
        detail: if missing.is_empty() {
            format!("All {repos_ok} repository member(s) are present")
        } else {
            format!(
                "{} present, {} missing: {}",
                repos_ok,
                missing.len(),
                missing.join(", ")
            )
        },
    });

    let repo_statuses = collect_all(cfg).ok();
    if let Some(ref statuses) = repo_statuses {
        for status in statuses {
            let detached = status.branch == "HEAD";
            let mut details = vec![format!("branch: {}", status.branch)];
            if status.is_dirty {
                details.push("dirty".to_string());
            }
            if let Some(ahead) = status.ahead.filter(|ahead| *ahead > 0) {
                details.push(format!("ahead: {ahead}"));
            }
            if let Some(behind) = status.behind.filter(|behind| *behind > 0) {
                details.push(format!("behind: {behind}"));
            }
            checks.push(IntegrationCheck {
                name: format!("repos.{}.git-health", status.name),
                passed: !detached,
                detail: if detached {
                    format!("DETACHED HEAD: {}", details.join(", "))
                } else {
                    format!("OK: {}", details.join(", "))
                },
            });
        }
    }

    let all_passed = checks.iter().all(|check| check.passed);
    Ok(IntegrationReport {
        all_passed,
        checks,
        repo_statuses,
    })
}
