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
        let msg = sanitize_terminal_text(&message.into());
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
        let sanitized = sanitize_terminal_text(&stdout.into());
        self.error.stdout_tail = Some(tail(&sanitized, 2000));
        self
    }

    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        let sanitized = sanitize_terminal_text(&stderr.into());
        self.error.stderr_tail = Some(tail(&sanitized, 2000));
        self
    }
}

fn sanitize_terminal_text(input: &str) -> String {
    #[derive(Clone, Copy)]
    enum EscapeState {
        Text,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }

    let mut state = EscapeState::Text;
    let mut output = String::with_capacity(input.len());

    for character in input.chars() {
        state = match state {
            EscapeState::Text => match character {
                '\u{1b}' => EscapeState::Escape,
                '\n' | '\t' => {
                    output.push(character);
                    EscapeState::Text
                }
                '\r' => EscapeState::Text,
                value if value.is_control() => EscapeState::Text,
                value => {
                    output.push(value);
                    EscapeState::Text
                }
            },
            EscapeState::Escape => match character {
                '[' => EscapeState::Csi,
                ']' => EscapeState::Osc,
                '\u{1b}' => EscapeState::Escape,
                _ => EscapeState::Text,
            },
            EscapeState::Csi => {
                if ('@'..='~').contains(&character) {
                    EscapeState::Text
                } else {
                    EscapeState::Csi
                }
            }
            EscapeState::Osc => match character {
                '\u{7}' => EscapeState::Text,
                '\u{1b}' => EscapeState::OscEscape,
                _ => EscapeState::Osc,
            },
            EscapeState::OscEscape => match character {
                '\\' => EscapeState::Text,
                '\u{1b}' => EscapeState::OscEscape,
                _ => EscapeState::Osc,
            },
        };
    }

    output
}

fn tail(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        s.to_string()
    } else {
        let start = char_count - max;
        format!("...\n{}", s.chars().skip(start).collect::<String>())
    }
}

pub type ToolOutput = Result<serde_json::Value, ToolFailure>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_text_strips_terminal_control_sequences() {
        let failure = ToolFailure::new(
            ErrorKind::CommandFailed,
            "\u{1b}[31mfailed\u{1b}[0m\r\nplain\u{7}",
            "audit",
        )
        .with_stderr("\u{1b}]0;title\u{7}stderr");

        assert_eq!(failure.summary, "failed\nplain");
        assert_eq!(failure.error.stderr_tail.as_deref(), Some("stderr"));
    }

    #[test]
    fn tail_is_unicode_safe() {
        let value = "é".repeat(2001);
        let tailed = tail(&value, 2000);
        assert!(tailed.starts_with("...\n"));
        assert_eq!(tailed.chars().skip(4).count(), 2000);
    }
}
