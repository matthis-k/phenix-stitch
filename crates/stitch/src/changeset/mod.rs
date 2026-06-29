pub mod abort;
pub mod commit;
pub mod new;
pub mod plan;
pub mod push;
pub mod set_files;
pub mod set_message;
pub mod validate;

use crate::model::{Changeset, WorkspaceConfig};

const PLAN_FILE: &str = ".stitch-plan.json";

pub enum ChangesetCommands {
    New { title: String },
    Status { json: bool },
    Plan { write: bool, json: bool },
    SetMessage { repo: String, message: String },
    SetFiles { repo: String, files: Vec<String> },
    Validate { json: bool },
    Commit,
    Push,
    Abort,
}

pub fn dispatch(command: &ChangesetCommands) -> Result<(), String> {
    match command {
        ChangesetCommands::New { title } => new::execute(title),
        ChangesetCommands::Status { json } => status_cmd(*json),
        ChangesetCommands::Plan { write, json } => plan::execute(*write, *json),
        ChangesetCommands::SetMessage { repo, message } => set_message::execute(repo, message),
        ChangesetCommands::SetFiles { repo, files } => set_files::execute(repo, files),
        ChangesetCommands::Validate { json } => validate::execute(*json),
        ChangesetCommands::Commit => commit::execute(),
        ChangesetCommands::Push => push::execute(),
        ChangesetCommands::Abort => abort::execute(),
    }
}

fn status_cmd(json: bool) -> Result<(), String> {
    let cs = load_current()?;
    match cs {
        Some(cs) => {
            if json {
                let output =
                    serde_json::to_string_pretty(&cs).map_err(|e| format!("JSON: {}", e))?;
                println!("{}", output);
            } else {
                println!("Changeset: {} ({})", cs.id, cs.title);
                println!("State: {}", cs.state);
                println!("Workspace: {}", cs.workspace);
                println!();
                for rp in &cs.repos {
                    let action = rp.action.as_deref().unwrap_or("-");
                    let msg = rp.message.as_deref().unwrap_or("<missing>");
                    let hash = rp.commit_hash.as_deref().unwrap_or("-");
                    println!(
                        "  {}  action={}  message={}  hash={}",
                        rp.name, action, msg, hash
                    );
                }
            }
        }
        None => {
            println!("No active changeset.");
        }
    }
    Ok(())
}

// --- Storage ---

fn stitch_dir() -> std::path::PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    cwd.join(".stitch")
}

fn plan_path() -> std::path::PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    cwd.join(PLAN_FILE)
}

fn changesets_dir() -> std::path::PathBuf {
    stitch_dir().join("changesets")
}

fn changeset_path(id: &str) -> std::path::PathBuf {
    changesets_dir().join(format!("{}.json", id))
}

pub fn load_current() -> Result<Option<Changeset>, String> {
    let plan_p = plan_path();
    if plan_p.exists() {
        let content = std::fs::read_to_string(&plan_p)
            .map_err(|e| format!("Failed to read plan file: {}", e))?;
        let cs: Changeset = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse plan file: {}", e))?;
        return Ok(Some(cs));
    }

    let cs_dir = changesets_dir();
    if !cs_dir.exists() {
        return Ok(None);
    }

    let mut entries: Vec<_> = std::fs::read_dir(&cs_dir)
        .map_err(|e| format!("Failed to read {}: {}", cs_dir.display(), e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();

    entries.sort_by_key(|e| e.path());
    if let Some(latest) = entries.last() {
        let content = std::fs::read_to_string(latest.path())
            .map_err(|e| format!("Failed to read {}: {}", latest.path().display(), e))?;
        let cs: Changeset = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {}", latest.path().display(), e))?;
        Ok(Some(cs))
    } else {
        Ok(None)
    }
}

pub fn save(cs: &Changeset) -> Result<(), String> {
    let cs_dir = changesets_dir();
    std::fs::create_dir_all(&cs_dir)
        .map_err(|e| format!("Failed to create {}: {}", cs_dir.display(), e))?;

    let cs_path = changeset_path(&cs.id);
    let content = serde_json::to_string_pretty(cs).map_err(|e| format!("JSON serialize: {}", e))?;
    std::fs::write(&cs_path, &content)
        .map_err(|e| format!("Failed to write {}: {}", cs_path.display(), e))?;

    let plan_p = plan_path();
    std::fs::write(&plan_p, &content)
        .map_err(|e| format!("Failed to write {}: {}", plan_p.display(), e))?;

    Ok(())
}

pub fn clear_plan_file() -> Result<(), String> {
    let plan_p = plan_path();
    if plan_p.exists() {
        std::fs::remove_file(&plan_p).map_err(|e| format!("Failed to remove plan file: {}", e))?;
    }
    Ok(())
}

pub fn verify_no_newxos_mutation(_cfg: &WorkspaceConfig, cs: &Changeset) -> Result<(), String> {
    for rp in &cs.repos {
        if rp.path == "newxos" || rp.path.starts_with("newxos/") {
            return Err(format!(
                "Changeset includes '{}' which is in newxos. Stitch must not mutate newxos.",
                rp.path
            ));
        }
        for f in &rp.files {
            if f.starts_with("newxos") || f.contains("/newxos/") {
                return Err(format!(
                    "Changeset file '{}' is inside newxos. Stitch must not mutate newxos.",
                    f
                ));
            }
        }
    }
    Ok(())
}
