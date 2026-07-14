from pathlib import Path

path = Path("crates/phenix-mcp-core/src/result.rs")
text = path.read_text()

text = text.replace(
    '''    pub fn new(kind: ErrorKind, message: impl Into<String>, audit_id: impl Into<String>) -> Self {
        let msg: String = message.into();
''',
    '''    pub fn new(kind: ErrorKind, message: impl Into<String>, audit_id: impl Into<String>) -> Self {
        let msg = sanitize_terminal_text(&message.into());
''',
    1,
)
text = text.replace(
    '''    pub fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
        let s: String = stdout.into();
        self.error.stdout_tail = Some(tail(&s, 2000));
''',
    '''    pub fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
        let sanitized = sanitize_terminal_text(&stdout.into());
        self.error.stdout_tail = Some(tail(&sanitized, 2000));
''',
    1,
)
text = text.replace(
    '''    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        let s: String = stderr.into();
        self.error.stderr_tail = Some(tail(&s, 2000));
''',
    '''    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        let sanitized = sanitize_terminal_text(&stderr.into());
        self.error.stderr_tail = Some(tail(&sanitized, 2000));
''',
    1,
)

old_tail = '''fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let start = s.len() - max;
        format!("...\n{}", &s[start..])
    }
}
'''
new_tail = '''fn sanitize_terminal_text(input: &str) -> String {
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
'''
if text.count(old_tail) != 1:
    raise SystemExit(f"expected one tail helper, found {text.count(old_tail)}")
text = text.replace(old_tail, new_tail, 1)

text += '''

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
'''

for needle in [
    "let msg = sanitize_terminal_text",
    "let sanitized = sanitize_terminal_text",
    "fn sanitize_terminal_text",
]:
    if needle not in text:
        raise SystemExit(f"missing expected repair: {needle}")

path.write_text(text)
