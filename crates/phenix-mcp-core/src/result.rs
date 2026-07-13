use serde::{Deserialize, Serialize};

use crate::types::{SuggestedAction, Warning};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult<T: Serialize> {
    pub ok: bool,
    pub summary: String,
    pub data: T,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Warning>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub next_actions: Vec<SuggestedAction>,
    pub audit_id: String,
}

impl<T: Serialize> ToolResult<T> {
    pub fn ok(data: T, summary: impl Into<String>, audit_id: impl Into<String>) -> Self {
        Self {
            ok: true,
            summary: summary.into(),
            data,
            warnings: Vec::new(),
            next_actions: Vec::new(),
            audit_id: audit_id.into(),
        }
    }

    pub fn with_warning(mut self, warning: Warning) -> Self {
        self.warnings.push(warning);
        self
    }

    pub fn with_action(mut self, action: SuggestedAction) -> Self {
        self.next_actions.push(action);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolErrorDetail {
    pub kind: ErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_tail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ErrorKind {
    InvalidInput,
    RootViolation,
    DirtyWorktree,
    CommandFailed,
    Timeout,
    Conflict,
    PolicyDenied,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFailure {
    pub ok: bool,
    pub summary: String,
    pub error: ToolErrorDetail,
    pub audit_id: String,
}

impl ToolFailure {
    pub fn new(kind: ErrorKind, message: impl Into<String>, audit_id: impl Into<String>) -> Self {
        let msg: String = message.into();
        Self {
            ok: false,
            summary: msg.clone(),
            error: ToolErrorDetail {
                kind,
                message: msg,
                command: None,
                exit_code: None,
                stdout_tail: None,
                stderr_tail: None,
            },
            audit_id: audit_id.into(),
        }
    }

    pub fn with_command(mut self, command: Vec<String>) -> Self {
        self.error.command = Some(command);
        self
    }

    pub fn with_exit_code(mut self, code: i32) -> Self {
        self.error.exit_code = Some(code);
        self
    }

    pub fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
        let s: String = stdout.into();
        self.error.stdout_tail = Some(tail(&s, 2000));
        self
    }

    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        let s: String = stderr.into();
        self.error.stderr_tail = Some(tail(&s, 2000));
        self
    }
}

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let start = s.len() - max;
        format!("...\n{}", &s[start..])
    }
}

pub type ToolOutput = Result<serde_json::Value, ToolFailure>;
