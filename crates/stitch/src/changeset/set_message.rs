use crate::changeset;

pub fn execute(repo: &str, message: &str) -> Result<(), String> {
    let mut cs = match changeset::load_current()? {
        Some(cs) => cs,
        None => return Err("No active changeset.".to_string()),
    };

    let rp = cs
        .repos
        .iter_mut()
        .find(|r| r.name == repo)
        .ok_or_else(|| format!("Repo '{}' not found in changeset", repo))?;

    rp.message = Some(message.to_string());
    rp.message_source = "human".to_string();

    changeset::save(&cs)?;
    println!("Set message for {}: {}", repo, message);
    Ok(())
}
