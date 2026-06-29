use crate::changeset;
use crate::config;
use crate::model::ChangesetState;
use crate::validate;

pub fn execute(json: bool) -> Result<(), String> {
    let cfg = config::find_and_load()?;
    let mut cs = match changeset::load_current()? {
        Some(cs) => cs,
        None => {
            return Err(
                "No active changeset. Run `stitch changeset new \"<title>\"` first.".to_string(),
            )
        }
    };

    let errors = validate::validate_changeset(&cfg, &cs)?;

    if json {
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "valid": errors.is_empty(),
            "changeset_id": cs.id,
            "errors": errors
        }))
        .map_err(|e| format!("JSON: {}", e))?;
        println!("{}", output);
    } else {
        if errors.is_empty() {
            println!("Changeset '{}' is valid.", cs.id);
        } else {
            println!("Changeset '{}' has {} error(s):", cs.id, errors.len());
            for e in &errors {
                println!("  - {}", e);
            }
        }
    }

    if !errors.is_empty() {
        return Err("Validation failed".to_string());
    }

    cs.state = ChangesetState::Validated;
    changeset::save(&cs)?;

    Ok(())
}
