use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub version: u32,
    pub workspace: String,
    pub repos: Vec<RepoConfig>,
    #[serde(skip)]
    pub config_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

impl RepoConfig {
    pub fn resolved_path(&self, ws: &WorkspaceConfig) -> PathBuf {
        let p = Path::new(&self.path);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(ref config_dir) = ws.config_dir {
            config_dir.join(p)
        } else {
            std::env::current_dir().unwrap_or_default().join(p)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changeset {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub workspace: String,
    pub state: ChangesetState,
    pub repos: Vec<RepoPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangesetState {
    Planned,
    Validated,
    CommittedPartial,
    Committed,
    PushedPartial,
    Pushed,
    Aborted,
}

impl std::fmt::Display for ChangesetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangesetState::Planned => write!(f, "planned"),
            ChangesetState::Validated => write!(f, "validated"),
            ChangesetState::CommittedPartial => write!(f, "committed-partial"),
            ChangesetState::Committed => write!(f, "committed"),
            ChangesetState::PushedPartial => write!(f, "pushed-partial"),
            ChangesetState::Pushed => write!(f, "pushed"),
            ChangesetState::Aborted => write!(f, "aborted"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoPlan {
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub message_source: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub push: bool,
    pub state: RepoState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RepoState {
    Planned,
    Validated,
    Committed,
    Pushed,
    Skipped,
    Failed,
}

impl std::fmt::Display for RepoState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoState::Planned => write!(f, "planned"),
            RepoState::Validated => write!(f, "validated"),
            RepoState::Committed => write!(f, "committed"),
            RepoState::Pushed => write!(f, "pushed"),
            RepoState::Skipped => write!(f, "skipped"),
            RepoState::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RepoAvailability {
    #[serde(rename = "present")]
    Present,
    #[serde(rename = "missing")]
    Missing,
    #[serde(rename = "not_git_repo")]
    NotGitRepo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStatus {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_dirty: bool,
    pub status: RepoAvailability,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<usize>,
}

pub fn generate_changeset_id(title: &str) -> String {
    let today = chrono_now();
    let slug = slugify(title);
    format!("{}-{}", today, slug)
}

fn chrono_now() -> String {
    // Simple date string without external chrono dep
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Crude date calculation (enough for YYYY-MM-DD)
    let days = secs / 86400;
    let y = 1970_f64 + (days as f64 - 1.0) / 365.25;
    let year = y as u64;
    let remaining = days - ((year - 1970) * 365 + (year - 1969) / 4);
    let month_days = [
        31,
        if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
            29
        } else {
            28
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    let mut day = remaining;
    for &md in &month_days {
        if day <= md {
            break;
        }
        day -= md;
        month += 1;
    }
    format!("{:04}-{:02}-{:02}", year, month, day)
}

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub fn add_trailers(message: &str, changeset_id: &str, workspace: &str) -> String {
    let mut msg = message.to_string();
    if !msg.ends_with('\n') {
        msg.push('\n');
    }
    msg.push('\n');
    msg.push_str(&format!("Change-Set: {}\n", changeset_id));
    msg.push_str(&format!("Workspace: {}\n", workspace));
    msg.push_str("Managed-By: stitch\n");
    msg
}

#[allow(dead_code)]
pub fn short_sha(sha: &str) -> String {
    if sha.len() > 7 {
        sha[..7].to_string()
    } else {
        sha.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_changeset_id() {
        let id = generate_changeset_id("Add Phenix Foundation");
        assert!(id.contains("phenix-foundation"));
        assert!(id.chars().filter(|&c| c == '-').count() >= 3);
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("  spaces!@#  "), "spaces");
        assert_eq!(slugify("uppercase"), "uppercase");
    }

    #[test]
    fn test_add_trailers() {
        let result = add_trailers("feat: add widget", "cs-001", "phenix");
        assert!(result.contains("Change-Set: cs-001"));
        assert!(result.contains("Workspace: phenix"));
        assert!(result.contains("Managed-By: stitch"));
        // The original message should be preserved
        assert!(result.starts_with("feat: add widget"));
    }

    #[test]
    fn test_add_trailers_trailing_newline() {
        let result = add_trailers("feat: add widget\n", "cs-001", "phenix");
        assert!(result.contains("Change-Set: cs-001"));
    }
}
