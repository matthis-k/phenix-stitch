use crate::changeset;
use crate::model::ChangesetState;

pub fn execute() -> Result<(), String> {
    let mut cs = match changeset::load_current()? {
        Some(cs) => cs,
        None => return Err("No active changeset.".to_string()),
    };

    cs.state = ChangesetState::Aborted;
    changeset::save(&cs)?;
    changeset::clear_plan_file()?;
    println!("Changeset '{}' aborted.", cs.id);
    Ok(())
}
