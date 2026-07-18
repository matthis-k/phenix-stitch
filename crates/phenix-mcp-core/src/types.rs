use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MutationLevel {
    ReadOnly,
    /// The tool deliberately accepts arbitrary operations and does not attempt
    /// to classify or gate their effects.
    Arbitrary,
    WritesWorktree,
    WritesGitIndex,
    CreatesCommit,
    Network,
    Destructive,
}

impl MutationLevel {
    pub fn is_read_only(&self) -> bool {
        matches!(self, MutationLevel::ReadOnly)
    }

    pub fn requires_apply(&self) -> bool {
        matches!(
            self,
            MutationLevel::WritesWorktree
                | MutationLevel::WritesGitIndex
                | MutationLevel::CreatesCommit
                | MutationLevel::Network
                | MutationLevel::Destructive
        )
    }

    pub fn requires_confirmation(&self) -> bool {
        matches!(
            self,
            MutationLevel::CreatesCommit | MutationLevel::Network | MutationLevel::Destructive
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl Warning {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
        }
    }

    pub fn with_code(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: Some(code.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedAction {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl SuggestedAction {
    pub fn new(action: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            reason: None,
        }
    }

    pub fn with_reason(action: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub status: ChangeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Untracked,
    Renamed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Risk {
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
}

impl Risk {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            severity: None,
        }
    }

    pub fn with_severity(description: impl Into<String>, severity: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            severity: Some(severity.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub mutation: MutationLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_plan: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_clean_worktree: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_confirmation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_roots_only: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_tools_do_not_gain_an_apply_gate() {
        assert!(!MutationLevel::Arbitrary.is_read_only());
        assert!(!MutationLevel::Arbitrary.requires_apply());
        assert!(!MutationLevel::Arbitrary.requires_confirmation());
    }
}
