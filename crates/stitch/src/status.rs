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
        let s = git::get_status(&repo.name, &path)?;
        statuses.push(s);
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

    // 1. Check .tend.json exists
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

    // 2. Check tend binary is locatable
    let tend_available = std::process::Command::new("tend")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    checks.push(IntegrationCheck {
        name: "tend.binary".to_string(),
        passed: tend_available,
        detail: if tend_available {
            "tend found on PATH".to_string()
        } else {
            "tend not found on PATH (may be available via nix run)".to_string()
        },
    });

    // 3. Check the root lock file exists as the workspace source of truth
    let lock_file = root.join("flake.lock");
    checks.push(IntegrationCheck {
        name: "stitch.locked-workspace".to_string(),
        passed: lock_file.exists(),
        detail: if lock_file.exists() {
            format!("Found: {}", lock_file.display())
        } else {
            format!("Missing: {}", lock_file.display())
        },
    });

    // 4. Check Stitch can derive and validate a DAG from flake locks only.
    let graph_report = graph::derive::derive_graph_from_locks(root, None).map(|dag| {
        graph::validate::validate_graph(&dag, &graph::validate::ValidateOptions { strict: true })
    });
    checks.push(IntegrationCheck {
        name: "stitch.locked-graph".to_string(),
        passed: graph_report.as_ref().is_ok_and(|report| report.valid),
        detail: match graph_report {
            Ok(report) => format!(
                "locked graph valid={}, nodes={}, edges={}, diagnostics={}",
                report.valid,
                report.node_count,
                report.edge_count,
                report.diagnostics.len()
            ),
            Err(err) => format!("Failed to derive locked graph: {err}"),
        },
    });

    // 5. Check all configured repos exist
    let mut repos_ok = 0u32;
    let mut repos_missing = 0u32;
    let mut repo_details = Vec::new();
    for repo in &cfg.repos {
        let path = repo.resolved_path(cfg);
        if path.exists() {
            repos_ok += 1;
            repo_details.push(format!("  ✓ {}", repo.name));
        } else {
            repos_missing += 1;
            repo_details.push(format!("  ✗ {} (missing: {})", repo.name, path.display()));
        }
    }
    checks.push(IntegrationCheck {
        name: "repos.present".to_string(),
        passed: repos_missing == 0,
        detail: if repos_missing == 0 {
            format!("All {} repo(s) present", repos_ok)
        } else {
            format!(
                "{} present, {} missing:\n{}",
                repos_ok,
                repos_missing,
                repo_details.join("\n")
            )
        },
    });

    // 6. Confirm the lock-derived Stitch DAG is usable without committed topology.
    let graph_ok = graph::derive::derive_graph_from_locks(root, None).map(|dag| {
        graph::validate::validate_graph(&dag, &graph::validate::ValidateOptions { strict: true })
    });
    checks.push(IntegrationCheck {
        name: "stitch.dag".to_string(),
        passed: graph_ok.as_ref().is_ok_and(|report| report.valid),
        detail: match graph_ok {
            Ok(report) => format!(
                "Lock-derived Stitch DAG valid={}, nodes={}, edges={}",
                report.valid, report.node_count, report.edge_count
            ),
            Err(e) => format!("Stitch DAG failed: {}", e),
        },
    });

    // 7. Per-repo git health: detached HEAD, dirty state, ahead/behind
    let repo_statuses = collect_all(cfg).ok();
    if let Some(ref statuses) = repo_statuses {
        for rs in statuses {
            let is_detached = rs.branch == "HEAD";
            let mut detail_parts = Vec::new();
            detail_parts.push(format!("branch: {}", rs.branch));
            if rs.is_dirty {
                detail_parts.push("dirty".to_string());
            }
            if let Some(ahead) = rs.ahead {
                if ahead > 0 {
                    detail_parts.push(format!("ahead: {}", ahead));
                }
            }
            if let Some(behind) = rs.behind {
                if behind > 0 {
                    detail_parts.push(format!("behind: {}", behind));
                }
            }
            let passed = !is_detached;
            let detail = detail_parts.join(", ");
            checks.push(IntegrationCheck {
                name: format!("repos.{}.git_health", rs.name),
                passed,
                detail: if is_detached {
                    format!("DETACHED HEAD: {}", detail)
                } else {
                    format!("OK: {}", detail)
                },
            });
        }

        // 8. Summary: all repos on valid branch
        let all_healthy = statuses.iter().all(|rs| rs.branch != "HEAD");
        checks.push(IntegrationCheck {
            name: "repos.all_healthy".to_string(),
            passed: all_healthy,
            detail: if all_healthy {
                "All repos on valid branches".to_string()
            } else {
                let detached: Vec<&str> = statuses
                    .iter()
                    .filter(|rs| rs.branch == "HEAD")
                    .map(|rs| rs.name.as_str())
                    .collect();
                format!("Detached HEAD repos: {}", detached.join(", "))
            },
        });
    }

    let all_passed = checks.iter().all(|c| c.passed);

    Ok(IntegrationReport {
        all_passed,
        checks,
        repo_statuses,
    })
}
