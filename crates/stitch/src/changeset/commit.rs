use crate::changeset;
use crate::config;
use crate::git;
use crate::model::{ChangesetState, RepoState};

pub fn execute() -> Result<(), String> {
    let cfg = config::find_and_load()?;
    let mut cs = match changeset::load_current()? {
        Some(cs) => cs,
        None => return Err("No active changeset.".to_string()),
    };

    if cs.state != ChangesetState::Validated && cs.state != ChangesetState::CommittedPartial {
        return Err(format!(
            "Changeset state '{}' does not support commit.",
            cs.state
        ));
    }

    // Revalidate before committing
    let errors = crate::validate::validate_changeset(&cfg, &cs)?;
    if !errors.is_empty() {
        cs.state = ChangesetState::Planned;
        changeset::save(&cs)?;
        return Err("Validation failed. Run `stitch changeset validate` for details.".to_string());
    }

    let ws_name = cfg.workspace.clone();
    let cs_id = cs.id.clone();
    let mut all_ok = true;

    for rp in &mut cs.repos {
        if rp.action.as_deref() != Some("commit") {
            rp.state = RepoState::Skipped;
            continue;
        }

        let repo_cfg = match cfg.repos.iter().find(|r| r.name == rp.name) {
            Some(r) => r,
            None => {
                rp.state = RepoState::Failed;
                all_ok = false;
                continue;
            }
        };

        let repo_path = repo_cfg.resolved_path(&cfg);
        let msg = match &rp.message {
            Some(m) => crate::model::add_trailers(m, &cs_id, &ws_name),
            None => {
                rp.state = RepoState::Failed;
                all_ok = false;
                eprintln!("Commit failed for {}: no message", rp.name);
                continue;
            }
        };

        // Stage files
        if !rp.files.is_empty() {
            if let Err(e) = git::git_add(&repo_path, &rp.files) {
                rp.state = RepoState::Failed;
                all_ok = false;
                eprintln!("Commit failed for {}: {}", rp.name, e);
                continue;
            }
        }

        // Commit
        match git::git_commit(&repo_path, &msg) {
            Ok(()) => {
                let hash = git::git_short_head(&repo_path).ok();
                rp.commit_hash = hash;
                rp.state = RepoState::Committed;
                println!(
                    "  committed {}: {}",
                    rp.name,
                    rp.commit_hash.as_deref().unwrap_or("?")
                );
            }
            Err(e) => {
                rp.state = RepoState::Failed;
                all_ok = false;
                eprintln!("Commit failed for {}: {}", rp.name, e);
            }
        }
    }

    if all_ok {
        cs.state = ChangesetState::Committed;
    } else {
        cs.state = ChangesetState::CommittedPartial;
    }

    changeset::save(&cs)?;

    if all_ok {
        println!("Changeset '{}' fully committed.", cs.id);
    } else {
        eprintln!(
            "Changeset '{}' partially committed. Some repos failed.",
            cs.id
        );
        return Err("Partial commit".to_string());
    }

    Ok(())
}
