use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
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
#[serde(deny_unknown_fields)]
pub struct Changeset {
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
    let today = crate::time::utc_date();
    let slug = slugify(title);
    format!("{}-{}", today, slug)
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

    #[test]
    fn retired_version_selectors_are_rejected() {
        let workspace = r#"{"version":1,"workspace":"test","repos":[]}"#;
        assert!(serde_json::from_str::<WorkspaceConfig>(workspace).is_err());

        let changeset = r#"{
            "version": 1,
            "id": "test",
            "title": "Test",
            "workspace": "test",
            "state": "Planned",
            "repos": []
        }"#;
        assert!(serde_json::from_str::<Changeset>(changeset).is_err());
    }
}
