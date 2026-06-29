use crate::changeset;
use crate::model::ChangesetState;

pub fn execute() -> Result<(), String> {
    let cs = match changeset::load_current()? {
        Some(cs) => cs,
        None => return Err("No active changeset.".to_string()),
    };

    if cs.state != ChangesetState::Committed {
        return Err(format!(
            "Changeset state is '{}', not 'committed'. Run `stitch changeset commit` first.",
            cs.state
        ));
    }

    Err("Push is not implemented yet".to_string())
}
