use crate::changeset;
use crate::config;
use crate::git;
use crate::model::{ChangesetState, RepoState};

pub fn execute(write: bool, json: bool) -> Result<(), String> {
    let cfg = config::find_and_load()?;
    let mut cs = match changeset::load_current()? {
        Some(cs) => cs,
        None => {
            return Err(
                "No active changeset. Run `stitch changeset new \"<title>\"` first.".to_string(),
            )
        }
    };

    if cs.state != ChangesetState::Planned {
        return Err(format!(
            "Changeset is in state '{}', not 'planned'. Cannot plan.",
            cs.state
        ));
    }

    // Build plans from dirty repos
    for rp in &mut cs.repos {
        let repo_cfg = match cfg.repos.iter().find(|r| r.name == rp.name) {
            Some(r) => r,
            None => continue,
        };
        let repo_path = repo_cfg.resolved_path(&cfg);
        if !repo_path.exists() {
            continue;
        }

        let status = git::get_status(&repo_cfg.name, &repo_path)?;
        if status.is_dirty {
            rp.action = Some("commit".to_string());
            if rp.message.is_none() {
                rp.message = None; // explicitly missing
            }
            if rp.files.is_empty() {
                rp.files = git::git_diff_names(&repo_path)?;
            }
            rp.state = RepoState::Planned;
        }
    }

    if json {
        let output = serde_json::to_string_pretty(&cs).map_err(|e| format!("JSON: {}", e))?;
        println!("{}", output);
    } else {
        println!("Changeset: {} ({})", cs.id, cs.title);
        for rp in &cs.repos {
            let action = rp.action.as_deref().unwrap_or("-");
            let msg = rp.message.as_deref().unwrap_or("<missing>");
            println!("  {}  action={}  message={}", rp.name, action, msg);
        }
    }

    if write {
        changeset::save(&cs)?;
        if !json {
            println!("\nPlan written.");
        }
    } else if !json {
        println!("\nUse --write to save this plan.");
    }

    Ok(())
}
