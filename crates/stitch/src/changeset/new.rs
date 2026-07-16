use crate::changeset;
use crate::model::{Changeset, ChangesetState, RepoPlan, RepoState};

pub fn execute(title: &str) -> Result<(), String> {
    if changeset::load_current()?.is_some() {
        return Err(
            "An active changeset already exists. Abort it first with `stitch changeset abort`."
                .to_string(),
        );
    }

    let id = crate::model::generate_changeset_id(title);
    let cfg = crate::config::find_and_load()?;

    let repos: Vec<RepoPlan> = cfg
        .repos
        .iter()
        .map(|r| RepoPlan {
            name: r.name.clone(),
            path: r.path.clone(),
            action: None,
            message: None,
            message_source: "human".to_string(),
            files: vec![],
            push: false,
            state: RepoState::Planned,
            commit_hash: None,
        })
        .collect();

    let cs = Changeset {
        id: id.clone(),
        title: title.to_string(),
        workspace: cfg.workspace.clone(),
        state: ChangesetState::Planned,
        repos,
    };

    changeset::save(&cs)?;
    println!("Created changeset: {}", id);
    Ok(())
}
