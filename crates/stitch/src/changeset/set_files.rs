use crate::changeset;

pub fn execute(repo: &str, files: &[String]) -> Result<(), String> {
    let mut cs = match changeset::load_current()? {
        Some(cs) => cs,
        None => return Err("No active changeset.".to_string()),
    };

    let rp = cs
        .repos
        .iter_mut()
        .find(|r| r.name == repo)
        .ok_or_else(|| format!("Repo '{}' not found in changeset", repo))?;

    rp.files = files.to_vec();

    changeset::save(&cs)?;
    println!(
        "Set {} files for {}: {}",
        files.len(),
        repo,
        files.join(" ")
    );
    Ok(())
}
